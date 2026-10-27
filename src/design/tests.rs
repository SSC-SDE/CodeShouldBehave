use super::*;
use crate::{Cancellation, ComponentDecision, EvidenceConfidence, PathScope, SecurityDomain};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "csb-hld-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = Self { root };
        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.name", "HLD fixture"]);
        fixture.git(&["config", "user.email", "fixture@example.invalid"]);
        fixture
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .arg("-C")
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
    fn write(&self, path: &str, text: &str) {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn commit(&self) -> SnapshotId {
        self.git(&["add", "."]);
        self.git(&["commit", "-q", "-m", "HLD fixture"]);
        SnapshotId::new(self.git(&["rev-parse", "HEAD"])).unwrap()
    }
    fn simple(&self) -> SnapshotId {
        self.write(
            "Cargo.toml",
            "[package]\nname = 'fixture'\nversion = '0.1.0'\n[dependencies]\nreqwest = '0.12'\n",
        );
        self.write("src/lib.rs","use reqwest::Client;\npub trait Gateway { fn capture(&self)->i32 { 1 } }\npub fn run()->i32 { helper() }\nfn helper()->i32 { 1 }\n");
        self.commit()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn scope() -> ScopeRecord {
    ScopeRecord {
        version: ScopeVersion::new("confirmed-scope").unwrap(),
        primary_repository: RepositoryId::new(
            SecurityDomain::new("fixture").unwrap(),
            "github",
            "test",
            "repository",
        )
        .unwrap(),
        primary_paths: PathScope::new(vec![".".into()], vec!["private".into()]).unwrap(),
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: true,
        model_destinations: BTreeSet::new(),
    }
}
fn component(id: &str, root: &str) -> ComponentProposal {
    ComponentProposal {
        id: id.into(),
        name: id.into(),
        role: "Rust component".into(),
        repository_index: 0,
        roots: vec![root.into()],
        stack_candidates: vec![],
        signals: vec![],
        evidence: vec![],
        confidence: EvidenceConfidence::UserSupplied,
        contradictions: vec![],
        decision: ComponentDecision::Corrected,
    }
}
fn context(commit: &SnapshotId) -> ConfirmedDesignContext {
    ConfirmedDesignContext {
        accepted_commit: commit.clone(),
        session_id: "onboarding-fixture".into(),
        session_revision: 5,
        input_fingerprint: "a".repeat(64),
        proposal_fingerprint: "b".repeat(64),
        components: vec![component("app", ".")],
        providers: vec![],
        integrations: vec![],
        gaps: vec![],
    }
}
fn generate(
    fixture: &Fixture,
    commit: &SnapshotId,
    scope: &ScopeRecord,
    context: &ConfirmedDesignContext,
) -> DesignVersion {
    generate_hld(
        &fixture.root,
        commit,
        scope,
        context,
        &Cancellation::default(),
    )
    .unwrap()
}

#[test]
fn hld_is_reproducible_scoped_and_has_complete_section_inventory() {
    let fixture = Fixture::new();
    let baseline = fixture.simple();
    fixture.write(".env", "API_SECRET=NEVER_DISCLOSE_HLD_SENTINEL");
    fixture.write("private/excluded.rs", "pub struct PrivateSentinel;");
    let snapshot = fixture.commit();
    let scope = scope();
    let context = context(&baseline);
    let left = generate(&fixture, &snapshot, &scope, &context);
    let right = generate(&fixture, &snapshot, &scope, &context);
    assert_eq!(left, right);
    assert_eq!(left.sections.len(), 9);
    assert!(left.coverage.complete, "{:?}", left.coverage.gaps);
    let bytes = serde_json::to_vec(&left).unwrap();
    let encoded = String::from_utf8(bytes.clone()).unwrap();
    assert!(!encoded.contains("NEVER_DISCLOSE"));
    assert!(!encoded.contains("PrivateSentinel"));
    assert!(!encoded.contains("private/excluded"));
    assert_eq!(
        serde_json::from_slice::<DesignVersion>(&bytes).unwrap(),
        left
    );
    for relation in &left.relations {
        assert!(!relation.evidence_ids.is_empty());
    }
    assert_eq!(
        left.elements
            .iter()
            .filter(|element| element.kind == DesignElementKind::Component)
            .count(),
        1
    );
    assert!(
        left.elements
            .iter()
            .any(|element| element.name == "reqwest package")
    );
    assert!(
        left.sections
            .iter()
            .any(|section| section.id == "major_flows" && !section.unknowns.is_empty())
    );
}
#[test]
fn private_helper_body_changes_do_not_become_hld_changes() {
    let fixture = Fixture::new();
    let baseline = fixture.simple();
    let scope = scope();
    let context = context(&baseline);
    let before = generate(&fixture, &baseline, &scope, &context);
    fixture.write("src/lib.rs","use reqwest::Client;\npub trait Gateway { fn capture(&self) -> i32 { 9 } }\npub fn run() -> i32 { helper()+3 }\nfn helper()->i32 { println!(\"private body\"); 80 }\n");
    let proposal = fixture.commit();
    let after = generate(&fixture, &proposal, &scope, &context);
    let delta = compare_hld(&before, &after).unwrap();
    assert_ne!(before.id, after.id);
    assert!(delta.changes.is_empty(), "{:?}", delta.changes);
    assert_eq!(delta.outcome, DesignOutcome::NoChangeDetectedWithinScope);
    assert_ne!(before.evidence, after.evidence);
}
#[test]
fn public_contract_and_manifest_changes_have_exact_commit_evidence() {
    let fixture = Fixture::new();
    let baseline = fixture.simple();
    let scope = scope();
    let context = context(&baseline);
    let before = generate(&fixture, &baseline, &scope, &context);
    fixture.write("src/lib.rs","use reqwest::Client;\npub trait Gateway { fn capture(&self)->u64 { 1 } }\npub fn run()->i32 { 1 }\n");
    fixture.write(
        "Cargo.toml",
        "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nreqwest='0.13'\nlapin='3'\n",
    );
    let proposal = fixture.commit();
    let after = generate(&fixture, &proposal, &scope, &context);
    let delta = compare_hld(&before, &after).unwrap();
    assert_eq!(delta.outcome, DesignOutcome::ChangeDetected);
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.after_name.as_deref() == Some("Gateway"))
    );
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.after_name.as_deref() == Some("lapin package"))
    );
    assert!(
        after
            .evidence
            .iter()
            .all(|evidence| evidence.commit == proposal)
    );
    assert!(
        after
            .elements
            .iter()
            .filter(|element| element.kind == DesignElementKind::Component)
            .count()
            == 1,
        "An SDK must not invent a queue service"
    );
}
#[test]
fn static_package_direction_supports_qualified_reverse_impact_and_constraints() {
    let fixture = Fixture::new();
    fixture.write("Cargo.toml", "[workspace]\nmembers=['api','shared']\n");
    fixture.write(
        "api/Cargo.toml",
        "[package]\nname='api'\nversion='0.1.0'\n[dependencies]\nshared={path='../shared'}\n",
    );
    fixture.write(
        "api/src/lib.rs",
        "use shared::Request;\npub fn serve()->i32 { 1 }\n",
    );
    fixture.write(
        "shared/Cargo.toml",
        "[package]\nname='shared'\nversion='0.1.0'\n",
    );
    fixture.write("shared/src/lib.rs", "pub struct Request { pub id:i32 }\n");
    let baseline = fixture.commit();
    let scope = scope();
    let mut context = context(&baseline);
    context.components = vec![
        component("workspace", "."),
        component("api", "api"),
        component("shared", "shared"),
    ];
    let before = generate(&fixture, &baseline, &scope, &context);
    let api = generate::component_id(&scope, "api");
    let shared = generate::component_id(&scope, "shared");
    assert!(before.relations.iter().any(|relation| relation.from == api
        && relation.to == shared
        && relation.kind == "local_package_dependency"));
    fixture.write("shared/src/lib.rs", "pub struct Request { pub id:u64 }\n");
    let proposal = fixture.commit();
    let after = generate(&fixture, &proposal, &scope, &context);
    let delta = compare_hld(&before, &after).unwrap();
    assert!(
        delta
            .possible_impacts
            .iter()
            .any(|impact| impact.component_id == api)
    );
    assert!(
        !delta
            .possible_impacts
            .iter()
            .any(|impact| impact.component_id == shared),
        "The changed contract's owner is directly affected, not an unchanged downstream consumer"
    );
    let constraints = vec![DesignConstraint {
        id: "constraint-no-api-shared".into(),
        kind: "prohibited_dependency".into(),
        from_component_id: api,
        to_component_id: Some(shared),
        value: "".into(),
        user_assertion: "Keep the API independent".into(),
        evidence_ids: vec![],
    }];
    let findings = evaluate_hld_constraints(&after, &constraints);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].resolution, DesignResolution::ObservedStructure);
    assert!(!findings[0].evidence_ids.is_empty());
}
#[test]
fn syntax_errors_and_unsupported_source_cannot_return_no_impact() {
    let fixture = Fixture::new();
    let baseline = fixture.simple();
    let scope = scope();
    let context = context(&baseline);
    fixture.write(
        "web/index.ts",
        "export function unresolvedRuntime(){return 1;}",
    );
    let unsupported = fixture.commit();
    let partial = generate(&fixture, &unsupported, &scope, &context);
    assert!(!partial.coverage.complete);
    assert_eq!(
        compare_hld(&partial, &partial).unwrap().outcome,
        DesignOutcome::Incomplete
    );
    fixture.write("src/lib.rs", "pub fn broken( {\n");
    let broken = fixture.commit();
    let error = generate(&fixture, &broken, &scope, &context);
    assert!(!error.coverage.complete);
    assert!(error.coverage.failed > 0);
}
#[test]
fn vercel_configuration_changes_are_structural_not_live_deployment_claims() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    fixture.write(
        "vercel.json",
        "{\"framework\":\"vite\",\"outputDirectory\":\"dist\"}",
    );
    let baseline = fixture.commit();
    let scope = scope();
    let context = context(&source);
    let before = generate(&fixture, &baseline, &scope, &context);
    fixture.write("vercel.json","{\"framework\":\"nextjs\",\"outputDirectory\":\".next\",\"env\":{\"TOKEN\":\"NEVER_DISCLOSE_CONFIG_SENTINEL\"}}");
    let proposal = fixture.commit();
    let after = generate(&fixture, &proposal, &scope, &context);
    let delta = compare_hld(&before, &after).unwrap();
    assert_eq!(delta.outcome, DesignOutcome::ChangeDetected);
    assert!(!after.coverage.complete);
    assert!(
        !serde_json::to_string(&after)
            .unwrap()
            .contains("NEVER_DISCLOSE_CONFIG_SENTINEL")
    );
    assert!(after.elements.iter().any(|element| {
        element.kind == DesignElementKind::Deployment
            && element
                .unknowns
                .iter()
                .any(|unknown| unknown.contains("not evidence of an active"))
    }));
}
#[test]
fn source_scope_and_version_changes_invalidate_design_identity() {
    let fixture = Fixture::new();
    let snapshot = fixture.simple();
    let original = scope();
    let context = context(&snapshot);
    let before = generate(&fixture, &snapshot, &original, &context);
    let mut next = original.clone();
    next.version = ScopeVersion::new("scope-v2").unwrap();
    let after = generate(&fixture, &snapshot, &next, &context);
    assert_ne!(before.id, after.id);
    let mut forged = before.clone();
    forged.evidence[0].path = "private/hidden.rs".into();
    assert!(validate_design_scope(&forged, &original).is_err());
    let mut sensitive = context.clone();
    sensitive.components[0].roots = vec![".env".into()];
    assert!(sensitive.validate(&original).is_err());
    let mut narrowed = original.clone();
    narrowed.primary_paths = PathScope::new(vec!["src".into()], vec![]).unwrap();
    assert!(context.validate(&narrowed).is_err());
}
#[test]
fn missing_snapshot_returns_partial_and_never_reads_working_tree() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let context = context(&source);
    fixture.write("src/lib.rs", "pub struct WorkingTreeOnly;");
    let missing = SnapshotId::new("f".repeat(40)).unwrap();
    let partial = generate(&fixture, &missing, &scope, &context);
    assert!(!partial.coverage.enumeration_complete);
    assert!(!partial.coverage.complete);
    assert!(partial.evidence.is_empty());
    assert!(!partial.markdown.contains("WorkingTreeOnly"));
}
#[test]
fn oversized_derived_inventory_is_explicitly_partial_and_fits_storage() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let declarations = (0..900)
        .map(|index| {
            format!(
                "pub struct Contract{index} {{ pub payload: [u64; 32], pub metadata: [u8; 128] }}\n"
            )
        })
        .collect::<String>();
    fixture.write("src/lib.rs", &declarations);
    let snapshot = fixture.commit();
    let version = generate(&fixture, &snapshot, &scope(), &context(&source));
    assert!(serde_json::to_vec(&version).unwrap().len() <= 900 * 1024);
    assert!(!version.coverage.complete);
    assert!(version.coverage.deferred > 0);
}

