use crate::{ComparisonMode, ScopeVersion, SnapshotId};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn identifier(kind: &str, value: impl Into<String>) -> Result<String, String> {
    let value = value.into();
    if value.is_empty()
        || value.len() > 128
        || value.chars().any(char::is_control)
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace)
    {
        Err(format!(
            "{kind} must contain 1–128 printable characters without surrounding whitespace"
        ))
    } else {
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SecurityDomain(String);

impl SecurityDomain {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        identifier("Security domain", value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepositoryId {
    security_domain: SecurityDomain,
    provider: String,
    owner: String,
    name: String,
}

impl RepositoryId {
    pub fn new(
        security_domain: SecurityDomain,
        provider: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            security_domain,
            provider: identifier("Repository provider", provider)?,
            owner: identifier("Repository owner", owner)?,
            name: identifier("Repository name", name)?,
        })
    }

    pub fn security_domain(&self) -> &SecurityDomain {
        &self.security_domain
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

fn normalize_path(path: impl Into<String>) -> Result<String, String> {
    let path = path.into();
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(['*', '?', '[', ']'])
        || path
            .trim_end_matches('/')
            .split('/')
            .any(|part| part.is_empty() || part == ".." || part == ".git")
    {
        return Err(format!("Invalid repository-relative scope path: {path:?}"));
    }
    Ok(if path == "." {
        path
    } else {
        path.trim_end_matches('/').to_owned()
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathScope {
    includes: BTreeSet<String>,
    excludes: BTreeSet<String>,
}

impl PathScope {
    pub fn new(
        includes: impl IntoIterator<Item = String>,
        excludes: impl IntoIterator<Item = String>,
    ) -> Result<Self, String> {
        let includes = includes
            .into_iter()
            .map(normalize_path)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if includes.is_empty() {
            return Err("At least one included path is required".into());
        }
        let excludes = excludes
            .into_iter()
            .map(normalize_path)
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(Self { includes, excludes })
    }

    pub fn all() -> Self {
        Self {
            includes: BTreeSet::from([".".into()]),
            excludes: BTreeSet::new(),
        }
    }

    pub fn allows(&self, path: &str) -> bool {
        fn matches(path: &str, prefix: &str) -> bool {
            prefix == "."
                || path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }
        self.includes.iter().any(|prefix| matches(path, prefix))
            && !self.excludes.iter().any(|prefix| matches(path, prefix))
    }

    pub fn includes(&self) -> impl Iterator<Item = &str> {
        self.includes.iter().map(String::as_str)
    }

    pub fn excludes(&self) -> impl Iterator<Item = &str> {
        self.excludes.iter().map(String::as_str)
    }

    pub(crate) fn intersects_prefix(&self, prefix: &str) -> bool {
        !self.excludes.iter().any(|excluded| {
            excluded == prefix
                || prefix
                    .strip_prefix(excluded)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) && self.includes.iter().any(|included| {
            included == "."
                || included == prefix
                || included
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || prefix
                    .strip_prefix(included)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextPin {
    pub name: String,
    pub repository: RepositoryId,
    pub commit: SnapshotId,
    pub paths: PathScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeRecord {
    pub version: ScopeVersion,
    pub primary_repository: RepositoryId,
    pub primary_paths: PathScope,
    pub contexts: Vec<ContextPin>,
    pub contracts: BTreeSet<String>,
    pub export_allowed: bool,
    pub model_destinations: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopeDelta {
    pub added_contexts: Vec<String>,
    pub removed_contexts: Vec<String>,
    pub changed_contexts: Vec<String>,
    pub primary_paths_changed: bool,
    pub contracts_changed: bool,
    pub export_policy_changed: bool,
    pub model_destinations_changed: bool,
    pub added_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub file_preview_truncated: bool,
}

impl ScopeDelta {
    pub fn changes_identity(&self) -> bool {
        self.primary_paths_changed
            || self.contracts_changed
            || self.export_policy_changed
            || self.model_destinations_changed
            || !self.added_contexts.is_empty()
            || !self.removed_contexts.is_empty()
            || !self.changed_contexts.is_empty()
    }
}

impl ScopeRecord {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put(&mut bytes, "flowdiff-scope-v1");
        put_repository(&mut bytes, &self.primary_repository);
        put_paths(&mut bytes, &self.primary_paths);
        let mut contexts = self.contexts.iter().collect::<Vec<_>>();
        contexts.sort_by(|left, right| {
            (&left.name, &left.repository).cmp(&(&right.name, &right.repository))
        });
        put_usize(&mut bytes, contexts.len());
        for context in contexts {
            put(&mut bytes, &context.name);
            put_repository(&mut bytes, &context.repository);
            put(&mut bytes, context.commit.as_str());
            put_paths(&mut bytes, &context.paths);
        }
        put_set(&mut bytes, &self.contracts);
        bytes.push(u8::from(self.export_allowed));
        put_set(&mut bytes, &self.model_destinations);
        bytes
    }

    pub fn diff(&self, proposed: &Self) -> Result<ScopeDelta, String> {
        if self.primary_repository != proposed.primary_repository {
            return Err("A scope change cannot replace the primary repository".into());
        }
        let current = self
            .contexts
            .iter()
            .map(|item| (&item.name, item))
            .collect::<std::collections::BTreeMap<_, _>>();
        let next = proposed
            .contexts
            .iter()
            .map(|item| (&item.name, item))
            .collect::<std::collections::BTreeMap<_, _>>();
        if current.len() != self.contexts.len() || next.len() != proposed.contexts.len() {
            return Err("Context names must be unique within a scope".into());
        }
        Ok(ScopeDelta {
            added_contexts: next
                .keys()
                .filter(|name| !current.contains_key(*name))
                .map(|name| (*name).clone())
                .collect(),
            removed_contexts: current
                .keys()
                .filter(|name| !next.contains_key(*name))
                .map(|name| (*name).clone())
                .collect(),
            changed_contexts: current
                .iter()
                .filter_map(|(name, item)| {
                    next.get(name)
                        .filter(|next_item| *next_item != item)
                        .map(|_| (*name).clone())
                })
                .collect(),
            primary_paths_changed: self.primary_paths != proposed.primary_paths,
            contracts_changed: self.contracts != proposed.contracts,
            export_policy_changed: self.export_allowed != proposed.export_allowed,
            model_destinations_changed: self.model_destinations != proposed.model_destinations,
            added_files: Vec::new(),
            removed_files: Vec::new(),
            file_preview_truncated: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewIdentityInput {
    pub repository: RepositoryId,
    pub selected_baseline: SnapshotId,
    pub analysis_baseline: SnapshotId,
    pub proposal: SnapshotId,
    pub comparison: ComparisonMode,
    pub scope: ScopeRecord,
    pub parser_version: String,
    pub resolver_version: String,
    pub rule_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewId(String);

impl ReviewId {
    pub fn from_input(input: &ReviewIdentityInput) -> Result<Self, String> {
        if input.repository != input.scope.primary_repository {
            return Err("Review repository must match the scope primary repository".into());
        }
        let mut bytes = Vec::new();
        put(&mut bytes, "flowdiff-review-v1");
        put_repository(&mut bytes, &input.repository);
        put(&mut bytes, input.selected_baseline.as_str());
        put(&mut bytes, input.analysis_baseline.as_str());
        put(&mut bytes, input.proposal.as_str());
        put(
            &mut bytes,
            match input.comparison {
                ComparisonMode::Exact => "exact",
                ComparisonMode::MergeBase => "merge-base",
            },
        );
        put(&mut bytes, input.scope.version.as_str());
        put_paths(&mut bytes, &input.scope.primary_paths);
        let mut contexts = input.scope.contexts.iter().collect::<Vec<_>>();
        contexts.sort_by(|left, right| {
            (&left.name, &left.repository).cmp(&(&right.name, &right.repository))
        });
        put_usize(&mut bytes, contexts.len());
        for context in contexts {
            put(&mut bytes, &context.name);
            put_repository(&mut bytes, &context.repository);
            put(&mut bytes, context.commit.as_str());
            put_paths(&mut bytes, &context.paths);
        }
        put_set(&mut bytes, &input.scope.contracts);
        bytes.push(u8::from(input.scope.export_allowed));
        put_set(&mut bytes, &input.scope.model_destinations);
        put(&mut bytes, &input.parser_version);
        put(&mut bytes, &input.resolver_version);
        put(&mut bytes, &input.rule_version);
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        Ok(Self(encoded))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn put(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn put_usize(bytes: &mut Vec<u8>, value: usize) {
    bytes.extend_from_slice(&(value as u64).to_be_bytes());
}

fn put_repository(bytes: &mut Vec<u8>, repository: &RepositoryId) {
    put(bytes, repository.security_domain.as_str());
    put(bytes, &repository.provider);
    put(bytes, &repository.owner);
    put(bytes, &repository.name);
}

fn put_paths(bytes: &mut Vec<u8>, paths: &PathScope) {
    put_set(bytes, &paths.includes);
    put_set(bytes, &paths.excludes);
}

fn put_set(bytes: &mut Vec<u8>, values: &BTreeSet<String>) {
    put_usize(bytes, values.len());
    for value in values {
        put(bytes, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(domain: &str) -> RepositoryId {
        RepositoryId::new(
            SecurityDomain::new(domain).unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap()
    }

    fn identity() -> ReviewIdentityInput {
        let repository = repository("tenant-a");
        ReviewIdentityInput {
            repository: repository.clone(),
            selected_baseline: SnapshotId::new("a".repeat(40)).unwrap(),
            analysis_baseline: SnapshotId::new("a".repeat(40)).unwrap(),
            proposal: SnapshotId::new("b".repeat(40)).unwrap(),
            comparison: ComparisonMode::Exact,
            scope: ScopeRecord {
                version: ScopeVersion::new("scope-1").unwrap(),
                primary_repository: repository,
                primary_paths: PathScope::new(
                    vec!["src/".into(), "docs/api.md".into()],
                    vec!["src/private".into()],
                )
                .unwrap(),
                contexts: vec![],
                contracts: BTreeSet::from(["contracts/payment-v2".into()]),
                export_allowed: false,
                model_destinations: BTreeSet::new(),
            },
            parser_version: "rust-parser-v1".into(),
            resolver_version: "rust-resolver-v1".into(),
            rule_version: "rules-v1".into(),
        }
    }

    #[test]
    fn canonical_identity_ignores_set_input_order() {
        let first = identity();
        let mut second = identity();
        second.scope.primary_paths = PathScope::new(
            vec!["docs/api.md".into(), "src".into()],
            vec!["src/private/".into()],
        )
        .unwrap();
        assert_eq!(
            ReviewId::from_input(&first).unwrap(),
            ReviewId::from_input(&second).unwrap()
        );
    }

    #[test]
    fn every_analysis_input_changes_identity() {
        let original = identity();
        let original_id = ReviewId::from_input(&original).unwrap();
        let assert_changed = |changed: ReviewIdentityInput| {
            assert_ne!(original_id, ReviewId::from_input(&changed).unwrap());
        };

        let mut changed = identity();
        changed.selected_baseline = SnapshotId::new("c".repeat(40)).unwrap();
        assert_changed(changed);
        let mut changed = identity();
        changed.analysis_baseline = SnapshotId::new("d".repeat(40)).unwrap();
        assert_changed(changed);
        let mut changed = identity();
        changed.proposal = SnapshotId::new("e".repeat(40)).unwrap();
        assert_changed(changed);
        let mut changed = identity();
        changed.comparison = ComparisonMode::MergeBase;
        assert_changed(changed);
        let mut changed = identity();
        changed.scope.version = ScopeVersion::new("scope-2").unwrap();
        assert_changed(changed);
        let mut changed = identity();
        changed.scope.primary_paths = PathScope::new(vec!["src/api".into()], vec![]).unwrap();
        assert_changed(changed);
        let mut changed = identity();
        changed.scope.contracts.insert("contracts/extra".into());
        assert_changed(changed);
        let mut changed = identity();
        changed.scope.export_allowed = true;
        assert_changed(changed);
        let mut changed = identity();
        changed
            .scope
            .model_destinations
            .insert("approved-gateway".into());
        assert_changed(changed);
        let mut changed = identity();
        changed.scope.contexts.push(ContextPin {
            name: "consumer".into(),
            repository: RepositoryId::new(
                SecurityDomain::new("tenant-a").unwrap(),
                "github",
                "owner",
                "consumer",
            )
            .unwrap(),
            commit: SnapshotId::new("f".repeat(40)).unwrap(),
            paths: PathScope::all(),
        });
        assert_changed(changed);
        let mut changed = identity();
        changed.parser_version = "rust-parser-v2".into();
        assert_changed(changed);
        let mut changed = identity();
        changed.resolver_version = "rust-resolver-v2".into();
        assert_changed(changed);
        let mut changed = identity();
        changed.rule_version = "rules-v2".into();
        assert_changed(changed);

        let mut changed = identity();
        changed.repository = repository("tenant-b");
        assert!(ReviewId::from_input(&changed).is_err());
    }

    #[test]
    fn path_scope_is_literal_and_exclusions_win() {
        let scope = PathScope::new(vec!["src".into()], vec!["src/private".into()]).unwrap();
        assert!(scope.allows("src/api.rs"));
        assert!(!scope.allows("src-private/api.rs"));
        assert!(!scope.allows("src/private/key.rs"));
        assert!(PathScope::new(vec!["../outside".into()], vec![]).is_err());
    }

    #[test]
    fn scope_preview_reports_context_and_policy_changes_without_mutation() {
        let current = identity().scope;
        let mut proposed = current.clone();
        proposed.primary_paths = PathScope::new(vec!["src/api".into()], vec![]).unwrap();
        proposed.contexts.push(ContextPin {
            name: "orders".into(),
            repository: RepositoryId::new(
                SecurityDomain::new("tenant-a").unwrap(),
                "github",
                "owner",
                "orders",
            )
            .unwrap(),
            commit: SnapshotId::new("f".repeat(40)).unwrap(),
            paths: PathScope::new(vec!["contracts".into()], vec!["contracts/private".into()])
                .unwrap(),
        });
        proposed.export_allowed = true;
        let delta = current.diff(&proposed).unwrap();
        assert_eq!(delta.added_contexts, vec!["orders"]);
        assert!(delta.primary_paths_changed);
        assert!(delta.export_policy_changed);
        assert!(delta.changes_identity());
        let mut wrong = proposed;
        wrong.primary_repository = repository("tenant-b");
        assert!(current.diff(&wrong).is_err());
    }
}
