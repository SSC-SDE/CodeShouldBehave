use super::*;
use crate::design::{ConfirmedDesignContext, DesignLevel, generate_hld};
use crate::{
    Cancellation, ComponentDecision, ComponentProposal, EvidenceConfidence, PathScope, ScopeRecord,
    SecurityDomain,
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn retained_source_with_recomputed_checksum_cannot_be_labeled_as_an_old_git_blob() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn guard(n:i32)->i32 { n + 1 }\n");
    let commit = fixture.commit();
    let (_, mut snapshot) = fixture.snapshot(&commit);
    let entry = function(&snapshot, "guard").clone();
    let input = snapshot
        .inputs
        .iter_mut()
        .find(|input| input.path == entry.path)
        .unwrap();
    input.source = input.source.replace("n + 1", "n + 9");
    input.source_fingerprint = crate::design::digest(&[&input.source]);
    assert!(build_lld_detail(&snapshot, &entry.id, &Cancellation::default()).is_err());
    let evidence = snapshot
        .catalog
        .evidence
        .iter()
        .find(|evidence| entry.evidence_ids.contains(&evidence.id))
        .unwrap();
    assert!(lld_source(&snapshot, evidence).is_err());
}
struct Fixture {
    root: PathBuf,
    scope: ScopeRecord,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "csb-lld-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("lld-scope").unwrap(),
            primary_repository: RepositoryId::new(
                SecurityDomain::new("fixture").unwrap(),
                "github",
                "test",
                "repo",
            )
            .unwrap(),
            primary_paths: PathScope::new(vec![".".into()], vec!["private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: true,
            model_destinations: BTreeSet::new(),
        };
        let fixture = Self { root, scope };
        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.name", "LLD fixture"]);
        fixture.git(&["config", "user.email", "fixture@example.invalid"]);
        fixture.write(
            "Cargo.toml",
            "[package]\nname = 'fixture'\nversion = '0.1.0'\n",
        );
        fixture
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "core.hooksPath=/dev/null", "-C"])
            .arg(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn write(&self, path: &str, source: &str) {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn commit(&self) -> SnapshotId {
        self.git(&["add", "."]);
        self.git(&["commit", "-q", "-m", "LLD fixture"]);
        SnapshotId::new(self.git(&["rev-parse", "HEAD"])).unwrap()
    }
    fn snapshot(&self, commit: &SnapshotId) -> (DesignVersion, LldSnapshot) {
        self.prioritized_snapshot(commit, &BTreeSet::new())
    }
    fn prioritized_snapshot(
        &self,
        commit: &SnapshotId,
        priorities: &BTreeSet<String>,
    ) -> (DesignVersion, LldSnapshot) {
        let context = ConfirmedDesignContext {
            accepted_commit: commit.clone(),
            session_id: "lld-context".into(),
            session_revision: 1,
            input_fingerprint: "a".repeat(64),
            proposal_fingerprint: "b".repeat(64),
            components: vec![ComponentProposal {
                id: "app".into(),
                name: "Application".into(),
                role: "Rust library".into(),
                repository_index: 0,
                roots: vec![".".into()],
                stack_candidates: vec![],
                signals: vec![],
                evidence: vec![],
                confidence: EvidenceConfidence::UserSupplied,
                contradictions: vec![],
                decision: ComponentDecision::Corrected,
            }],
            providers: vec![],
            integrations: vec![],
            gaps: vec![],
        };
        let hld = generate_hld(
            &self.root,
            commit,
            &self.scope,
            &context,
            &Cancellation::default(),
        )
        .unwrap();
        let lld = generate_lld_prioritized(
            &self.root,
            commit,
            &self.scope,
            &context,
            &hld,
            &Cancellation::default(),
            priorities,
        )
        .unwrap();
        (hld, lld)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn function<'a>(snapshot: &'a LldSnapshot, name: &str) -> &'a LldEntry {
    snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.kind == "function" && entry.name == name)
        .unwrap()
}