#[test]
fn oversized_contract_inventory_preserves_dependency_diff_and_exact_evidence() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let declarations = (0..900)
        .map(|index| {
            format!(
                "pub struct Contract{index} {{ pub payload: [u64; 32], pub metadata: [u8; 128] }}\n"
            )
        })
        .collect::<String>();
    fixture.write(
        "src/lib.rs",
        &format!("use reqwest::Client;\n{declarations}"),
    );
    let baseline = fixture.commit();
    let before = generate(&fixture, &baseline, &scope(), &context(&source));
    fixture.write(
        "Cargo.toml",
        "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nreqwest='0.12'\nlapin='3'\n",
    );
    fixture.write(
        "src/lib.rs",
        &format!("use reqwest::Client;\nuse lapin::Connection;\n{declarations}"),
    );
    let proposal = fixture.commit();
    let after = generate(&fixture, &proposal, &scope(), &context(&source));
    for version in [&before, &after] {
        assert!(serde_json::to_vec(version).unwrap().len() <= 900 * 1024);
        assert!(!version.coverage.complete);
        assert!(version.coverage.deferred > 0);
        assert!(
            version
                .elements
                .iter()
                .any(|element| element.name == "reqwest package")
        );
        validate_design_scope(version, &scope()).unwrap();
    }
    assert!(
        !before
            .elements
            .iter()
            .any(|element| element.name == "lapin package")
    );
    let dependency = after
        .elements
        .iter()
        .find(|element| element.name == "lapin package")
        .unwrap();
    for kind in ["declares_package_dependency", "imports_declared_package"] {
        let relation = after
            .relations
            .iter()
            .find(|relation| relation.to == dependency.id && relation.kind == kind)
            .unwrap();
        for id in &relation.evidence_ids {
            let evidence = after
                .evidence
                .iter()
                .find(|evidence| evidence.id == *id)
                .unwrap();
            assert_eq!(evidence.commit, proposal);
            assert!(matches!(
                evidence.path.as_str(),
                "Cargo.toml" | "src/lib.rs"
            ));
        }
    }
    let delta = compare_hld(&before, &after).unwrap();
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.after_name.as_deref() == Some("lapin package"))
    );
    // The dependency difference remains visible without claiming full coverage.
    assert_eq!(delta.outcome, DesignOutcome::Incomplete);
}

