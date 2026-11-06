use super::*;
use std::{cell::Cell, path::PathBuf};

struct Provider {
    denied: Cell<bool>,
    wrong_repository: Cell<bool>,
    calls: Cell<usize>,
}
impl AppActivationProvider for Provider {
    fn current_activation_access(
        &self,
        app: i64,
        installation: i64,
        repository: i64,
        actor: i64,
    ) -> Result<CurrentActivationAccess> {
        self.calls.set(self.calls.get() + 1);
        Ok(CurrentActivationAccess {
            app_id: app,
            installation_id: installation,
            repository_id: if self.wrong_repository.get() {
                repository + 1
            } else {
                repository
            },
            actor_id: actor,
            owner: "fixture".into(),
            name: "app".into(),
            public: true,
            archived: false,
            suspended: false,
            actor_admin: !self.denied.get(),
            contents: "read".into(),
            pull_requests: "read".into(),
            checks: "write".into(),
        })
    }
}
fn sign<T: Serialize>(document: T, pair: &Ed25519KeyPair) -> SignedActivationDocument<T> {
    let signature = URL_SAFE_NO_PAD.encode(pair.sign(&canonical_json(&document).unwrap()).as_ref());
    SignedActivationDocument {
        document,
        signature,
    }
}
struct Fixture {
    root: PathBuf,
    operator: AppActivationOperator,
    signer: Ed25519KeyPair,
    qualifier: Ed25519KeyPair,
    engine: SignedActivationDocument<ActivationEngineEvidence>,
    host: SignedActivationDocument<ActivationHostEvidence>,
    provider: Provider,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("csb-app-activation-{}", secret().unwrap()));
        let storage = Storage::open(&root.join("state.db"), &root.join("artifacts")).unwrap();
        let at = now().unwrap();
        let policy: PublicPolicy = serde_json::from_value(json!({"schema_version":"csb-public-policy-v1","repository":{"id":3,"owner":"fixture","name":"app"},"public":true,"publication_consent":true,"include":["src"],"exclude":[],"baseline_mode":"merge_base","max_details":100})).unwrap();
        let scope = json!({"schema":"csb-app-scope-v1","installation_id":2,"repository_id":3,"commit":"a".repeat(40),"tree":"b".repeat(40),"default_branch":"main","include":["src"],"exclude":[],"files":["src/lib.rs"],"rust_candidates":1,"unsupported_files":0,"automatically_excluded":0,"inventory_fingerprint":"a".repeat(64),"sensitive_path_policy":"v1","source_scanned":false,"architecture_confirmed":false,"publication_consent":false});
        let context = json!({"usable":true,"confirmation":{"actor":"github-user-4"}}).to_string();
        storage.connection.execute("INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) VALUES(1,2,3,'fixture','app',4,1)", []).unwrap();
        storage.connection.execute("INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) VALUES(1,2,3,1,4,1,?1)", [scope.to_string()]).unwrap();
        storage.connection.execute("INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) VALUES(1,2,3,1,4,1,?1)", [&context]).unwrap();
        let runtime = ActivationRuntimePins {
            authentication_origin: "https://auth.example.test".into(),
            report_origin: "https://reports.example.test".into(),
            engine_digest: "a".repeat(64),
            engine_metadata_digest: "b".repeat(64),
            launcher_digest: "c".repeat(64),
            interpreter_digest: "d".repeat(64),
            host_identity: "e".repeat(64),
            host_target: "x86_64-unknown-linux-gnu".into(),
        };
        let consent = json!({"schema":"csb-app-publication-consent-v1","app_id":1,"installation_id":2,"repository_id":3,"actor_id":4,"expected_revision":0,"scope_revision":1,"scope_commit":"a".repeat(40),"context_digest":sha256(context.as_bytes()),"operator":{"report_origin":runtime.report_origin,"engine_digest":runtime.engine_digest,"engine_metadata_digest":runtime.engine_metadata_digest},"policy":policy,"expires_at":at+600,"activation":"off"});
        let encoded = canonical_json(&consent).unwrap();
        storage.connection.execute("INSERT INTO app_publication_consents(app_id,installation_id,repository_id,revision,actor_id,created_at,document,digest) VALUES(1,2,3,1,4,?1,?2,?3)", params![at, String::from_utf8(encoded.clone()).unwrap(), sha256(&encoded)]).unwrap();
        // Deterministic local signing fixtures are not real release/host evidence.
        let signer = Ed25519KeyPair::from_seed_unchecked(&[1; 32]).unwrap();
        let engine_signer = Ed25519KeyPair::from_seed_unchecked(&[2; 32]).unwrap();
        let qualifier = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
        let engine = sign(
            ActivationEngineEvidence {
                schema: "csb-app-signed-engine-verification-v1".into(),
                engine_digest: runtime.engine_digest.clone(),
                engine_metadata_digest: runtime.engine_metadata_digest.clone(),
                target: runtime.host_target.clone(),
                provenance_digest: "f".repeat(64),
                verified_at: at - 1,
                expires_at: at + 3600,
            },
            &engine_signer,
        );
        let host = sign(
            ActivationHostEvidence {
                schema: "csb-app-host-qualification-v1".into(),
                runtime: runtime.clone(),
                engine_evidence_digest: engine.digest().unwrap(),
                sandbox_profile: "csb-linux-bwrap-prlimit-v1".into(),
                qualification_evidence_digest: "9".repeat(64),
                qualified_at: at - 1,
                expires_at: at + 3600,
            },
            &qualifier,
        );
        let authorities = ActivationAuthorities {
            operator: signer.public_key().as_ref().try_into().unwrap(),
            engine_verifier: engine_signer.public_key().as_ref().try_into().unwrap(),
            host_qualifier: qualifier.public_key().as_ref().try_into().unwrap(),
        };
        let operator = AppActivationOperator::new(storage, 1, runtime, authorities).unwrap();
        Self {
            root,
            operator,
            signer,
            qualifier,
            engine,
            host,
            provider: Provider {
                denied: Cell::new(false),
                wrong_repository: Cell::new(false),
                calls: Cell::new(0),
            },
        }
    }
    fn approval(&self) -> SignedActivationDocument<ActivationDocument> {
        sign(
            self.operator
                .preview(
                    ActivationSelection {
                        installation_id: 2,
                        repository_id: 3,
                        expected_consent_revision: 1,
                        expected_activation_revision: 0,
                    },
                    &self.engine,
                    &self.host,
                    &self.provider,
                )
                .unwrap(),
            &self.signer,
        )
    }
    fn commit(
        &self,
        approval: &SignedActivationDocument<ActivationDocument>,
    ) -> Result<ActivationCommit> {
        self.operator
            .commit(approval, &self.engine, &self.host, &self.provider)
    }
    fn sql(&self, sql: &str) {
        self.operator
            .queue
            .storage
            .lock()
            .unwrap()
            .connection
            .execute_batch(sql)
            .unwrap();
    }
    fn counts(&self) -> (i64, i64) {
        self.operator.queue.storage.lock().unwrap().connection.query_row(
            "SELECT (SELECT count(*) FROM app_execution_grants WHERE enabled=1),(SELECT count(*) FROM app_execution_activations)", [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn explicit_exact_approval_is_required_and_idempotent() {
    let f = Fixture::new();
    let approval = f.approval();
    assert_eq!(f.counts(), (0, 0));
    let mut unsigned = approval.clone();
    unsigned.signature.clear();
    assert!(f.commit(&unsigned).is_err());
    assert_eq!(f.counts(), (0, 0));
    let first = f.commit(&approval).unwrap();
    assert_eq!(first.revision, 1);
    assert!(!first.already_committed);
    assert_eq!(f.counts(), (1, 1));
    {
        let storage = f.operator.queue.storage.lock().unwrap();
        let grant = super::super::grant(&storage.connection, 1, 2, 3).unwrap();
        assert_eq!(grant.consent, approval.document.consent_digest);
        assert_eq!(grant.engine, approval.document.runtime.engine_digest);
    }
    assert!(f.commit(&approval).unwrap().already_committed);
    assert_eq!(f.counts(), (1, 1));
    f.sql("UPDATE app_execution_grants SET enabled=0");
    assert_eq!(f.commit(&approval), Err(SetupError::Conflict));
    assert_eq!(f.counts(), (0, 1));
}
#[test]
fn missing_or_forged_qualification_never_enables() {
    let f = Fixture::new();
    let approval = f.approval();
    let mut host = f.host.clone();
    host.signature.clear();
    assert!(
        f.operator
            .commit(&approval, &f.engine, &host, &f.provider)
            .is_err()
    );
    let mut engine = f.engine.clone();
    engine.document.engine_digest = "1".repeat(64);
    assert!(
        f.operator
            .commit(&approval, &engine, &f.host, &f.provider)
            .is_err()
    );
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn independently_signed_origin_binary_and_host_mismatches_fail() {
    let f = Fixture::new();
    let approval = f.approval();
    for field in ["origin", "binary", "host"] {
        let mut document = f.host.document.clone();
        match field {
            "origin" => document.runtime.report_origin = "https://elsewhere.example.test".into(),
            "binary" => document.runtime.engine_digest = "1".repeat(64),
            _ => document.runtime.host_identity = "2".repeat(64),
        }
        let host = sign(document, &f.qualifier);
        assert!(
            f.operator
                .commit(&approval, &f.engine, &host, &f.provider)
                .is_err()
        );
    }
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn expired_approval_and_qualification_never_enable() {
    let f = Fixture::new();
    let mut document = f.approval().document;
    document.reviewed_at = now().unwrap() - 600;
    document.expires_at = now().unwrap();
    assert!(f.commit(&sign(document, &f.signer)).is_err());
    let mut document = f.host.document.clone();
    document.expires_at = now().unwrap();
    let expired_host = sign(document, &f.qualifier);
    assert!(
        f.operator
            .preview(
                ActivationSelection {
                    installation_id: 2,
                    repository_id: 3,
                    expected_consent_revision: 1,
                    expected_activation_revision: 0
                },
                &f.engine,
                &expired_host,
                &f.provider
            )
            .is_err()
    );
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn stale_consent_never_overwrites_a_newer_decision() {
    let f = Fixture::new();
    let approval = f.approval();
    f.sql("INSERT INTO app_publication_consents(app_id,installation_id,repository_id,revision,actor_id,created_at,document,digest) SELECT app_id,installation_id,repository_id,2,actor_id,created_at,document,digest FROM app_publication_consents WHERE revision=1");
    assert_eq!(f.commit(&approval), Err(SetupError::Conflict));
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn revoked_consent_and_sticky_scope_revocation_fail_closed() {
    let f = Fixture::new();
    let approval = f.approval();
    f.sql("UPDATE app_publication_consents SET revoked=1");
    assert!(f.commit(&approval).is_err());
    assert_eq!(f.counts(), (0, 0));
    let f = Fixture::new();
    let approval = f.approval();
    f.sql("INSERT INTO app_repository_blocks(app_id,installation_id,repository_id,reason,changed_at_s) VALUES(1,2,3,'fixture revocation',1)");
    assert_eq!(f.commit(&approval), Err(SetupError::Denied));
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn fresh_actor_permission_and_cross_repo_rechecks_fail_closed() {
    let f = Fixture::new();
    let approval = f.approval();
    f.provider.denied.set(true);
    assert_eq!(f.commit(&approval), Err(SetupError::Denied));
    f.provider.denied.set(false);
    f.provider.wrong_repository.set(true);
    assert_eq!(f.commit(&approval), Err(SetupError::Denied));
    f.provider.wrong_repository.set(false);
    let mut document = approval.document.clone();
    document.repository_id = 4;
    assert!(f.commit(&sign(document, &f.signer)).is_err());
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn new_scope_context_and_expected_revision_are_fenced() {
    let f = Fixture::new();
    let approval = f.approval();
    let mut wrong = approval.document.clone();
    wrong.expected_activation_revision = 1;
    assert_eq!(f.commit(&sign(wrong, &f.signer)), Err(SetupError::Conflict));
    f.sql("INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) SELECT app_id,installation_id,repository_id,2,actor_id,confirmed_at_s,document FROM app_setup_scopes WHERE revision=1");
    assert_eq!(f.commit(&approval), Err(SetupError::Conflict));
    assert_eq!(f.counts(), (0, 0));
}
#[test]
fn failed_grant_write_rolls_back_approval_consumption_and_audit() {
    let f = Fixture::new();
    let approval = f.approval();
    f.sql("CREATE TRIGGER fixture_reject_grant BEFORE INSERT ON app_execution_grants BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;");
    assert!(f.commit(&approval).is_err());
    assert_eq!(f.counts(), (0, 0));
    f.sql("DROP TRIGGER fixture_reject_grant");
    f.commit(&approval).unwrap();
    assert_eq!(f.counts(), (1, 1));
    let c = f.operator.queue.storage.lock().unwrap();
    assert!(
        c.connection
            .execute("DELETE FROM app_execution_activations", [])
            .is_err()
    );
    assert!(
        c.connection
            .execute("UPDATE app_execution_activations SET committed_at=1", [])
            .is_err()
    );
}
#[test]
fn consumed_approval_cannot_replay_over_newer_activation() {
    let f = Fixture::new();
    let first = f.approval();
    f.commit(&first).unwrap();
    let next = sign(
        f.operator
            .preview(
                ActivationSelection {
                    installation_id: 2,
                    repository_id: 3,
                    expected_consent_revision: 1,
                    expected_activation_revision: 1,
                },
                &f.engine,
                &f.host,
                &f.provider,
            )
            .unwrap(),
        &f.signer,
    );
    f.commit(&next).unwrap();
    assert_eq!(f.commit(&first), Err(SetupError::Conflict));
    assert_eq!(f.counts(), (1, 2));
}

#[test]
fn exact_actor_and_immutable_accepted_context_are_bound() {
    let f = Fixture::new();
    let approval = f.approval();
    let mut different_actor = approval.document.clone();
    different_actor.actor_id = 5;
    assert!(f.commit(&sign(different_actor, &f.signer)).is_err());
    // Simulate an inconsistent restored DB: even matching scope/consent revisions
    // cannot authorize a different accepted context.
    f.sql("DELETE FROM app_architecture_acceptances; INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) VALUES(1,2,3,1,4,1,'{\"usable\":true,\"confirmation\":{\"actor\":\"github-user-5\"}}')");
    assert!(f.commit(&approval).is_err());
    assert_eq!(f.counts(), (0, 0));
}