#[test]
fn qualified_trait_inherent_and_nested_declarations_remain_distinct() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs","struct Thing { value: i32 }\ntrait First { fn run(&self) -> i32; }\ntrait Second { fn run(&self) -> i32; }\nimpl First for Thing { fn run(&self)->i32 { self.value } }\nimpl Second for Thing { fn run(&self)->i32 { 2 } }\nimpl Thing { fn run(&self)->i32 { 3 } }\nmod nested { fn run()->i32 { 4 } }\n");
    let commit = fixture.commit();
    let (_, snapshot) = fixture.snapshot(&commit);
    let methods = snapshot
        .catalog
        .entries
        .iter()
        .filter(|entry| entry.kind == "function" && entry.name == "run")
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 6);
    assert_eq!(
        methods
            .iter()
            .map(|entry| &entry.logical_key)
            .collect::<BTreeSet<_>>()
            .len(),
        6
    );
    assert!(
        methods
            .iter()
            .any(|entry| entry.container.contains("impl<5:First>"))
    );
    assert!(
        methods
            .iter()
            .any(|entry| entry.module.contains("::nested"))
    );
    assert!(
        snapshot
            .catalog
            .entries
            .iter()
            .any(|entry| entry.kind == "field" && entry.name == "value")
    );
}
#[test]
fn comments_line_shifts_and_formatting_do_not_change_semantic_projections() {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "fn run(x:i32)->i32 { if x > 0 { 1 } else { 2 } }\n",
    );
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    fixture.write("src/lib.rs","// shifted declaration\n\nfn run( x : i32 ) -> i32 {\n  if x > 0 { /* explanation */ 1 } else {2}\n}\n");
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    assert_ne!(function(&before, "run").id, function(&after, "run").id);
    assert_eq!(
        function(&before, "run").logical_key,
        function(&after, "run").logical_key
    );
    assert_eq!(
        function(&before, "run").body_fingerprint,
        function(&after, "run").body_fingerprint
    );
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert!(delta.changes.is_empty(), "{:?}", delta.changes);
    assert_eq!(delta.outcome, "incomplete");
    let da = build_lld_detail(
        &before,
        &function(&before, "run").id,
        &Cancellation::default(),
    )
    .unwrap();
    let db = build_lld_detail(
        &after,
        &function(&after, "run").id,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        compare_lld_detail(Some(&da), Some(&db)).outcome,
        "no_change_detected_within_scope"
    );
}
#[test]
fn literal_bytes_spaces_escapes_and_comparison_operators_are_preserved() {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "fn run(x:i32)->&'static str { if x > 0 { r#\"a b\"# } else { \"x\\n\" } }\n",
    );
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    fixture.write(
        "src/lib.rs",
        "fn run(x:i32)->&'static str { if x >= 0 { r#\"ab\"# } else { \"x\\t\" } }\n",
    );
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert_eq!(delta.outcome, "change_detected");
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "condition_changed")
    );
    assert_ne!(
        function(&before, "run").body_fingerprint,
        function(&after, "run").body_fingerprint
    );
}
#[test]
fn private_body_invalidates_lld_even_with_unchanged_hld_public_projection() {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "pub fn entry()->i32 { helper(1) }\nfn helper(x:i32)->i32 { if x > 0 { x } else { 0 } }\n",
    );
    let a = fixture.commit();
    let (hlda, before) = fixture.snapshot(&a);
    fixture.write("src/lib.rs","pub fn entry()->i32 { helper(1) }\nfn helper(x:i32)->i32 { if x >= 0 { x } else { -1 } }\n");
    let b = fixture.commit();
    let (hldb, after) = fixture.snapshot(&b);
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert_eq!(delta.outcome, "change_detected");
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "condition_changed")
    );
    assert_eq!(
        crate::design::compare_hld(&hlda, &hldb).unwrap().outcome,
        crate::design::DesignOutcome::NoChangeDetectedWithinScope
    );
    assert!(
        delta
            .possible_callers
            .iter()
            .any(|caller| caller.logical_key.contains("entry"))
    );
    assert!(
        delta
            .possible_callers
            .iter()
            .all(|caller| caller.resolution == "name_only_candidate")
    );
}
#[test]
fn pinned_ir_does_not_read_mutated_or_deleted_current_worktree() {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "fn run(x:i32)->i32 { if x > 7 { x } else { 0 } }\n",
    );
    let commit = fixture.commit();
    let (hld, snapshot) = fixture.snapshot(&commit);
    std::fs::remove_file(fixture.root.join("src/lib.rs")).unwrap();
    let detail = build_lld_detail(
        &snapshot,
        &function(&snapshot, "run").id,
        &Cancellation::default(),
    )
    .unwrap();
    let node = detail
        .graph
        .nodes
        .iter()
        .find(|node| node.operation == "condition" && node.label.contains("x > 7"))
        .unwrap();
    let evidence = detail
        .evidence
        .iter()
        .find(|evidence| evidence.id == node.evidence_id)
        .unwrap();
    assert!(lld_source(&snapshot, evidence).unwrap().contains("x > 7"));
    validate_lld_snapshot(&snapshot, &hld, &fixture.scope).unwrap();
}
#[test]
fn immutable_input_checksum_scope_and_parent_tampering_are_rejected() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run(){ }\n");
    let commit = fixture.commit();
    let (hld, snapshot) = fixture.snapshot(&commit);
    let mut changed = snapshot.clone();
    changed.inputs[0].source.push_str("tampered");
    assert!(validate_lld_snapshot(&changed, &hld, &fixture.scope).is_err());
    let mut changed = snapshot.clone();
    changed.catalog.entries[0].parent.component_id = None;
    assert!(validate_lld_snapshot(&changed, &hld, &fixture.scope).is_err());
    let mut changed = snapshot.clone();
    changed.inputs[0].path = "private/secret.rs".into();
    assert!(validate_lld_snapshot(&changed, &hld, &fixture.scope).is_err());
}
#[test]
fn old_hld_json_decodes_without_lld_and_typed_lld_uses_no_fake_hld_sections() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run(x:i32)->i32 { x+1 }\n");
    let commit = fixture.commit();
    let (hld, snapshot) = fixture.snapshot(&commit);
    let json = serde_json::to_value(&hld).unwrap();
    assert!(json.get("level").is_none());
    assert!(json.get("lld").is_none());
    let old: DesignVersion = serde_json::from_value(json).unwrap();
    assert_eq!(old.level, DesignLevel::Hld);
    let detail = build_lld_detail(
        &snapshot,
        &function(&snapshot, "run").id,
        &Cancellation::default(),
    )
    .unwrap();
    let design = lld_detail_design(&snapshot, &detail).unwrap();
    assert_eq!(design.level, DesignLevel::Lld);
    assert!(design.sections.is_empty());
    crate::design::validate_design_scope(&design, &fixture.scope).unwrap();
    let roundtrip: DesignVersion =
        serde_json::from_slice(&serde_json::to_vec(&design).unwrap()).unwrap();
    assert_eq!(roundtrip, design);
}
#[test]
fn duplicate_qualified_names_are_ambiguous_not_collapsed_or_fabricated_modified() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run()->i32 { 1 }\nfn run()->i32 { 2 }\n");
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    fixture.write("src/lib.rs", "fn run()->i32 { 1 }\nfn run()->i32 { 3 }\n");
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert!(delta.ambiguous_groups > 0);
    assert!(
        !delta
            .changes
            .iter()
            .any(|change| change.logical_key.ends_with("|run"))
    );
}
#[test]
fn source_move_requires_unique_identical_blob_and_does_not_prove_behavioral_equivalence() {
    let fixture = Fixture::new();
    fixture.write("src/old.rs", "fn run()->i32 { 1 }\n");
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    std::fs::rename(
        fixture.root.join("src/old.rs"),
        fixture.root.join("src/new.rs"),
    )
    .unwrap();
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "declaration_moved")
    );
    assert!(
        delta
            .changes
            .iter()
            .filter(|change| change.kind == "declaration_moved")
            .all(|change| change
                .limitations
                .iter()
                .any(|gap| gap.contains("unverified")))
    );
}
#[test]
fn incomplete_inventory_never_manufactures_removed_declarations_or_clean_result() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run()->i32 { 1 }\n");
    let commit = fixture.commit();
    let (_, snapshot) = fixture.snapshot(&commit);
    let mut missing = snapshot.catalog.clone();
    missing.entries.clear();
    missing.coverage.inventory_complete = false;
    missing.coverage.structure_complete = false;
    let delta = compare_lld(&snapshot.catalog, &missing);
    assert_eq!(delta.outcome, "incomplete");
    assert!(
        delta
            .changes
            .iter()
            .all(|change| change.classification == "unknown")
    );
}
#[test]
fn schema_changes_are_fingerprinted_without_execution_or_runtime_store_claims() {
    let fixture = Fixture::new();
    fixture.write("schema/001.sql", "CREATE TABLE users(id INTEGER);\n");
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    fixture.write(
        "schema/001.sql",
        "CREATE TABLE users(id INTEGER, name TEXT);\n",
    );
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert_eq!(delta.outcome, "change_detected");
    assert!(
        after
            .catalog
            .entries
            .iter()
            .filter(|entry| entry.kind == "schema")
            .all(|entry| entry.limitations.iter().any(|gap| gap.contains("unknown")))
    );
    assert!(!after.catalog.coverage.structure_complete);
}
#[test]
fn source_evidence_wrong_snapshot_is_rejected_and_excluded_paths_never_enter_private_ir() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run(){ }\n");
    fixture.write("private/secret.rs", "PRIVATE_SENTINEL");
    fixture.write(".env", "SECRET_SENTINEL");
    let commit = fixture.commit();
    let (_, snapshot) = fixture.snapshot(&commit);
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains("PRIVATE_SENTINEL"));
    assert!(!json.contains("SECRET_SENTINEL"));
    let mut evidence = snapshot.catalog.evidence[0].clone();
    evidence.commit = SnapshotId::new("f".repeat(40)).unwrap();
    assert!(lld_source(&snapshot, &evidence).is_err());
}