#[test]
fn schema_introduction_and_declared_feature_or_lock_changes_remain_qualified() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    fixture.write(
        "Cargo.lock",
        "version=3\n[[package]]\nname='reqwest'\nversion='0.12.0'\n",
    );
    let baseline = fixture.commit();
    let scope = scope();
    let context = context(&source);
    let before = generate(&fixture, &baseline, &scope, &context);
    fixture.write(
        "Cargo.lock",
        "version=3\n[[package]]\nname='reqwest'\nversion='0.13.0'\n",
    );
    fixture.write("Cargo.toml","[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nreqwest={version='0.12',features=['json'],default-features=false}\n");
    fixture.write(
        "migrations/001.sql",
        "CREATE TABLE payment (id BIGINT PRIMARY KEY);\n",
    );
    let proposal = fixture.commit();
    let after = generate(&fixture, &proposal, &scope, &context);
    let delta = compare_hld(&before, &after).unwrap();
    assert_eq!(delta.outcome, DesignOutcome::ChangeDetected);
    assert!(!delta.coverage.complete);
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.after_name.as_deref() == Some("reqwest locked package"))
    );
    assert!(after.elements.iter().any(|element| {
        element.kind == DesignElementKind::Schema
            && element
                .unknowns
                .iter()
                .any(|unknown| unknown.contains("exclusive data ownership"))
    }));
}
#[test]
fn narrowed_viewer_scope_cannot_read_even_an_authorized_parent_label() {
    let fixture = Fixture::new();
    let commit = fixture.simple();
    let original = scope();
    let context = context(&commit);
    let version = generate(&fixture, &commit, &original, &context);
    let mut narrower = original.clone();
    narrower.primary_paths = PathScope::new(vec!["src".into()], vec![]).unwrap();
    assert!(validate_design_scope(&version, &narrower).is_err());
    let mut intended = version.clone();
    let component = intended
        .elements
        .iter()
        .find(|element| element.kind == DesignElementKind::Component)
        .unwrap()
        .id
        .clone();
    intended.relations.push(DesignRelation {
        id: "user-flow-fixture".into(),
        from: component.clone(),
        to: component,
        kind: "user_major_flow".into(),
        direction: "user_asserted".into(),
        protocol: None,
        synchrony: "unknown".into(),
        resolution: DesignResolution::UserAssertion,
        evidence_ids: vec![],
        unknowns: vec!["User assertion; source behavior remains unverified".into()],
    });
    assert!(validate_design_scope(&intended, &original).is_ok());
    intended.relations.last_mut().unwrap().resolution = DesignResolution::ResolvedReference;
    assert!(validate_design_scope(&intended, &original).is_err());
}

