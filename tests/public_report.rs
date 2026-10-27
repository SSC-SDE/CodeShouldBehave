#![cfg(feature = "server")]
use flowdiff::design::{ConfirmedDesignContext, generate_hld};
use flowdiff::lld::{LldReview, compare_lld, generate_lld};
use flowdiff::public_report::{
    ExpectedComparison, PublicPolicy, PublicRepository, canonical_json, export_public_report,
    sha256, verify_bundle, write_bundle,
};
use flowdiff::{
    Cancellation, ComponentDecision, ComponentProposal, EvidenceConfidence, PathScope,
    RepositoryId, ScopeRecord, ScopeVersion, SecurityDomain, SnapshotId,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    artifact: Value,
    policy: PublicPolicy,
    expected: ExpectedComparison,
}
impl Fixture {
    fn new() -> Self {
        Self::with_proposal("pub fn guard(n:i32)->i32 { if n > 5 { return 3; } 0 }\n")
    }
    fn with_proposal(proposed_source: &str) -> Self {
        Self::with_inventory(proposed_source, "")
    }
    fn with_inventory(proposed_source: &str, unchanged_source: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "csb-public-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let git = |args: &[&str]| {
            let result = Command::new("git")
                .args(["-c", "core.hooksPath=/dev/null", "-C"])
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            String::from_utf8(result.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Public fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        fs::create_dir(root.join("src")).unwrap();
        fs::create_dir(root.join("private")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='public-fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn guard(n:i32)->i32 { if n > 1 { return 2; } 0 }\n",
        )
        .unwrap();
        fs::write(
            root.join("private/internal.rs"),
            "pub fn EXCLUDED_PRIVATE_SOURCE_SENTINEL(){}\n",
        )
        .unwrap();
        if !unchanged_source.is_empty() {
            fs::write(root.join("src/a_inventory.rs"), unchanged_source).unwrap();
        }
        git(&["add", "."]);
        git(&["commit", "-qm", "baseline"]);
        let baseline = git(&["rev-parse", "HEAD"]);
        fs::write(root.join("src/lib.rs"), proposed_source).unwrap();
        git(&["add", "."]);
        git(&["commit", "--allow-empty", "-qm", "proposal"]);
        let proposal = git(&["rev-parse", "HEAD"]);
        let repository = RepositoryId::new(
            SecurityDomain::new("PRIVATE_DOMAIN_SENTINEL").unwrap(),
            "github",
            "fixture",
            "public",
        )
        .unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("PRIVATE_SCOPE_VERSION_SENTINEL").unwrap(),
            primary_repository: repository,
            primary_paths: PathScope::new(vec![".".into()], vec!["private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: true,
            model_destinations: BTreeSet::new(),
        };
        let context = ConfirmedDesignContext {
            accepted_commit: SnapshotId::new(baseline.clone()).unwrap(),
            session_id: "PRIVATE_OPERATOR_SENTINEL".into(),
            session_revision: 1,
            input_fingerprint: "a".repeat(64),
            proposal_fingerprint: "b".repeat(64),
            components: vec![ComponentProposal {
                id: "app".into(),
                name: "PRIVATE_COMPONENT_LABEL_SENTINEL".into(),
                role: "PRIVATE_ROLE_SENTINEL".into(),
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
        let mut snapshots = vec![];
        for commit in [&baseline, &proposal] {
            let commit = SnapshotId::new(commit.clone()).unwrap();
            let hld =
                generate_hld(&root, &commit, &scope, &context, &Cancellation::default()).unwrap();
            let lld = generate_lld(
                &root,
                &commit,
                &scope,
                &context,
                &hld,
                &Cancellation::default(),
            )
            .unwrap();
            snapshots.push((hld, lld));
        }
        let (after_hld, after_lld) = snapshots.pop().unwrap();
        let (before_hld, before_lld) = snapshots.pop().unwrap();
        let delta = compare_lld(&before_lld.catalog, &after_lld.catalog);
        let lld = LldReview {
            baseline: before_lld,
            proposal: after_lld,
            delta,
        };
        let artifact = json!({"artifact_version":"csb-review-artifact-v1","review_id":"a".repeat(64),"selected_baseline":baseline,"baseline":baseline,"proposal":proposal,"scope_version":scope.version.as_str(),"scope_manifest":{"primary_repository":{"security_domain":"PRIVATE_DOMAIN_SENTINEL","provider":"github","owner":"fixture","name":"public"},"include":["."],"exclude":["private"],"export_allowed":true,"contexts":[]},"files":[{"path":"src/lib.rs","old_path":null,"kind":"modified","additions":1,"deletions":1}],"architecture":{"nodes":[{"path":"src/lib.rs","status":"modified","sourceBefore":"RAW_PRIVATE_SERIALIZATION_SENTINEL","sourceAfter":"RAW_PRIVATE_SERIALIZATION_SENTINEL","definitionsBefore":["guard"],"definitionsAfter":["guard"]}],"edges":[]},"coverage":{"indexed":2,"unsupported":0,"excluded":1,"failed":0,"deferred":0,"enumeration_complete":true},"warnings":["PRIVATE_WARNING_SENTINEL"],"onboarding_context":{"operator":"PRIVATE_OPERATOR_SENTINEL"},"hld":{"baseline":before_hld,"proposal":after_hld},"lld":lld});
        let policy:PublicPolicy=serde_json::from_value(json!({"schema_version":"csb-public-policy-v1","repository":{"id":42,"owner":"fixture","name":"public"},"public":true,"publication_consent":true,"include":["."],"exclude":["private"],"baseline_mode":"exact","max_details":100})).unwrap();
        let expected = ExpectedComparison {
            repository: PublicRepository {
                id: 42,
                owner: "fixture".into(),
                name: "public".into(),
            },
            pr_number: 7,
            selected_baseline: baseline.clone(),
            baseline,
            proposal,
        };
        Self {
            root,
            artifact,
            policy,
            expected,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn public_projection_is_deterministic_and_retains_real_behavior_without_private_ir() {
    let f = Fixture::new();
    let first = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let second = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    assert_eq!(
        first.manifest.public_report_id,
        second.manifest.public_report_id
    );
    assert_eq!(first.files, second.files);
    assert_ne!(
        first.manifest.public_report_id,
        first.manifest.analysis_review_id
    );
    let output = String::from_utf8(first.files.values().flatten().copied().collect()).unwrap();
    for value in [
        "PRIVATE_DOMAIN_SENTINEL",
        "PRIVATE_OPERATOR_SENTINEL",
        "PRIVATE_COMPONENT_LABEL_SENTINEL",
        "PRIVATE_ROLE_SENTINEL",
        "PRIVATE_WARNING_SENTINEL",
        "EXCLUDED_PRIVATE_SOURCE_SENTINEL",
        "RAW_PRIVATE_SERIALIZATION_SENTINEL",
        "\"inputs\"",
        "\"tokens\"",
        "onboarding_session_id",
        "PRIVATE_SCOPE_VERSION_SENTINEL",
    ] {
        assert!(!output.contains(value), "leaked {value}");
    }
    assert!(
        output.contains("local_flow_changed")
            || output.contains("body_changed")
            || output.contains("body_fingerprint")
    );
    assert!(first.manifest.routes.sources.len() > 2);
    assert!(!first.manifest.routes.lld_details.is_empty());
    for file in first.manifest.routes.file_sources.iter().map(|s| &s.path) {
        let source: Value = serde_json::from_slice(&first.files[file]).unwrap();
        assert!(source["text"].as_str().unwrap().contains("pub fn guard"));
        assert_eq!(source["path"], "src/lib.rs");
    }
    if let Some(directory) = std::env::var_os("CSB_PUBLIC_REPORT_FIXTURE") {
        let directory = PathBuf::from(directory);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("private-artifact.json"),
            canonical_json(&f.artifact).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join("policy.json"),
            canonical_json(&f.policy).unwrap(),
        )
        .unwrap();
        fs::write(directory.join("expected.json"),canonical_json(&json!({"repository":f.expected.repository,"pr_number":f.expected.pr_number,"selected_baseline":f.expected.selected_baseline,"baseline":f.expected.baseline,"proposal":f.expected.proposal})).unwrap()).unwrap();
        write_bundle(&first, &directory.join("bundle")).unwrap();
    }
}
#[test]
fn explicit_public_consent_pins_and_scope_are_mandatory() {
    let mut f = Fixture::new();
    f.policy.publication_consent = false;
    assert!(export_public_report(&f.artifact, &f.policy, &f.expected).is_err());
    f.policy.publication_consent = true;
    f.expected.repository.id = 9;
    assert!(export_public_report(&f.artifact, &f.policy, &f.expected).is_err());
    f.expected.repository.id = 42;
    f.expected.proposal = "f".repeat(40);
    assert!(export_public_report(&f.artifact, &f.policy, &f.expected).is_err());
    f.expected.proposal = f.artifact["proposal"].as_str().unwrap().into();
    f.artifact["scope_manifest"]["contexts"] = json!([{"name":"private-related"}]);
    assert!(export_public_report(&f.artifact, &f.policy, &f.expected).is_err());
    f.artifact["scope_manifest"]["contexts"] = json!([]);
    f.policy.include = vec!["src".into()];
    assert!(export_public_report(&f.artifact, &f.policy, &f.expected).is_err());
}
#[test]
fn sensitive_and_traversal_paths_are_denied_before_projection() {
    let mut f = Fixture::new();
    for path in [
        "../outside.rs",
        "src/../outside.rs",
        ".env",
        "src/secret.rs",
        "src/%2e%2e/other.rs",
        "/etc/passwd",
        "src\\other.rs",
    ] {
        f.artifact["files"][0]["path"] = json!(path);
        assert!(
            export_public_report(&f.artifact, &f.policy, &f.expected).is_err(),
            "{path}"
        );
    }
}
#[test]
fn tampered_retained_source_cannot_be_republished_with_a_recomputed_checksum() {
    use sha2::{Digest, Sha256};
    let mut f = Fixture::new();
    let input = f.artifact["lld"]["proposal"]["inputs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|i| i["path"] == "src/lib.rs")
        .unwrap();
    let source = input["source"].as_str().unwrap().replace("n > 5", "n > 9");
    let mut hash = Sha256::new();
    hash.update((source.len() as u64).to_be_bytes());
    hash.update(source.as_bytes());
    input["source"] = json!(source);
    input["source_fingerprint"] = json!(
        hash.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert!(
        export_public_report(&f.artifact, &f.policy, &f.expected)
            .unwrap_err()
            .contains("blob")
    );
}
#[test]
fn missing_hld_or_exhausted_detail_budget_cannot_claim_complete() {
    let mut f = Fixture::new();
    f.policy.max_details = 0;
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    assert!(!bundle.manifest.coverage.complete);
    assert!(bundle.manifest.coverage.deferred_details > 0);
    assert!(bundle.manifest.routes.lld_details.is_empty());
    f.artifact["hld"] = Value::Null;
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    assert!(!bundle.manifest.coverage.complete);
    assert_eq!(bundle.manifest.capabilities["hld"], false);
}
#[test]
fn unknown_policy_fields_and_duplicate_paths_fail_closed() {
    let f = Fixture::new();
    let mut policy = serde_json::to_value(&f.policy).unwrap();
    policy["token"] = json!("NOT_A_REAL_SECRET");
    assert!(serde_json::from_value::<PublicPolicy>(policy).is_err());
    let mut p = f.policy.clone();
    p.include.push(".".into());
    assert!(p.validate().is_err());
}
#[test]
fn verified_directory_rejects_tamper_extra_files_and_overwrite() {
    let f = Fixture::new();
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let out = f.root.join("bundle");
    write_bundle(&bundle, &out).unwrap();
    assert_eq!(
        verify_bundle(&out).unwrap().public_report_id,
        bundle.manifest.public_report_id
    );
    assert!(write_bundle(&bundle, &out).is_err());
    fs::write(out.join("extra.json"), "{}").unwrap();
    assert!(verify_bundle(&out).is_err());
    fs::remove_file(out.join("extra.json")).unwrap();
    fs::write(out.join("artifact.json"), "{}").unwrap();
    assert!(verify_bundle(&out).is_err());
}
#[cfg(unix)]
#[test]
fn output_and_input_symlinks_are_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let link = f.root.join("link");
    symlink(&f.root, &link).unwrap();
    assert!(write_bundle(&bundle, &link.join("bundle")).is_err());
    let out = f.root.join("bundle");
    write_bundle(&bundle, &out).unwrap();
    symlink(f.root.join("Cargo.toml"), out.join("unexpected")).unwrap();
    assert!(verify_bundle(&out).is_err());
}
#[test]
fn derived_identity_binds_policy_and_file_content() {
    let f = Fixture::new();
    let mut a = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let original = a.manifest.public_report_id.clone();
    a.files.get_mut("artifact.json").unwrap().push(b' ');
    assert!(a.validate().is_err());
    let mut p = f.policy.clone();
    p.max_details = 1;
    let b = export_public_report(&f.artifact, &p, &f.expected).unwrap();
    assert_ne!(original, b.manifest.public_report_id);
    assert_eq!(sha256(&canonical_json(&p).unwrap()).len(), 64);
}

fn resign(bundle: &mut flowdiff::public_report::PublicBundle) {
    bundle.manifest.public_report_id = bundle.manifest.identity().unwrap();
    bundle.manifest.review_id = bundle.manifest.public_report_id.clone();
}
#[test]
fn recomputed_manifest_cannot_rebind_parent_component_or_source_evidence() {
    let f = Fixture::new();
    let original = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let mut changed = original.clone();
    changed.manifest.routes.lld_details[0].parent_hld = "other-parent".into();
    resign(&mut changed);
    assert!(changed.validate().is_err());
    let mut changed = original.clone();
    changed.manifest.routes.lld_details[0].component = Some("other-component".into());
    resign(&mut changed);
    assert!(changed.validate().is_err());
    let mut changed = original.clone();
    let source = changed
        .manifest
        .routes
        .sources
        .iter_mut()
        .find(|r| r.entry.is_some())
        .unwrap();
    source.evidence = Some("other-evidence".into());
    resign(&mut changed);
    assert!(changed.validate().is_err());
    let mut changed = original.clone();
    let source = changed
        .manifest
        .routes
        .sources
        .iter_mut()
        .find(|r| r.entry.is_some())
        .unwrap();
    source.component = Some("other-component".into());
    resign(&mut changed);
    assert!(changed.validate().is_err());
    let mut artifact = f.artifact.clone();
    artifact["lld"]["baseline"]["catalog"]["hld_design_id"] = json!("other-parent");
    assert!(export_public_report(&artifact, &f.policy, &f.expected).is_err());
    let mut artifact = f.artifact.clone();
    artifact["lld"]["proposal"]["catalog"]["evidence"][0]["path"] = json!("private/internal.rs");
    assert!(export_public_report(&artifact, &f.policy, &f.expected).is_err());
}

#[test]
fn unchecked_operator_diagnostics_titles_and_stored_renderings_are_not_public_fields() {
    use sha2::{Digest, Sha256};
    let mut f = Fixture::new();
    for side in ["baseline", "proposal"] {
        f.artifact["hld"][side]["sections"][0]["title"] = json!("PRIVATE_SECTION_TITLE_SENTINEL");
        f.artifact["hld"][side]["markdown"] = json!("PRIVATE_MARKDOWN_SENTINEL");
        f.artifact["hld"][side]["mermaid"] = json!("PRIVATE_MERMAID_SENTINEL");
        f.artifact["hld"][side]["coverage"]["profile"] = json!("PRIVATE_PROFILE_SENTINEL");
    }
    let mut pair: LldReview = serde_json::from_value(f.artifact["lld"].clone()).unwrap();
    pair.baseline
        .catalog
        .coverage
        .gaps
        .push("PRIVATE_GAP_SENTINEL".into());
    pair.baseline.catalog.coverage.structure_complete = false;
    let catalog = &mut pair.baseline.catalog;
    let parts = [
        flowdiff::lld::LLD_GENERATOR_VERSION.to_string(),
        flowdiff::lld::LLD_IR_VERSION.to_string(),
        flowdiff::lld::LLD_PROFILE.to_string(),
        catalog.snapshot.as_str().to_string(),
        catalog.scope_version.as_str().to_string(),
        catalog.scope_fingerprint.clone(),
        catalog.hld_design_id.clone(),
        serde_json::to_string(&catalog.entries).unwrap(),
        serde_json::to_string(&catalog.coverage).unwrap(),
    ];
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    catalog.id = format!(
        "lld-catalog-{}",
        hash.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    f.artifact["lld"] = serde_json::to_value(pair).unwrap();
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let text = String::from_utf8(bundle.files.values().flatten().copied().collect()).unwrap();
    for sentinel in [
        "PRIVATE_SECTION_TITLE_SENTINEL",
        "PRIVATE_MARKDOWN_SENTINEL",
        "PRIVATE_MERMAID_SENTINEL",
        "PRIVATE_PROFILE_SENTINEL",
        "PRIVATE_GAP_SENTINEL",
    ] {
        assert!(!text.contains(sentinel), "{sentinel}");
    }
    assert!(!bundle.manifest.coverage.complete);
}

#[test]
fn supported_unchanged_graphs_are_aggregated_only_after_full_precomputation() {
    let mut f = Fixture::with_proposal("pub fn guard(n:i32)->i32 { if n > 1 { return 2; } 0 }\n");
    f.artifact["files"] = json!([]);
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    assert!(
        bundle.manifest.coverage.complete,
        "{:?}",
        bundle.manifest.coverage.gaps
    );
    let artifact: Value = serde_json::from_slice(&bundle.files["artifact.json"]).unwrap();
    assert_eq!(artifact["lld_status"], "no_change_detected_within_scope");
    let catalog: Value = serde_json::from_slice(&bundle.files["lld/catalog.json"]).unwrap();
    assert_eq!(
        catalog["delta"]["outcome"],
        "no_change_detected_within_scope"
    );
    assert!(
        catalog["delta"]["id"]
            .as_str()
            .unwrap()
            .starts_with("public-lld-delta-")
    );
    f.policy.max_details = 0;
    assert!(
        !export_public_report(&f.artifact, &f.policy, &f.expected)
            .unwrap()
            .manifest
            .coverage
            .complete
    );
}

#[test]
fn strict_loader_rejects_duplicate_keys_at_every_depth() {
    let f = Fixture::new();
    let path = f.root.join("duplicate.json");
    for json in [r#"{"key":1,"key":2}"#, r#"[{"nested":{"key":1,"key":2}}]"#] {
        fs::write(&path, json).unwrap();
        assert!(flowdiff::public_report::read_json_file(&path, 1024).is_err());
    }
    fs::write(&path, r#"{"nested":[true,null,1,-2,0.5,"safe"]}"#).unwrap();
    assert!(flowdiff::public_report::read_json_file(&path, 1024).is_ok());
    let mut bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let original = bundle.files["artifact.json"].clone();
    let mut duplicate = br#"{"analysis_review_id":"PRIVATE_DUPLICATE_SENTINEL","#.to_vec();
    duplicate.extend(&original[1..]);
    let metadata = bundle
        .manifest
        .files
        .iter_mut()
        .find(|p| p.path == "artifact.json")
        .unwrap();
    metadata.bytes = duplicate.len();
    metadata.sha256 = sha256(&duplicate);
    bundle.files.insert("artifact.json".into(), duplicate);
    resign(&mut bundle);
    assert!(bundle.validate().is_err());
}

#[test]
fn actual_manifest_bytes_count_toward_the_directory_cap() {
    let f = Fixture::new();
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    let directory = f.root.join("padded");
    write_bundle(&bundle, &directory).unwrap();
    let manifest = canonical_json(&bundle.manifest).unwrap();
    let payload = bundle.files.values().map(Vec::len).sum::<usize>();
    let padding = flowdiff::public_report::MAX_PUBLIC_BYTES - payload - manifest.len() + 1;
    let mut bytes = vec![b' '; padding];
    bytes.extend(manifest);
    fs::write(directory.join("manifest.json"), bytes).unwrap();
    assert!(
        verify_bundle(&directory)
            .unwrap_err()
            .contains("byte limit")
    );
}

#[test]
fn large_declaration_inventory_keeps_changed_details_within_public_budget() {
    let mut source = String::from("pub fn guard(n:i32)->i32 { if n > 1 { return 2; } 0 }\n");
    for index in 0..160 {
        source.push_str(&format!(
            "pub fn changed_{index}(n:i32)->i32 {{ if n > 10 {{ return n; }} n + 1 }}\n"
        ));
    }
    let mut f = Fixture::with_proposal(&source);
    f.policy.max_details = 100;
    let bundle = export_public_report(&f.artifact, &f.policy, &f.expected).unwrap();
    assert!(bundle.manifest.routes.hld.is_some());
    assert!(bundle.manifest.routes.lld_catalog.is_some());
    assert!(!bundle.manifest.routes.lld_details.is_empty());
    assert!(bundle.manifest.coverage.deferred_details > 0);
    assert!(!bundle.manifest.coverage.complete);
    assert!(bundle.files.values().map(Vec::len).sum::<usize>() < 8 * 1024 * 1024);
}

#[test]
fn changed_file_precedes_unchanged_inventory_in_published_catalog() {
    let inventory = (0..80)
        .map(|i| format!("pub fn unchanged_{i}() -> u64 {{ {i} }}\n"))
        .collect::<String>();
    let fixture = Fixture::with_inventory(
        "pub fn guard(n:i32)->i32 { if n > 5 { return 3; } 0 }",
        &inventory,
    );
    let bundle =
        export_public_report(&fixture.artifact, &fixture.policy, &fixture.expected).unwrap();
    let catalog: Value = serde_json::from_slice(&bundle.files["lld/catalog.json"]).unwrap();
    let entries = catalog["entries"].as_array().unwrap();
    assert_eq!(entries[0]["proposal"]["path"], "src/lib.rs");
    assert!(
        entries
            .iter()
            .any(|e| e["proposal"]["path"] == "src/a_inventory.rs")
    );
    assert!(
        entries
            .iter()
            .take(50)
            .any(|e| e["proposal"]["name"] == "guard")
    );
}

#[test]
fn large_hld_markdown_keeps_diagram_and_evidence_without_duplicate_inventory() {
    let inventory = (0..600)
        .map(|i| {
            format!(
                "pub struct Contract{i} {{ pub payload: [u64; 32], pub metadata: [u8; 128] }}\n"
            )
        })
        .collect::<String>();
    let fixture = Fixture::with_inventory(
        "pub fn guard(n:i32)->i32 { if n > 5 { return 3; } 0 }",
        &inventory,
    );
    let bundle =
        export_public_report(&fixture.artifact, &fixture.policy, &fixture.expected).unwrap();
    let hld: Value = serde_json::from_slice(&bundle.files["hld.json"]).unwrap();
    for side in ["baseline", "proposal"] {
        assert!(hld[side]["markdown"].as_str().unwrap().len() < 64 * 1024);
        assert!(
            hld[side]["markdown"]
                .as_str()
                .unwrap()
                .contains("presentation budget")
        );
        assert!(!hld[side]["mermaid"].as_str().unwrap().is_empty());
        assert_eq!(
            hld[side]["elements"].as_array().unwrap().len(),
            fixture.artifact["hld"][side]["elements"]
                .as_array()
                .unwrap()
                .len()
        );
        assert!(!hld[side]["evidence"].as_array().unwrap().is_empty());
    }
    assert!(!bundle.manifest.routes.lld_details.is_empty());
    assert!(!bundle.manifest.routes.sources.is_empty());
    assert!(bundle.files.values().map(Vec::len).sum::<usize>() < 7 * 1024 * 1024);
    bundle.validate().unwrap();
}

#[test]
fn modified_public_counterpart_keeps_first_detail_and_both_source_sides() {
    let mut source = (0..80)
        .map(|i| format!("pub fn added_{i}(n:i32)->i32 {{ if n > 3 {{ return n; }} 0 }}\n"))
        .collect::<String>();
    source.push_str("pub fn guard(n:i32)->i32 { if n > 9 { return 4; } 0 }\n");
    let mut fixture = Fixture::with_proposal(&source);
    fixture.policy.max_details = 1;
    let bundle =
        export_public_report(&fixture.artifact, &fixture.policy, &fixture.expected).unwrap();
    let catalog: Value = serde_json::from_slice(&bundle.files["lld/catalog.json"]).unwrap();
    let entry = &catalog["entries"][0];
    assert_eq!(entry["proposal"]["name"], "guard");
    assert_eq!(entry["baseline"]["detail_state"], "published");
    assert_eq!(entry["proposal"]["detail_state"], "published");
    for side in ["baseline", "proposal"] {
        let id = entry[side]["id"].as_str().unwrap();
        assert!(
            bundle
                .manifest
                .routes
                .sources
                .iter()
                .any(|route| route.entry.as_deref() == Some(id))
        );
    }
    assert!(bundle.manifest.coverage.deferred_details > 0);
    bundle.validate().unwrap();
}