#[test]
fn hld_parent_uses_most_specific_roots_and_preserves_equal_owner_ambiguity() {
    let fixture = Fixture::new();
    fixture.write("src/nested/lib.rs", "fn run(){ }\n");
    let commit = fixture.commit();
    let (mut hld, _) = fixture.snapshot(&commit);
    let root = hld
        .elements
        .iter()
        .find(|entry| entry.kind == crate::design::DesignElementKind::Component)
        .unwrap()
        .clone();
    let mut src = root.clone();
    src.id = "component-src".into();
    src.roots = vec!["src".into()];
    let mut nested = root.clone();
    nested.id = "component-nested".into();
    nested.roots = vec!["src/nested".into()];
    hld.elements.extend([src, nested]);
    let mapped = catalog::parent_binding(&hld, "src/nested/lib.rs");
    assert_eq!(mapped.state, "matched");
    assert_eq!(mapped.component_id.as_deref(), Some("component-nested"));
    let mut conflict = root;
    conflict.id = "component-conflict".into();
    conflict.roots = vec!["src/nested".into()];
    hld.elements.push(conflict);
    let ambiguous = catalog::parent_binding(&hld, "src/nested/lib.rs");
    assert_eq!(ambiguous.state, "ambiguous");
    assert!(ambiguous.component_id.is_none());
    assert_eq!(ambiguous.candidate_component_ids.len(), 2);
    for entry in &mut hld.elements {
        if entry.kind == crate::design::DesignElementKind::Component {
            entry.roots = vec!["unselected".into()];
        }
    }
    let unmapped = catalog::parent_binding(&hld, "src/nested/lib.rs");
    assert_eq!(unmapped.state, "unmapped");
    assert!(unmapped.component_id.is_none());
}
#[test]
fn configuration_changes_invalidate_lld_with_unchanged_function_body_and_no_runtime_claim() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run(){ }\n");
    fixture.write("vercel.json", "{\"buildCommand\":\"cargo build\"}\n");
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    fixture.write(
        "vercel.json",
        "{\"buildCommand\":\"cargo build --release\"}\n",
    );
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    let delta = compare_lld(&before.catalog, &after.catalog);
    assert_eq!(
        function(&before, "run").body_fingerprint,
        function(&after, "run").body_fingerprint
    );
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "configuration_artifact_changed")
    );
    assert!(
        !delta
            .changes
            .iter()
            .filter(|change| change.logical_key == "configuration|vercel.json")
            .any(|change| change.kind == "signature_changed")
    );
}
#[test]
fn one_mib_catalog_and_child_detail_bounds_explicitly_defer_wide_inventory() {
    let fixture = Fixture::new();
    let name = "a".repeat(2800);
    let mut source = String::new();
    for i in 0..110 {
        source.push_str(&format!("fn function_{i}({name}_{i}:i32)->i32 {{ 1 }}\n"));
    }
    fixture.write("src/lib.rs", &source);
    let commit = fixture.commit();
    let (hld, snapshot) = fixture.snapshot(&commit);
    assert!(serde_json::to_vec(&snapshot.catalog).unwrap().len() <= MAX_LLD_CATALOG_BYTES);
    assert!(snapshot.catalog.coverage.deferred > 0);
    assert!(!snapshot.catalog.coverage.structure_complete);
    validate_lld_snapshot(&snapshot, &hld, &fixture.scope).unwrap();
    let file = snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.kind == "file")
        .unwrap();
    let detail = build_lld_detail(&snapshot, &file.id, &Cancellation::default()).unwrap();
    assert!(detail.declarations.len() <= 100);
    assert!(serde_json::to_vec(&detail.declarations).unwrap().len() <= 128 * 1024);
}
#[test]
fn syntax_inputs_exceeding_two_mib_are_deferred_not_read_from_checkout_later() {
    let fixture = Fixture::new();
    for i in 0..6 {
        fixture.write(
            &format!("src/file_{i}.rs"),
            &format!("// {}\nfn function_{i}(){{}}\n", "x".repeat(400 * 1024)),
        );
    }
    let commit = fixture.commit();
    let (hld, snapshot) = fixture.snapshot(&commit);
    assert!(
        snapshot
            .inputs
            .iter()
            .map(|input| input.source.len())
            .sum::<usize>()
            <= MAX_LLD_INPUT_BYTES
    );
    assert!(!snapshot.catalog.coverage.structure_complete);
    assert!(snapshot.catalog.coverage.failed > 0 || snapshot.catalog.coverage.deferred > 0);
    validate_lld_snapshot(&snapshot, &hld, &fixture.scope).unwrap();
}