#[test]
fn explicit_correspondence_classifies_moves_without_guessing_same_named_identity() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let old_context = context(&source);
    let before = generate(&fixture, &source, &scope, &old_context);
    let mut new_context = old_context.clone();
    new_context.components[0].id = "moved-logical-component".into();
    new_context.components[0].name = "renamed app".into();
    new_context.proposal_fingerprint = "c".repeat(64);
    let mut after = generate(&fixture, &source, &scope, &new_context);
    let from = before
        .elements
        .iter()
        .find(|element| element.kind == DesignElementKind::Component)
        .unwrap()
        .id
        .clone();
    let to = after
        .elements
        .iter()
        .find(|element| element.kind == DesignElementKind::Component)
        .unwrap()
        .id
        .clone();
    after.correspondence.push(DesignCorrespondence {
        kind: "move".into(),
        from_ids: vec![from.clone()],
        to_ids: vec![to.clone()],
        resolution: DesignResolution::UserAssertion,
    });
    let delta = compare_hld(&before, &after).unwrap();
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "moved" && change.element_id.as_ref() == Some(&to))
    );
    assert!(
        !delta
            .changes
            .iter()
            .any(|change| change.kind == "removed" && change.element_id.as_ref() == Some(&from))
    );
    after.correspondence.push(after.correspondence[0].clone());
    let ambiguous = compare_hld(&before, &after).unwrap();
    assert!(
        ambiguous
            .questions
            .iter()
            .any(|question| question.contains("overlaps another mapping"))
    );
}

