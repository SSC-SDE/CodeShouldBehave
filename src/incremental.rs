use crate::architecture::{Architecture, FileNode};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathChange {
    Added { path: String },
    Deleted { path: String },
    Modified { path: String },
    Renamed { from: String, to: String },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InvalidationCause {
    Body(String),
    Signature(String),
    Imports(String),
    ModuleMove { from: String, to: String },
    Manifest(String),
    Contracts,
    ResolverConfiguration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionStrategy {
    Local,
    Full { reasons: Vec<InvalidationCause> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    pub id: String,
    pub baseline: String,
    pub proposal: String,
    pub changes: Vec<PathChange>,
    pub parse_paths: Vec<String>,
    pub reused_paths: Vec<String>,
    pub affected_unchanged_callers: Vec<String>,
    pub invalidations: Vec<InvalidationCause>,
    pub resolution: ResolutionStrategy,
}

pub(crate) fn plan(
    baseline: &str,
    proposal: &str,
    architecture: &Architecture,
    resolver_configuration_changed: bool,
    contracts_changed: bool,
) -> ChangeSet {
    let before = by_path(&architecture.before);
    let after = by_path(&architecture.after);
    let shared = before
        .keys()
        .filter(|path| after.contains_key(*path))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut deleted = before
        .keys()
        .filter(|path| !after.contains_key(*path))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut added = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut changes = Vec::new();

    let deleted_by_oid = group_by_oid(&deleted, &before);
    let added_by_oid = group_by_oid(&added, &after);
    for (oid, old_paths) in deleted_by_oid {
        let Some(new_paths) = added_by_oid.get(&oid) else {
            continue;
        };
        if old_paths.len() == 1 && new_paths.len() == 1 {
            let from = old_paths[0];
            let to = new_paths[0];
            deleted.remove(from);
            added.remove(to);
            changes.push(PathChange::Renamed {
                from: from.into(),
                to: to.into(),
            });
        }
    }
    changes.extend(
        deleted
            .into_iter()
            .map(|path| PathChange::Deleted { path: path.into() }),
    );
    changes.extend(
        added
            .into_iter()
            .map(|path| PathChange::Added { path: path.into() }),
    );
    for path in &shared {
        if before[path].oid != after[path].oid {
            changes.push(PathChange::Modified {
                path: (*path).into(),
            });
        }
    }
    changes.sort_by_key(change_key);

    let mut invalidations = Vec::new();
    let mut parse_paths = BTreeSet::new();
    for change in &changes {
        match change {
            PathChange::Added { path } => {
                parse_paths.insert(path.clone());
                if !after[path.as_str()].definitions.is_empty() {
                    invalidations.push(InvalidationCause::Signature(path.clone()));
                }
                classify_special(path, &mut invalidations);
            }
            PathChange::Deleted { path } => {
                if !before[path.as_str()].definitions.is_empty() {
                    invalidations.push(InvalidationCause::Signature(path.clone()));
                }
                classify_special(path, &mut invalidations);
            }
            PathChange::Renamed { from, to } => {
                parse_paths.insert(to.clone());
                invalidations.push(InvalidationCause::ModuleMove {
                    from: from.clone(),
                    to: to.clone(),
                });
                classify_special(from, &mut invalidations);
                classify_special(to, &mut invalidations);
            }
            PathChange::Modified { path } => {
                parse_paths.insert(path.clone());
                classify_modified(
                    path,
                    before[path.as_str()],
                    after[path.as_str()],
                    &mut invalidations,
                );
                classify_special(path, &mut invalidations);
            }
        }
    }
    if contracts_changed {
        invalidations.push(InvalidationCause::Contracts);
    }
    if resolver_configuration_changed {
        invalidations.push(InvalidationCause::ResolverConfiguration);
    }
    invalidations.sort();
    invalidations.dedup();

    let changed_paths = changes
        .iter()
        .flat_map(|change| match change {
            PathChange::Added { path }
            | PathChange::Deleted { path }
            | PathChange::Modified { path } => vec![path.as_str()],
            PathChange::Renamed { from, to } => vec![from.as_str(), to.as_str()],
        })
        .collect::<BTreeSet<_>>();
    let changed_symbol_keys = architecture
        .before
        .iter()
        .chain(&architecture.after)
        .filter(|node| changed_paths.contains(node.path.as_str()))
        .flat_map(symbol_keys)
        .collect::<BTreeSet<_>>();
    let affected_unchanged_callers = architecture
        .before_references
        .iter()
        .chain(&architecture.after_references)
        .filter(|reference| {
            !changed_paths.contains(reference.source_path.as_str())
                && reference
                    .candidates
                    .iter()
                    .any(|candidate| changed_symbol_keys.contains(candidate))
        })
        .map(|reference| reference.source_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let reused_paths = shared
        .into_iter()
        .filter(|path| before[path].oid == after[path].oid)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let resolution = if invalidations
        .iter()
        .all(|cause| matches!(cause, InvalidationCause::Body(_)))
    {
        ResolutionStrategy::Local
    } else {
        ResolutionStrategy::Full {
            reasons: invalidations
                .iter()
                .filter(|cause| !matches!(cause, InvalidationCause::Body(_)))
                .cloned()
                .collect(),
        }
    };
    let id = change_set_id(
        baseline,
        proposal,
        &changes,
        resolver_configuration_changed,
        contracts_changed,
    );
    ChangeSet {
        id,
        baseline: baseline.into(),
        proposal: proposal.into(),
        changes,
        parse_paths: parse_paths.into_iter().collect(),
        reused_paths,
        affected_unchanged_callers,
        invalidations,
        resolution,
    }
}

fn symbol_keys(node: &FileNode) -> Vec<String> {
    if node.symbols.is_empty() {
        node.definitions
            .iter()
            .map(|name| format!("{}:heuristic:{name}", node.path))
            .collect()
    } else {
        node.symbols
            .iter()
            .map(|symbol| symbol.stable_key.clone())
            .collect()
    }
}

pub(crate) fn apply(
    baseline: &[FileNode],
    clean_proposal: &[FileNode],
    change_set: &ChangeSet,
) -> Result<Vec<FileNode>, String> {
    let proposal = by_path(clean_proposal);
    let mut result = baseline
        .iter()
        .map(|node| (node.path.clone(), node.clone()))
        .collect::<BTreeMap<_, _>>();
    for change in &change_set.changes {
        match change {
            PathChange::Deleted { path } => {
                result.remove(path).ok_or_else(|| {
                    format!("Incremental deletion is absent from baseline: {path}")
                })?;
            }
            PathChange::Added { path } | PathChange::Modified { path } => {
                let node = proposal
                    .get(path.as_str())
                    .ok_or_else(|| format!("Incremental proposal path is absent: {path}"))?;
                result.insert(path.clone(), (*node).clone());
            }
            PathChange::Renamed { from, to } => {
                result.remove(from).ok_or_else(|| {
                    format!("Incremental rename origin is absent from baseline: {from}")
                })?;
                let node = proposal
                    .get(to.as_str())
                    .ok_or_else(|| format!("Incremental rename target is absent: {to}"))?;
                result.insert(to.clone(), (*node).clone());
            }
        }
    }
    Ok(result.into_values().collect())
}

fn by_path(nodes: &[FileNode]) -> BTreeMap<&str, &FileNode> {
    nodes
        .iter()
        .map(|node| (node.path.as_str(), node))
        .collect()
}

fn group_by_oid<'a>(
    paths: &BTreeSet<&'a str>,
    nodes: &BTreeMap<&'a str, &'a FileNode>,
) -> BTreeMap<String, Vec<&'a str>> {
    let mut result = BTreeMap::new();
    for path in paths {
        result
            .entry(nodes[path].oid.clone())
            .or_insert_with(Vec::new)
            .push(*path);
    }
    result
}

fn signatures(node: &FileNode) -> BTreeSet<(&str, &str)> {
    node.symbols
        .iter()
        .map(|symbol| (symbol.kind.as_str(), symbol.correspondence_key.as_str()))
        .collect()
}

fn classify_modified(
    path: &str,
    before: &FileNode,
    after: &FileNode,
    output: &mut Vec<InvalidationCause>,
) {
    if signatures(before) != signatures(after) || before.definitions != after.definitions {
        output.push(InvalidationCause::Signature(path.into()));
    }
    if before.imports != after.imports {
        output.push(InvalidationCause::Imports(path.into()));
    }
    if signatures(before) == signatures(after)
        && before.definitions == after.definitions
        && before.imports == after.imports
    {
        output.push(InvalidationCause::Body(path.into()));
    }
}

fn classify_special(path: &str, output: &mut Vec<InvalidationCause>) {
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches!(
        name,
        "Cargo.toml"
            | "Cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "pyproject.toml"
            | "requirements.txt"
    ) {
        output.push(InvalidationCause::Manifest(path.into()));
    }
}

fn change_key(change: &PathChange) -> (String, u8) {
    match change {
        PathChange::Added { path } => (path.clone(), 0),
        PathChange::Deleted { path } => (path.clone(), 1),
        PathChange::Modified { path } => (path.clone(), 2),
        PathChange::Renamed { from, .. } => (from.clone(), 3),
    }
}

fn change_set_id(
    baseline: &str,
    proposal: &str,
    changes: &[PathChange],
    resolver: bool,
    contracts: bool,
) -> String {
    let mut hash = Sha256::new();
    for value in ["flowdiff-change-set-v1", baseline, proposal] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    hash.update([u8::from(resolver), u8::from(contracts)]);
    for change in changes {
        hash.update(format!("{change:?}").as_bytes());
        hash.update([0]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::{ReferenceEvidence, ReferenceResolution};

    fn file(path: &str, oid: &str, definitions: &[&str], imports: &[&str]) -> FileNode {
        FileNode {
            path: path.into(),
            oid: oid.into(),
            definitions: definitions.iter().map(|value| (*value).into()).collect(),
            imports: imports.iter().map(|value| (*value).into()).collect(),
            ..FileNode::default()
        }
    }

    fn file_with_calls(path: &str, oid: &str, definitions: &[&str], calls: &[&str]) -> FileNode {
        let mut node = file(path, oid, definitions, &[]);
        node.calls = calls.iter().map(|value| (*value).into()).collect();
        node
    }

    #[test]
    fn body_only_change_is_local_and_tracks_unchanged_callers() {
        let architecture = Architecture {
            before: vec![
                file("src/caller.rs", "same", &["caller"], &[]),
                file("src/target.rs", "old", &["target"], &[]),
            ],
            after: vec![
                file("src/caller.rs", "same", &["caller"], &[]),
                file("src/target.rs", "new", &["target"], &[]),
            ],
            edges: vec![(
                "src/caller.rs".into(),
                "src/target.rs".into(),
                "target".into(),
                "retained",
            )],
            after_references: vec![ReferenceEvidence {
                source_path: "src/caller.rs".into(),
                name: "target".into(),
                span: None,
                resolution: ReferenceResolution::Resolved,
                candidates: vec!["src/target.rs:heuristic:target".into()],
            }],
            ..Architecture::default()
        };
        let change = plan("base", "head", &architecture, false, false);
        assert_eq!(
            change.changes,
            vec![PathChange::Modified {
                path: "src/target.rs".into()
            }]
        );
        assert_eq!(change.reused_paths, vec!["src/caller.rs"]);
        assert_eq!(change.affected_unchanged_callers, vec!["src/caller.rs"]);
        assert_eq!(change.resolution, ResolutionStrategy::Local);
    }

    #[test]
    fn signatures_imports_manifests_and_configuration_force_full_resolution() {
        let architecture = Architecture {
            before: vec![
                file("src/lib.rs", "old", &["old_name"], &["old::dep"]),
                file("Cargo.toml", "manifest-old", &[], &[]),
            ],
            after: vec![
                file("src/lib.rs", "new", &["new_name"], &["new::dep"]),
                file("Cargo.toml", "manifest-new", &[], &[]),
            ],
            ..Architecture::default()
        };
        let change = plan("base", "head", &architecture, true, true);
        let ResolutionStrategy::Full { reasons } = change.resolution else {
            panic!("expected full resolution")
        };
        assert!(reasons.contains(&InvalidationCause::Signature("src/lib.rs".into())));
        assert!(reasons.contains(&InvalidationCause::Imports("src/lib.rs".into())));
        assert!(reasons.contains(&InvalidationCause::Manifest("Cargo.toml".into())));
        assert!(reasons.contains(&InvalidationCause::Contracts));
        assert!(reasons.contains(&InvalidationCause::ResolverConfiguration));
    }

    #[test]
    fn rename_requires_a_unique_identical_blob_and_pr_identities_are_isolated() {
        let renamed = Architecture {
            before: vec![file("src/old.rs", "blob", &["item"], &[])],
            after: vec![file("src/new.rs", "blob", &["item"], &[])],
            ..Architecture::default()
        };
        let first = plan("base", "head-one", &renamed, false, false);
        let second = plan("base", "head-two", &renamed, false, false);
        assert_eq!(
            first.changes,
            vec![PathChange::Renamed {
                from: "src/old.rs".into(),
                to: "src/new.rs".into()
            }]
        );
        assert_ne!(first.id, second.id);

        let ambiguous = Architecture {
            before: vec![
                file("src/a.rs", "blob", &[], &[]),
                file("src/b.rs", "blob", &[], &[]),
            ],
            after: vec![file("src/c.rs", "blob", &[], &[])],
            ..Architecture::default()
        };
        let change = plan("base", "head", &ambiguous, false, false);
        assert!(
            change
                .changes
                .iter()
                .all(|item| !matches!(item, PathChange::Renamed { .. }))
        );
    }

    type NormalizedFile = (String, String, Vec<String>, Vec<String>, Vec<String>);

    fn normalized_graph(
        nodes: &[FileNode],
    ) -> (Vec<NormalizedFile>, Vec<(String, String, String)>) {
        let mut files = nodes
            .iter()
            .map(|node| {
                (
                    node.path.clone(),
                    node.oid.clone(),
                    node.definitions.iter().cloned().collect(),
                    node.imports.iter().cloned().collect(),
                    node.calls.iter().cloned().collect(),
                )
            })
            .collect::<Vec<_>>();
        files.sort();
        let mut definitions = BTreeMap::<&str, Vec<&str>>::new();
        for node in nodes {
            for definition in &node.definitions {
                definitions.entry(definition).or_default().push(&node.path);
            }
        }
        let mut edges = Vec::new();
        for node in nodes {
            for call in &node.calls {
                for target in definitions.get(call.as_str()).into_iter().flatten() {
                    edges.push((node.path.clone(), (*target).into(), call.clone()));
                }
            }
        }
        edges.sort();
        (files, edges)
    }

    #[test]
    fn incremental_application_matches_clean_rebuild_corpus() {
        let cases = vec![
            (
                vec![
                    file("src/a.rs", "a1", &["a"], &[]),
                    file_with_calls("src/caller.rs", "caller", &[], &["a"]),
                ],
                vec![
                    file("src/a.rs", "a2", &["a"], &[]),
                    file_with_calls("src/caller.rs", "caller", &[], &["a"]),
                ],
            ),
            (
                vec![
                    file("src/a.rs", "a1", &["a"], &["old"]),
                    file("src/delete.rs", "d1", &["gone"], &[]),
                ],
                vec![
                    file("src/a.rs", "a2", &["renamed"], &["new"]),
                    file("src/add.rs", "n1", &["added"], &[]),
                ],
            ),
            (
                vec![
                    file("src/old.rs", "same", &["moved"], &[]),
                    file("Cargo.toml", "c1", &[], &[]),
                ],
                vec![
                    file("src/new.rs", "same", &["moved"], &[]),
                    file("Cargo.toml", "c2", &[], &[]),
                ],
            ),
        ];
        for (before, after) in cases {
            let architecture = Architecture {
                before: before.clone(),
                after: after.clone(),
                ..Architecture::default()
            };
            let change = plan("base", "head", &architecture, false, false);
            assert_eq!(
                normalized_graph(&apply(&before, &after, &change).unwrap()),
                normalized_graph(&after)
            );
        }
    }

    #[test]
    fn unchanged_review_requires_no_parse_or_graph_delta() {
        let nodes = vec![file("src/a.rs", "same", &["a"], &[])];
        let architecture = Architecture {
            before: nodes.clone(),
            after: nodes.clone(),
            ..Architecture::default()
        };
        let change = plan("base", "base", &architecture, false, false);
        assert!(change.changes.is_empty());
        assert!(change.parse_paths.is_empty());
        assert_eq!(change.reused_paths, vec!["src/a.rs"]);
        assert_eq!(
            normalized_graph(&apply(&nodes, &nodes, &change).unwrap()),
            normalized_graph(&nodes)
        );
    }
}