#[test]
fn explicit_rename_split_and_merge_are_traceable_assertions_without_equivalence_or_receipt_transfer()
 {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "fn old()->i32{1}\nfn split()->i32{1}\nfn merge_a()->i32{1}\nfn merge_b()->i32{2}\n",
    );
    let a = fixture.commit();
    let (_, before) = fixture.snapshot(&a);
    fixture.write(
        "src/lib.rs",
        "fn renamed()->i32{1}\nfn split_a()->i32{1}\nfn split_b()->i32{2}\nfn merged()->i32{3}\n",
    );
    let b = fixture.commit();
    let (_, after) = fixture.snapshot(&b);
    let assertions = vec![
        crate::design::DesignCorrespondence {
            kind: "rename".into(),
            from_ids: vec![function(&before, "old").id.clone()],
            to_ids: vec![function(&after, "renamed").id.clone()],
            resolution: crate::design::DesignResolution::UserAssertion,
        },
        crate::design::DesignCorrespondence {
            kind: "split".into(),
            from_ids: vec![function(&before, "split").id.clone()],
            to_ids: vec![
                function(&after, "split_a").id.clone(),
                function(&after, "split_b").id.clone(),
            ],
            resolution: crate::design::DesignResolution::UserAssertion,
        },
        crate::design::DesignCorrespondence {
            kind: "merge".into(),
            from_ids: vec![
                function(&before, "merge_a").id.clone(),
                function(&before, "merge_b").id.clone(),
            ],
            to_ids: vec![function(&after, "merged").id.clone()],
            resolution: crate::design::DesignResolution::UserAssertion,
        },
    ];
    let delta =
        compare_lld_with_correspondence(&before.catalog, &after.catalog, &assertions).unwrap();
    assert_eq!(delta.correspondence, assertions);
    assert_eq!(
        delta
            .changes
            .iter()
            .filter(|change| change.resolution == "user_assertion")
            .count(),
        5
    );
    assert!(
        delta
            .changes
            .iter()
            .filter(|change| change.resolution == "user_assertion")
            .all(|change| change
                .limitations
                .iter()
                .any(|gap| gap.contains("acceptance transfer")))
    );
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "signature_changed"
                && change.proposal_entry_id.as_deref() == Some(&function(&after, "renamed").id))
    );
    let mut wrong = assertions.clone();
    wrong[0].to_ids[0] = "outside-selected-catalog".into();
    assert!(compare_lld_with_correspondence(&before.catalog, &after.catalog, &wrong).is_err());
    let mut wrong = assertions.clone();
    wrong[0].resolution = crate::design::DesignResolution::ResolvedReference;
    assert!(compare_lld_with_correspondence(&before.catalog, &after.catalog, &wrong).is_err());
    let mut wrong = assertions.clone();
    wrong[1].to_ids.truncate(1);
    assert!(compare_lld_with_correspondence(&before.catalog, &after.catalog, &wrong).is_err());
}
#[test]
fn invariant_receipt_and_correspondence_validation_bind_current_behavior_without_mutating_facts() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "fn run()->i32{1}\n");
    let commit = fixture.commit();
    let (_, snapshot) = fixture.snapshot(&commit);
    let detail = build_lld_detail(
        &snapshot,
        &function(&snapshot, "run").id,
        &Cancellation::default(),
    )
    .unwrap();
    let mut design = lld_detail_design(&snapshot, &detail).unwrap();
    let data = design.lld.as_mut().unwrap();
    data.invariants.push(LldInvariant {
        id: "reviewer-statement".into(),
        entry_id: data.entry_id.clone(),
        body_fingerprint: data.behavior_fingerprint.clone(),
        statement: "Return remains nonnegative; reviewer verification required".into(),
        resolution: "user_assertion".into(),
        verification: "requires_reviewer_verification".into(),
    });
    crate::design::validate_design_scope(&design, &fixture.scope).unwrap();
    assert_eq!(design.lld.as_ref().unwrap().detail, detail);
    design.lld.as_mut().unwrap().invariants[0].body_fingerprint = "old-body".into();
    assert!(crate::design::validate_design_scope(&design, &fixture.scope).is_err());
}