#[test]
fn missing_or_malformed_snapshot_never_manufactures_architecture_removals_or_additions() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let context = context(&source);
    let complete = generate(&fixture, &source, &scope, &context);
    let absent = generate(
        &fixture,
        &SnapshotId::new("f".repeat(40)).unwrap(),
        &scope,
        &context,
    );
    for delta in [
        compare_hld(&complete, &absent).unwrap(),
        compare_hld(&absent, &complete).unwrap(),
    ] {
        assert_eq!(delta.outcome, DesignOutcome::Incomplete);
        assert!(!delta.changes.is_empty());
        assert!(
            delta
                .changes
                .iter()
                .all(|change| change.kind == "uncertain")
        );
    }
    fixture.write("src/lib.rs", "pub fn malformed( {\n");
    let malformed = fixture.commit();
    let partial = generate(&fixture, &malformed, &scope, &context);
    let delta = compare_hld(&complete, &partial).unwrap();
    assert_eq!(delta.outcome, DesignOutcome::Incomplete);
    assert!(
        delta
            .changes
            .iter()
            .all(|change| change.kind == "uncertain")
    );
    assert!(delta.possible_impacts.is_empty());
}

#[test]
fn diagnostic_unknown_messages_do_not_change_architecture_semantics() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let context = context(&source);
    let before = generate(&fixture, &source, &scope, &context);
    let mut after = before.clone();
    for element in &mut after.elements {
        element.unknowns = vec!["Different diagnostic wording and layout".into()];
        element.facts.push(DesignFact {
            kind: "diagnostic_only".into(),
            value: "Unavailable runtime detail".into(),
            resolution: DesignResolution::Unknown,
            evidence_ids: vec![],
        });
    }
    for relation in &mut after.relations {
        relation.unknowns = vec!["Different diagnostic explanation".into()];
    }
    after.coverage.complete = false;
    after.coverage.gaps = vec!["New honest coverage diagnostic".into()];
    let delta = compare_hld(&before, &after).unwrap();
    assert!(delta.changes.is_empty());
    assert_eq!(delta.outcome, DesignOutcome::Incomplete);
    after.coverage = before.coverage.clone();
    assert_eq!(
        compare_hld(&before, &after).unwrap().outcome,
        DesignOutcome::NoChangeDetectedWithinScope
    );
}

