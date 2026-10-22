//! Read-only architecture-context drift. This is not PR impact or runtime proof.
use super::*;
use crate::{
    Cancellation, ContextPin, InventoryItem, OnboardingInventory, RepositoryId, ScopeVersion,
    SecurityDomain,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser, Point};

const VERSION: &str = "csb-onboarding-drift-v1";

fn repository(value: &Value) -> Result<RepositoryId, String> {
    let field = |key: &str| {
        value[key]
            .as_str()
            .ok_or_else(|| "Invalid accepted repository".to_string())
    };
    RepositoryId::new(
        SecurityDomain::new(field("security_domain")?)?,
        field("provider")?,
        field("owner")?,
        field("name")?,
    )
}

fn paths(value: &Value) -> Result<PathScope, String> {
    let list = |key: &str| -> Result<Vec<String>, String> {
        let values = value[key].as_array().ok_or("Invalid accepted paths")?;
        if values.len() > 100 {
            return Err("Accepted paths exceed bounded drift input".into());
        }
        values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("Invalid accepted path".into())
            })
            .collect()
    };
    PathScope::new(list("include")?, list("exclude")?)
}
fn scope(context: &Value) -> Result<ScopeRecord, String> {
    if context["version"] != "csb-accepted-context-v1"
        || context["usable"] != true
        || context["confirmation"].is_null()
    {
        return Err("Drift requires a usable confirmed architecture context".into());
    }
    let pins = context["contexts"]
        .as_array()
        .ok_or("Invalid accepted contexts")?;
    if pins.len() > 20 {
        return Err("Accepted contexts exceed bounded drift input".into());
    }
    Ok(ScopeRecord {
        version: ScopeVersion::new(
            context["scope_version"]
                .as_str()
                .ok_or("Invalid accepted scope version")?,
        )?,
        primary_repository: repository(&context["repository"])?,
        primary_paths: paths(context)?,
        contexts: pins
            .iter()
            .map(|pin| {
                Ok(ContextPin {
                    name: pin["name"]
                        .as_str()
                        .ok_or("Invalid accepted context name")?
                        .into(),
                    repository: repository(&pin["repository"])?,
                    commit: SnapshotId::new(
                        pin["commit"]
                            .as_str()
                            .ok_or("Invalid accepted context commit")?
                            .to_owned(),
                    )?,
                    paths: paths(pin)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        contracts: BTreeSet::new(),
        export_allowed: false,
        model_destinations: BTreeSet::new(),
    })
}
pub(crate) fn validate_access(
    context: &Value,
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<(), String> {
    let scope = scope(context)?;
    let source = SnapshotId::new(
        context["commit"]
            .as_str()
            .ok_or("Invalid accepted commit")?
            .to_owned(),
    )?;
    OnboardingSession::new(
        "drift-access".into(),
        principal.clone(),
        source,
        scope,
        crate::INVENTORY_VERSION.into(),
        grants,
    )
    .map(|_| ())
}
fn inventory(
    root: &Path,
    scope: &ScopeRecord,
    commit: &SnapshotId,
    principal: &PrincipalId,
    grants: &[Grant],
    started: Instant,
) -> Result<Option<OnboardingInventory>, String> {
    let session = OnboardingSession::new(
        "drift-inventory".into(),
        principal.clone(),
        commit.clone(),
        scope.clone(),
        crate::INVENTORY_VERSION.into(),
        grants,
    )?;
    let mut inventory = None;
    for _ in 0..64 {
        if started.elapsed() >= Duration::from_secs(10) {
            return Ok(None);
        }
        let next = crate::onboarding_scan::scan_chunk(
            root,
            &session,
            inventory.as_ref(),
            &Cancellation::default(),
            principal,
            grants,
        )?;
        let complete = next.complete;
        inventory = Some(next);
        if complete {
            return Ok(inventory);
        }
    }
    Ok(None)
}
fn hash_node(node: Node<'_>, source: &[u8], skip_blocks: bool) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut stack = vec![node];
    let mut count = 0;
    while let Some(node) = stack.pop() {
        count += 1;
        if count > 100000 {
            return Err("Rust signature exceeds traversal budget".into());
        }
        if matches!(node.kind(), "line_comment" | "block_comment")
            || (skip_blocks && node.kind() == "block")
        {
            continue;
        }
        hash.update(node.kind().as_bytes());
        hash.update([0]);
        if node.child_count() == 0 {
            hash.update(&source[node.byte_range()]);
            hash.update([0]);
        } else {
            let mut cursor = node.walk();
            stack.extend(
                node.children(&mut cursor)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev(),
            );
        }
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
fn rust_signatures(source: &str) -> Result<BTreeMap<&'static str, BTreeSet<String>>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| "Rust drift parser unavailable")?;
    let started = Instant::now();
    let mut progress = |_: &tree_sitter::ParseState| started.elapsed() >= Duration::from_secs(2);
    let mut input = |offset: usize, _point: Point| &source.as_bytes()[offset..];
    let tree = parser
        .parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or("Rust drift parser reached its two-second budget")?;
    if tree.root_node().has_error() {
        return Err("Rust syntax errors leave architecture context drift unresolved".into());
    }
    let mut signatures = BTreeMap::from([
        ("imports", BTreeSet::new()),
        ("contracts", BTreeSet::new()),
        ("modules", BTreeSet::new()),
        ("entrypoints", BTreeSet::new()),
    ]);
    let mut stack = vec![tree.root_node()];
    let mut count = 0;
    while let Some(node) = stack.pop() {
        count += 1;
        if count > 100000 || started.elapsed() >= Duration::from_secs(2) {
            return Err("Rust drift signature traversal exceeds its budget".into());
        }
        let category = match node.kind() {
            "use_declaration" | "extern_crate_declaration" => Some("imports"),
            "trait_item" => Some("contracts"),
            "mod_item" => Some("modules"),
            "function_item"
                if node
                    .child_by_field_name("name")
                    .is_some_and(|name| &source.as_bytes()[name.byte_range()] == b"main") =>
            {
                Some("entrypoints")
            }
            "function_item" | "struct_item" | "enum_item" | "type_item" => {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .any(|child| child.kind() == "visibility_modifier")
                    .then_some("contracts")
            }
            _ => None,
        };
        if let Some(category) = category {
            // Module ownership is its declaration/header, not every nested function body.
            let signature = if category == "modules" || category == "entrypoints" {
                let mut hash = Sha256::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if node
                        .child_by_field_name("body")
                        .is_some_and(|body| body.id() == child.id())
                    {
                        continue;
                    }
                    hash.update(hash_node(child, source.as_bytes(), true)?);
                }
                hash.finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            } else {
                hash_node(node, source.as_bytes(), category == "contracts")?
            };
            signatures.get_mut(category).unwrap().insert(signature);
        }
        let mut cursor = node.walk();
        stack.extend(
            node.children(&mut cursor)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
    Ok(signatures)
}
fn blob(root: &Path, item: &InventoryItem, budget: &mut usize) -> Result<String, String> {
    if *budget == 0 {
        return Err("Architecture drift blob budget exhausted".into());
    }
    let values = crate::snapshot::read_chunk(
        root,
        std::slice::from_ref(&item.blob),
        crate::snapshot::ReadLimits {
            objects_per_chunk: 1,
            bytes_per_object: 256 * 1024,
            bytes_per_chunk: (*budget).min(256 * 1024),
        },
        &Cancellation::default(),
    )?;
    match values.into_iter().next() {
        Some(crate::git::BatchBlob::Data(bytes)) => {
            *budget = budget.saturating_sub(bytes.len());
            String::from_utf8(bytes)
                .map_err(|_| "Source encoding unsupported by Rust drift parser".into())
        }
        _ => Err("Source blob unavailable or oversized for architecture drift".into()),
    }
}
fn under(path: &str, root: &str) -> bool {
    root == "."
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn evidence(item: &InventoryItem, commit: &SnapshotId, side: &str) -> Value {
    json!({"repository_index":0,"path":item.path,"blob":item.blob,"start_line":1,"end_line":1,"producer_version":VERSION,"commit":commit.as_str(),"side":side})
}
pub(crate) fn assess(
    roots: &BTreeMap<RepositoryId, PathBuf>,
    context: &Value,
    target: &SnapshotId,
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<Value, String> {
    validate_access(context, principal, grants)?;
    let mut scope = scope(context)?;
    let related = !scope.contexts.is_empty();
    scope.contexts.clear();
    let source = SnapshotId::new(
        context["commit"]
            .as_str()
            .ok_or("Invalid accepted commit")?
            .to_owned(),
    )?;
    let root = roots
        .get(&scope.primary_repository)
        .ok_or("Repository unavailable")?;
    let components: Vec<ComponentProposal> = serde_json::from_value(context["components"].clone())
        .map_err(|_| "Invalid accepted components")?;
    if components.len() > 100 {
        return Err("Accepted components exceed bounded drift input".into());
    }
    let started = Instant::now();
    let previous = inventory(root, &scope, &source, principal, grants, started)?;
    let next = inventory(root, &scope, target, principal, grants, started)?;
    let mut gaps = BTreeSet::new();
    let mut incomplete = false;
    if related {
        gaps.insert("Only the primary repository is assessed; related repository pins are unchanged and are not read".to_string());
    }
    let mut affected: BTreeMap<String, (BTreeSet<String>, Vec<Value>)> = BTreeMap::new();
    let complete = |inventory: &Option<OnboardingInventory>| {
        inventory.as_ref().is_some_and(|item| {
            item.complete && item.enumeration_complete && item.failed == 0 && item.deferred == 0
        })
    };
    if !complete(&previous) || !complete(&next) {
        incomplete = true;
        gaps.insert("Pinned source or target inventory is unavailable, incomplete, deferred or exceeds the ten-second/metadata budget; reuse cannot be established".into());
    }
    let entries = |inventory: &Option<OnboardingInventory>| {
        inventory
            .as_ref()
            .map(|value| {
                value
                    .items
                    .iter()
                    .map(|item| (item.path.clone(), item.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default()
    };
    let old = entries(&previous);
    let new = entries(&next);
    let mut budget = 10 * 1024 * 1024;
    let mut changed = 0;
    for path in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        let before = old.get(path);
        let after = new.get(path);
        if before.zip(after).is_some_and(|(before, after)| {
            before.blob == after.blob && before.kind == after.kind && before.status == after.status
        }) {
            continue;
        }
        let item = after.or(before).unwrap();
        if item.kind == "unsupported" {
            continue;
        }
        changed += 1;
        if changed > 200 || started.elapsed() >= Duration::from_secs(10) {
            incomplete = true;
            gaps.insert(
                "Changed structural paths exceed the 200-path or ten-second drift budget".into(),
            );
            break;
        }
        let mut reasons = BTreeSet::new();
        if before.is_none() || after.is_none() {
            reasons.insert(
                "Source or configuration boundary path was added or removed within accepted roots"
                    .into(),
            );
        } else if let (Some(before), Some(after), true) = (
            before,
            after,
            item.kind == "source" && path.ends_with(".rs"),
        ) {
            let signatures = blob(root, before, &mut budget)
                .and_then(|text| rust_signatures(&text))
                .and_then(|left| {
                    blob(root, after, &mut budget)
                        .and_then(|text| rust_signatures(&text))
                        .map(|right| (left, right))
                });
            match signatures {
                Ok((left, right)) => {
                    for category in ["imports", "contracts", "modules", "entrypoints"] {
                        if left[category] != right[category] {
                            reasons.insert(format!("Rust {category} signatures changed; observed syntax requires architecture-context re-confirmation"));
                        }
                    }
                }
                Err(_) => {
                    incomplete = true;
                    reasons.insert("Rust structural signatures could not be established within the supported parser/budget".into());
                }
            }
        } else if matches!(item.kind.as_str(), "manifest" | "lockfile") {
            reasons.insert("Dependency or package manifest/lockfile bytes changed; active dependency use is not inferred".into());
        } else if matches!(
            item.kind.as_str(),
            "deployment_config" | "executable_deployment_config" | "build_metadata" | "ci_config"
        ) {
            reasons.insert("Hosting/build configuration bytes changed; no configuration is executed or provider contacted".into());
        } else if matches!(item.kind.as_str(), "api_schema" | "database_schema") {
            reasons.insert("Source contract/schema bytes changed; compatibility and ownership remain unverified".into());
        } else {
            incomplete = true;
            reasons.insert(
                "Changed source language is outside the Rust structural drift subset".into(),
            );
        }
        if reasons.is_empty() {
            continue;
        }
        let owners = components
            .iter()
            .filter(|component| {
                component.repository_index == 0
                    && component.roots.iter().any(|root| under(path, root))
            })
            .collect::<Vec<_>>();
        if owners.is_empty() {
            incomplete = true;
            gaps.insert("A changed accepted path has no confirmed component owner; no ownership is invented".into());
        }
        if owners.len() > 1 {
            gaps.insert("Overlapping confirmed component roots leave ownership ambiguous; all matching components are shown".into());
        }
        for component in owners {
            let entry = affected.entry(component.id.clone()).or_default();
            entry.0.extend(reasons.clone());
            for (item, commit, side) in [(before, &source, "baseline"), (after, target, "target")] {
                if let Some(item) = item {
                    if entry.1.len() < 16 {
                        entry.1.push(evidence(item, commit, side));
                    } else {
                        gaps.insert("Component drift evidence is capped at sixteen source-qualified blobs; additional changed paths are represented by bounded reasons".into());
                    }
                }
            }
        }
    }
    for component in components.iter().filter(|item| item.repository_index == 0) {
        for root in &component.roots {
            if !new.values().any(|item| {
                item.status == "indexed"
                    && ((item.kind == "source" && under(&item.path, root))
                        || (item.kind == "manifest"
                            && item.path.rsplit_once('/').map_or(".", |(parent, _)| parent)
                                == root))
            }) {
                affected.entry(component.id.clone()).or_default().0.insert("Previously confirmed root has no indexed source or manifest at the target; moved roots are not added to scope".into());
            }
        }
    }
    if old
        .values()
        .chain(new.values())
        .any(|item| item.kind == "source" && !item.path.ends_with(".rs"))
    {
        gaps.insert("Non-Rust source uses byte identity only; semantic drift for changed files is unresolved".into());
    }
    gaps.insert("Rust macros, conditional compilation and runtime ownership/direction are not resolved; this is context drift, not a PR impact finding".into());
    let differences=affected.into_iter().map(|(id,(reasons,evidence))|json!({"component_id":id,"previously_confirmed":components.iter().find(|item|item.id==id).map(|item|json!({"name":item.name,"role":item.role,"roots":item.roots})),"decision":"pending","reasons":reasons.into_iter().take(8).collect::<Vec<_>>(),"evidence":evidence})).collect::<Vec<_>>();
    let reuse = !incomplete && differences.is_empty();
    Ok(
        json!({"version":VERSION,"session_id":context["session_id"],"session_revision":context["session_revision"],"source_commit":source.as_str(),"target_commit":target.as_str(),"input_fingerprint":context["input_fingerprint"],"proposal_fingerprint":context["proposal_fingerprint"],"accepted_scope_fingerprint":context["accepted_scope_fingerprint"],"status":if incomplete {"incomplete"} else if reuse {"unchanged"} else {"changed"},"reuse_allowed":reuse,"reconfirmation_required":!reuse,"affected_components":differences,"gaps":gaps.into_iter().take(20).collect::<Vec<_>>(),"defaults_changed":false,"scope_expanded":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    struct Fixture {
        root: PathBuf,
        repository: RepositoryId,
        principal: PrincipalId,
        grants: Vec<Grant>,
        source: SnapshotId,
        context: Value,
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn commit(root: &Path) -> SnapshotId {
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "fixture"]);
        SnapshotId::new(git(root, &["rev-parse", "HEAD"])).unwrap()
    }
    const SOURCE: &str = "use axum::Router;\npub trait Gateway { fn capture(&self) -> i32 { 1 } }\npub fn run() -> i32 { 1 }\n";
    fn fixture() -> Fixture {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "csb-drift-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(root.join("outside-unapproved")).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "fixture@example.invalid"]);
        git(&root, &["config", "user.name", "Fixture"]);
        std::fs::write(root.join("app/lib.rs"), SOURCE).unwrap();
        std::fs::write(
            root.join("app/Cargo.toml"),
            "[package]\nname='api'\nversion='0.1.0'\n[dependencies]\naxum='0.8'\n",
        )
        .unwrap();
        std::fs::write(root.join("app/vercel.json"), "{\"version\":2}").unwrap();
        std::fs::write(
            root.join("outside-unapproved/hidden.rs"),
            "OUTSIDE_SOURCE_SENTINEL",
        )
        .unwrap();
        std::fs::write(
            root.join("app/credentials.json"),
            "SENSITIVE_SOURCE_SENTINEL",
        )
        .unwrap();
        let source = commit(&root);
        let repository = RepositoryId::new(
            SecurityDomain::new("test").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let principal = PrincipalId::new("maintainer").unwrap();
        let paths = PathScope::new(vec!["app".into()], vec![]).unwrap();
        let grants = vec![Grant {
            principal: principal.clone(),
            repository: repository.clone(),
            actions: BTreeSet::from([Action::Index, Action::ConfirmArchitecture]),
            paths: paths.clone(),
        }];
        let mut session = OnboardingSession::new(
            "confirmed".into(),
            principal.clone(),
            source.clone(),
            ScopeRecord {
                version: ScopeVersion::new("test-boundary").unwrap(),
                primary_repository: repository.clone(),
                primary_paths: paths,
                contexts: vec![],
                contracts: BTreeSet::new(),
                export_allowed: false,
                model_destinations: BTreeSet::new(),
            },
            crate::INVENTORY_VERSION.into(),
            &grants,
        )
        .unwrap();
        let mut inventory = OnboardingInventory::empty(&session);
        inventory.complete = true;
        let blob = git(
            &root,
            &["rev-parse", &format!("{}:app/lib.rs", source.as_str())],
        );
        inventory.items.push(InventoryItem {
            repository_index: 0,
            path: "app/lib.rs".into(),
            blob: blob.clone(),
            kind: "source".into(),
            status: "indexed".into(),
            variable_names: vec![],
        });
        session
            .record_inventory(0, inventory, &principal, &grants)
            .unwrap();
        let mut component = ComponentProposal {
            id: "api".into(),
            name: "API".into(),
            role: "backend".into(),
            repository_index: 0,
            roots: vec!["app".into()],
            stack_candidates: vec!["Rust".into()],
            signals: vec![],
            evidence: vec![OnboardingEvidence {
                repository_index: 0,
                path: "app/lib.rs".into(),
                blob,
                start_line: 1,
                end_line: 1,
                producer_version: "fixture-v1".into(),
            }],
            confidence: EvidenceConfidence::Candidate,
            contradictions: vec![],
            decision: ComponentDecision::Pending,
        };
        session
            .set_proposal(
                1,
                vec![component.clone()],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        component.decision = ComponentDecision::Confirmed;
        session
            .decide_component(3, component, &principal, &grants)
            .unwrap();
        session
            .confirm(
                4,
                &session.proposal_fingerprint(),
                false,
                1,
                &principal,
                &grants,
            )
            .unwrap();
        let context =
            crate::onboarding_service::accepted_context_view(&session, &principal, &grants)
                .unwrap();
        Fixture {
            root,
            repository,
            principal,
            grants,
            source,
            context,
        }
    }
    impl Fixture {
        fn assess(&self, target: &SnapshotId) -> Value {
            assess(
                &BTreeMap::from([(self.repository.clone(), self.root.clone())]),
                &self.context,
                target,
                &self.principal,
                &self.grants,
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn body_only_changes_reuse_but_import_contract_provider_dependency_and_root_changes_require_review()
     {
        let fixture = fixture();
        let immutable = fixture.context.clone();
        std::fs::write(fixture.root.join("app/lib.rs"),"// body-only/comment change\nuse axum :: Router;\npub trait Gateway { fn capture(&self) -> i32 { 42 } }\npub fn run() -> i32 { 100 }\n").unwrap();
        let body = commit(&fixture.root);
        let result = fixture.assess(&body);
        assert_eq!(result["reuse_allowed"], true);
        assert_eq!(result["status"], "unchanged");
        assert_eq!(result["source_commit"], fixture.source.as_str());
        assert_eq!(result["target_commit"], body.as_str());
        for (path, changed, reason) in [
            (
                "app/lib.rs",
                "use actix_web::App;\npub trait Gateway { fn capture(&self)->i32 {1} }\npub fn run()->i32 {1}",
                "imports",
            ),
            (
                "app/lib.rs",
                "use axum::Router;\npub trait Gateway { fn capture(&self)->u64 {1} }\npub fn run()->i32 {1}",
                "contracts",
            ),
            (
                "app/Cargo.toml",
                "[package]\nname='api'\nversion='0.1.0'\n[dependencies]\naxum='0.9'\n",
                "Dependency",
            ),
            (
                "app/vercel.json",
                "{\"buildCommand\":\"HOSTING_CONFIG_VALUE_SENTINEL\"}",
                "Hosting",
            ),
        ] {
            std::fs::write(fixture.root.join(path), changed).unwrap();
            let target = commit(&fixture.root);
            let result = fixture.assess(&target);
            assert_eq!(result["reuse_allowed"], false);
            assert_eq!(result["reconfirmation_required"], true);
            assert_eq!(result["defaults_changed"], false);
            assert_eq!(result["scope_expanded"], false);
            assert!(
                result["affected_components"][0]["reasons"]
                    .to_string()
                    .contains(reason)
            );
            assert!(
                result["affected_components"][0]["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["commit"] == fixture.source.as_str()
                        || item["commit"] == target.as_str())
            );
            assert!(!result.to_string().contains("HOSTING_CONFIG_VALUE_SENTINEL"));
            assert!(!result.to_string().contains("SENSITIVE_SOURCE_SENTINEL"));
            assert!(!result.to_string().contains("OUTSIDE_SOURCE_SENTINEL"));
        }
        std::fs::rename(
            fixture.root.join("app"),
            fixture.root.join("new-outside-root"),
        )
        .unwrap();
        let moved = commit(&fixture.root);
        let result = fixture.assess(&moved);
        assert_eq!(result["reuse_allowed"], false);
        assert!(
            result["affected_components"][0]["reasons"]
                .to_string()
                .contains("Previously confirmed root")
        );
        assert!(!result.to_string().contains("new-outside-root"));
        assert_eq!(fixture.context, immutable);
    }
    #[test]
    fn drift_denies_revoked_boundaries_before_read_and_marks_missing_source_or_non_rust_changes_incomplete()
     {
        let fixture = fixture();
        let unavailable =
            BTreeMap::from([(fixture.repository.clone(), PathBuf::from("does-not-exist"))]);
        assert!(
            assess(
                &unavailable,
                &fixture.context,
                &fixture.source,
                &fixture.principal,
                &[]
            )
            .unwrap_err()
            .contains("denied")
        );
        let mut narrowed = fixture.grants.clone();
        narrowed[0].paths = PathScope::new(vec!["app/narrow".into()], vec![]).unwrap();
        assert!(
            assess(
                &unavailable,
                &fixture.context,
                &fixture.source,
                &fixture.principal,
                &narrowed
            )
            .is_err()
        );
        let mut missing = fixture.context.clone();
        missing["commit"] = json!("0".repeat(40));
        let result = assess(
            &BTreeMap::from([(fixture.repository.clone(), fixture.root.clone())]),
            &missing,
            &fixture.source,
            &fixture.principal,
            &fixture.grants,
        )
        .unwrap();
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["reuse_allowed"], false);
        std::fs::write(fixture.root.join("app/ui.ts"), "export const value=1;").unwrap();
        let added = commit(&fixture.root);
        assert_eq!(fixture.assess(&added)["reuse_allowed"], false);
        let mut context = fixture.context.clone();
        context["commit"] = json!(added.as_str());
        std::fs::write(fixture.root.join("app/ui.ts"), "export const value=2;").unwrap();
        let changed = commit(&fixture.root);
        let result = assess(
            &BTreeMap::from([(fixture.repository.clone(), fixture.root.clone())]),
            &context,
            &changed,
            &fixture.principal,
            &fixture.grants,
        )
        .unwrap();
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["reuse_allowed"], false);
    }
}