#[test]
fn unsupported_union_preserves_exact_frontier_without_misparenting_its_fields() {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "union RawBits { union_integer: u32, union_float: f32 }\nstruct SafeValue { safe_field: u32 }\nfn run()->u32 { 1 }\n",
    );
    let commit = fixture.commit();
    let (hld, snapshot) = fixture.snapshot(&commit);
    assert!(!snapshot.catalog.coverage.structure_complete);
    assert!(snapshot.catalog.coverage.unsupported > 0);
    assert!(
        snapshot
            .catalog
            .coverage
            .gaps
            .iter()
            .any(|gap| gap.contains("Union declaration"))
    );
    let frontier = snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.name == "RawBits")
        .unwrap();
    assert_eq!(frontier.kind, "unsupported_declaration");
    assert_eq!(frontier.detail_state, "unsupported");
    assert!(frontier.child_entry_ids.is_empty());
    assert!(
        !snapshot
            .catalog
            .entries
            .iter()
            .any(|entry| matches!(entry.name.as_str(), "union_integer" | "union_float"))
    );
    let safe_type = snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.name == "SafeValue")
        .unwrap();
    let safe_field = snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.name == "safe_field")
        .unwrap();
    assert_eq!(
        safe_field.parent_entry_id.as_deref(),
        Some(safe_type.id.as_str())
    );
    let evidence = snapshot
        .catalog
        .evidence
        .iter()
        .find(|evidence| frontier.evidence_ids.contains(&evidence.id))
        .unwrap();
    assert!(
        lld_source(&snapshot, evidence)
            .unwrap()
            .contains("union RawBits")
    );
    validate_lld_snapshot(&snapshot, &hld, &fixture.scope).unwrap();
    let detail = build_lld_detail(&snapshot, &frontier.id, &Cancellation::default()).unwrap();
    assert!(detail.graph.nodes.is_empty());
    assert!(!detail.coverage.structure_complete);
    assert_eq!(
        compare_lld(&snapshot.catalog, &snapshot.catalog).outcome,
        "incomplete"
    );
}