#[test]
fn positive_public_fact_changes_remain_observed_despite_other_incomplete_inputs() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let context = context(&source);
    let before = generate(&fixture, &source, &scope, &context);
    fixture.write("src/lib.rs","use reqwest::Client;\npub trait Gateway { fn capture(&self)->u64 {1} }\npub fn run()->i32 {1}\n");
    fixture.write("other/unresolved.ts", "export const unresolved=1;");
    let target = fixture.commit();
    let after = generate(&fixture, &target, &scope, &context);
    let delta = compare_hld(&before, &after).unwrap();
    assert_eq!(delta.outcome, DesignOutcome::ChangeDetected);
    assert!(!delta.coverage.complete);
    assert!(
        delta
            .changes
            .iter()
            .any(|change| change.kind == "modified"
                && change.after_name.as_deref() == Some("Gateway"))
    );
}

#[test]
fn required_contract_is_bound_to_logical_identity_and_owner_never_rationale_text() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let context = context(&source);
    let complete = generate(&fixture, &source, &scope, &context);
    let contract = complete
        .elements
        .iter()
        .find(|element| element.kind == DesignElementKind::Contract && element.name == "Gateway")
        .unwrap();
    let rule = DesignConstraint {
        id: "required-contract".into(),
        kind: "required_contract".into(),
        from_component_id: contract.component_id.clone().unwrap(),
        to_component_id: Some(contract.id.clone()),
        value: "Expose the stable gateway interface so selected consumers remain inspectable"
            .into(),
        user_assertion: "Explicit required contract rule".into(),
        evidence_ids: contract.evidence_ids.clone(),
    };
    assert!(evaluate_hld_constraints(&complete, std::slice::from_ref(&rule)).is_empty());
    fixture.write(
        "src/lib.rs",
        "use reqwest::Client;\npub fn run()->i32 {1}\n",
    );
    let removed = fixture.commit();
    let after = generate(&fixture, &removed, &scope, &context);
    let findings = evaluate_hld_constraints(&after, std::slice::from_ref(&rule));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].resolution, DesignResolution::ObservedStructure);
    let missing = generate(
        &fixture,
        &SnapshotId::new("f".repeat(40)).unwrap(),
        &scope,
        &context,
    );
    let findings = evaluate_hld_constraints(&missing, &[rule]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].resolution, DesignResolution::Unknown);
}
#[test]
fn resolving_a_candidate_relationship_is_evidence_progress_not_architecture_change() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    let scope = scope();
    let context = context(&source);
    let mut before = generate(&fixture, &source, &scope, &context);
    let after = before.clone();
    before.relations[0].resolution = DesignResolution::NameOnlyCandidate;
    before.coverage.complete = false;
    let delta = compare_hld(&before, &after).unwrap();
    assert!(delta.changes.is_empty());
    assert_eq!(delta.outcome, DesignOutcome::Incomplete);
    assert!(
        delta
            .questions
            .iter()
            .any(|question| question.contains("certainty is not an architectural modification"))
    );
}

