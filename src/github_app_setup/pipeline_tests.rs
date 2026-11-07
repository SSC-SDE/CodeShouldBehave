//! Local-only real engine fixtures. No App activation or provider requests.
use super::pr_worker::object;
use super::*;
use crate::public_report::{PublicBundle, PublicPolicy, canonical_json, sha256};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, process::Command, sync::Arc};
pub(super) struct Fixture {
    pub root: PathBuf,
    pub request: PrWorkerRequest,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!("csb-pr-pipeline-{}", secret().unwrap()));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let git = |args: &[&str]| {
            let result = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.test",
                ])
                .args(args)
                .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            result.stdout
        };
        git(&["init", "-q"]);
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::create_dir(root.join("private")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn decide(n:i32)->i32 { if n > 1 { return 2; } 0 }\n",
        )
        .unwrap();
        std::fs::write(root.join("private/secret.rs"), "EXCLUDED_SOURCE_SENTINEL").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "before"]);
        let base = String::from_utf8(git(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();
        std::fs::write(
            root.join("src/lib.rs"),
            "mod retry; pub fn decide(n:i32)->i32 { if n > 5 { return retry::retry(n); } -1 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/retry.rs"),
            "pub fn retry(n:i32)->i32 { if n > 10 { 3 } else { 4 } }\n",
        )
        .unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "after"]);
        let head = String::from_utf8(git(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();
        let policy:PublicPolicy=serde_json::from_value(json!({"schema_version":"csb-public-policy-v1","repository":{"id":3,"owner":"fixture","name":"app"},"public":true,"publication_consent":true,"include":["."],"exclude":["private"],"baseline_mode":"merge_base","max_details":100})).unwrap();
        let input = AnalysisInput {
            schema: "csb-app-analysis-v1".into(),
            app_id: 1,
            installation_id: 2,
            repository_id: 3,
            scope_revision: 1,
            consent_digest: digest(&format!("1:{}", policy.digest().unwrap())),
            engine_digest: "c".repeat(64),
            engine_metadata_digest: sha256(&canonical_json(&engine()).unwrap()),
            comparison: CurrentPullRequest {
                repository_id: 3,
                number: 7,
                selected_base: base.clone(),
                merge_base: base.clone(),
                head: head.clone(),
                open: true,
                draft: false,
            },
        };
        let component = crate::ComponentProposal {
            id: "app".into(),
            name: "Application".into(),
            role: "component".into(),
            repository_index: 0,
            roots: vec![".".into()],
            stack_candidates: vec![],
            signals: vec![],
            evidence: vec![],
            confidence: crate::EvidenceConfidence::UserSupplied,
            contradictions: vec![],
            decision: crate::ComponentDecision::Confirmed,
        };
        let context = json!({"version":"csb-accepted-context-v1","usable":true,"sensitive_paths_excluded":true,"repository":{"security_domain":"github-app-2","provider":"github","owner":"fixture","name":"app"},"confirmation":{"input_fingerprint":"a".repeat(64),"proposal_fingerprint":"b".repeat(64)},"commit":base,"session_id":"fixture-context","session_revision":1,"input_fingerprint":"a".repeat(64),"proposal_fingerprint":"b".repeat(64),"components":[component],"providers":[],"integrations":[],"gaps":[],"contexts":[]});
        let mut objects = BTreeMap::new();
        for commit in [&base, &head] {
            objects.insert(
                commit.clone(),
                object("commit", &git(&["cat-file", "commit", commit])),
            );
            let tree = String::from_utf8(git(&["rev-parse", &format!("{commit}^{{tree}}")]))
                .unwrap()
                .trim()
                .to_owned();
            objects.insert(
                tree.clone(),
                object("tree", &git(&["cat-file", "tree", &tree])),
            );
            let entries = String::from_utf8(git(&["ls-tree", "-rt", commit])).unwrap();
            for line in entries.lines() {
                let (meta, path) = line.split_once('\t').unwrap();
                let fields = meta.split_whitespace().collect::<Vec<_>>();
                let kind = fields[1];
                let oid = fields[2];
                if kind == "tree" || !path.starts_with("private/") {
                    objects.insert(oid.into(), object(kind, &git(&["cat-file", kind, oid])));
                }
            }
        }
        Self {
            root,
            request: PrWorkerRequest {
                input,
                policy,
                context,
                objects: objects.into_values().collect(),
            },
        }
    }
    pub fn queue(&self) -> Arc<AppExecutionQueue> {
        let storage =
            Storage::open(&self.root.join("state.db"), &self.root.join("artifacts")).unwrap();
        let i = &self.request.input;
        let p = &self.request.policy;
        let document = json!({"schema":"csb-app-scope-v1","installation_id":2,"repository_id":3,"commit":i.comparison.merge_base,"tree":"a".repeat(40),"default_branch":"main","include":p.include,"exclude":p.exclude,"files":["Cargo.toml","src/lib.rs"],"rust_candidates":1,"unsupported_files":0,"automatically_excluded":1,"inventory_fingerprint":"a".repeat(64),"sensitive_path_policy":"v1","source_scanned":false,"architecture_confirmed":false,"publication_consent":false});
        storage.connection.execute("INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) VALUES(1,2,3,'fixture','app',4,1)",[]).unwrap();
        storage.connection.execute("INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) VALUES(1,2,3,1,4,1,?1)",[document.to_string()]).unwrap();
        storage.connection.execute("INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) VALUES(1,2,3,1,4,1,?1)",[self.request.context.to_string()]).unwrap();
        // Only tests create enabled grants. No production consent API exists yet.
        storage.connection.execute("INSERT INTO app_execution_grants(app_id,installation_id,repository_id,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,enabled) VALUES(1,2,3,1,?1,?2,?3,?4,1)",params![i.consent_digest,i.engine_digest,i.engine_metadata_digest,serde_json::to_string(p).unwrap()]).unwrap();
        Arc::new(AppExecutionQueue::new(storage, 1).unwrap())
    }
    pub fn bundle(&self) -> PublicBundle {
        analyze_pr_objects(&self.request, &self.root.join("worker"))
            .unwrap()
            .into_bundle()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
pub(super) fn engine() -> Value {
    json!({"package":"flowdiff","version":env!("CARGO_PKG_VERSION"),"hld_generator":crate::design::HLD_GENERATOR_VERSION,"lld_generator":crate::lld::LLD_GENERATOR_VERSION,"lld_parser":crate::lld::LLD_IR_VERSION,"public_projection":crate::public_report::PUBLIC_SCHEMA})
}
pub(super) struct Provider(pub CurrentPullRequest);
impl AppExecutionProvider for Provider {
    fn current_pr(&self, i: i64, r: i64, p: i64) -> Result<CurrentPullRequest> {
        assert_eq!((i, r, p), (2, 3, 7));
        Ok(self.0.clone())
    }
}
pub(super) fn signer() -> Arc<AppReceiptSigner> {
    Arc::new(
        AppReceiptSigner::from_pkcs8(
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap(),
    )
}
#[test]
fn worker_materializes_only_authorized_exact_objects_and_generates_real_diffs() {
    let f = Fixture::new();
    let bundle = f.bundle();
    assert!(bundle.manifest.routes.hld.is_some());
    assert!(bundle.manifest.routes.lld_catalog.is_some());
    assert!(!bundle.manifest.routes.lld_details.is_empty());
    assert_eq!(
        bundle.manifest.baseline,
        f.request.input.comparison.merge_base
    );
    assert_eq!(bundle.manifest.proposal, f.request.input.comparison.head);
    let all = String::from_utf8(bundle.files.values().flatten().copied().collect()).unwrap();
    assert!(!all.contains("EXCLUDED_SOURCE_SENTINEL"));
    let sources = bundle
        .manifest
        .routes
        .file_sources
        .iter()
        .map(|f| serde_json::from_slice::<Value>(&bundle.files[&f.path]).unwrap())
        .collect::<Vec<_>>();
    assert!(
        sources
            .iter()
            .any(|v| v["text"].as_str().is_some_and(|s| s.contains("n > 1")))
    );
    assert!(
        sources
            .iter()
            .any(|v| v["text"].as_str().is_some_and(|s| s.contains("n > 5")))
    );
    let artifact: Value =
        serde_json::from_slice(&bundle.files[&bundle.manifest.routes.artifact]).unwrap();
    assert_ne!(artifact["lld_status"], "unavailable");
    let catalog: Value = serde_json::from_slice(&bundle.files["lld/catalog.json"]).unwrap();
    assert!(
        catalog["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["proposal"]["name"] == "decide")
    );
    assert!(!f.root.join("worker").exists());
}
#[test]
fn worker_rejects_extra_missing_tampered_objects_and_wrong_context() {
    let f = Fixture::new();
    let mut bad = f.request.clone();
    bad.objects
        .push(object("blob", b"EXCLUDED_SOURCE_SENTINEL"));
    assert!(analyze_pr_objects(&bad, &f.root.join("bad")).is_err());
    assert!(!f.root.join("bad").exists());
    let mut bad = f.request.clone();
    bad.objects.retain(|o| o.kind != "blob");
    assert!(analyze_pr_objects(&bad, &f.root.join("bad")).is_err());
    let mut bad = f.request.clone();
    bad.objects[0].oid = "f".repeat(40);
    assert!(analyze_pr_objects(&bad, &f.root.join("bad")).is_err());
    let mut bad = f.request.clone();
    bad.context["repository"]["security_domain"] = json!("another-installation");
    assert!(analyze_pr_objects(&bad, &f.root.join("bad")).is_err());
}