#[test]
fn changed_paths_survive_catalog_bounds_without_expanding_scope() {
    let fixture = Fixture::new();
    let source = (0..900)
        .map(|i| format!("pub fn unchanged_{i}() -> u64 {{ {i} }}\n"))
        .collect::<String>();
    fixture.write("src/a_inventory.rs", &source);
    fixture.write(
        "src/z_changed.rs",
        "pub fn changed(value: bool) -> u64 { if value { 1 } else { 2 } }",
    );
    fixture.write("private/secret.rs", "pub fn secret() {}");
    let commit = fixture.commit();
    let priorities = BTreeSet::from(["src/z_changed.rs".into(), "private/secret.rs".into()]);
    let (_, snapshot) = fixture.prioritized_snapshot(&commit, &priorities);
    assert!(snapshot.catalog.coverage.deferred > 0);
    assert!(!snapshot.catalog.coverage.structure_complete);
    assert!(serde_json::to_vec(&snapshot.catalog).unwrap().len() <= MAX_LLD_CATALOG_BYTES);
    assert!(
        !snapshot
            .catalog
            .entries
            .iter()
            .any(|e| e.path.starts_with("private/"))
    );
    let entry = function(&snapshot, "changed");
    build_lld_detail(&snapshot, &entry.id, &Cancellation::default()).unwrap();
    let (_, repeated) = fixture.prioritized_snapshot(&commit, &priorities);
    assert_eq!(snapshot.catalog.id, repeated.catalog.id);
}