#[test]
fn hosting_confirmation_commit_and_verification_freshness_are_not_architecture_changes() {
    let fixture = Fixture::new();
    let source = fixture.simple();
    fixture.write(
        "vercel.json",
        "{\"framework\":\"vite\",\"outputDirectory\":\"dist\"}",
    );
    let before_commit = fixture.commit();
    let scope = scope();
    let mut old = context(&source);
    old.accepted_commit = before_commit.clone();
    let blob = fixture.git(&[
        "rev-parse",
        &format!("{}:vercel.json", before_commit.as_str()),
    ]);
    old.providers.push(crate::ComponentProviderMapping {
        component_id: "app".into(),
        provider: "Vercel".into(),
        project_id: None,
        service_id: None,
        environment: "unverified".into(),
        repository_index: 0,
        source_root: ".".into(),
        observed_commit: Some(before_commit.as_str().into()),
        verified_at_ms: None,
        verification: crate::ProviderVerification::NotRequested,
        evidence: vec![crate::OnboardingEvidence {
            repository_index: 0,
            path: "vercel.json".into(),
            blob,
            start_line: 1,
            end_line: 1,
            producer_version: "fixture-static-config".into(),
        }],
        notes: vec![],
        decision: ComponentDecision::Confirmed,
    });
    let before = generate(&fixture, &before_commit, &scope, &old);
    fixture.write(
        "README.md",
        "Design documentation candidate; no source architecture change\n",
    );
    let after_commit = fixture.commit();
    let mut fresh = old.clone();
    fresh.accepted_commit = after_commit.clone();
    fresh.input_fingerprint = "d".repeat(64);
    fresh.proposal_fingerprint = "e".repeat(64);
    fresh.session_revision = 2;
    fresh.providers[0].verification = crate::ProviderVerification::Stale;
    let after = generate(&fixture, &after_commit, &scope, &fresh);
    let delta = compare_hld(&before, &after).unwrap();
    assert!(delta.changes.is_empty(), "{:?}", delta.changes);
    assert_eq!(delta.outcome, DesignOutcome::NoChangeDetectedWithinScope);
    fresh.providers[0].source_root = "src".into();
    let changed = generate(&fixture, &after_commit, &scope, &fresh);
    assert_eq!(
        compare_hld(&after, &changed).unwrap().outcome,
        DesignOutcome::ChangeDetected
    );
}
