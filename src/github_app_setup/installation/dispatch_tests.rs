//! Offline composition tests. GitHub writes and consent are test fixtures only.
use super::*;
use crate::github_app_setup::{pipeline_tests::Fixture, pipeline_tests::signer, *};
use crate::public_report::PublicBundle;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct Api {
    objects: BTreeMap<String, Value>,
    comparison: CurrentPullRequest,
    calls: AtomicUsize,
    checks: Mutex<Vec<Value>>,
}
impl Transport for Arc<Api> {
    fn request(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!token.is_empty());
        let permissions =
            json!({"contents":"read","pull_requests":"read","checks":"write","metadata":"read"});
        let c = &self.comparison;
        match (method, path) {
            ("GET", "/app/installations/2") => {
                Ok(json!({"id":2,"app_id":1,"suspended_at":null,"permissions":permissions}))
            }
            ("POST", "/app/installations/2/access_tokens") => {
                assert_eq!(body.unwrap()["repository_ids"], json!([3]));
                let expiry=std::process::Command::new("python3").args(["-c","from datetime import datetime,timedelta,timezone; print((datetime.now(timezone.utc)+timedelta(hours=1)).strftime('%Y-%m-%dT%H:%M:%SZ'))"]).output().unwrap();
                Ok(
                    json!({"repositories":[{"id":3}],"permissions":permissions,"token":"fixture_installation_only","expires_at":String::from_utf8(expiry.stdout).unwrap().trim()}),
                )
            }
            ("GET", "/repos/fixture/app") => Ok(json!({"id":3,"private":false,"archived":false})),
            ("GET", "/repos/fixture/app/pulls/7") => Ok(
                json!({"number":7,"state":"open","draft":false,"base":{"sha":c.selected_base,"repo":{"id":3}},"head":{"sha":c.head,"repo":{"id":3,"private":false}}}),
            ),
            ("GET", _) if path.contains("/compare/") => Ok(
                json!({"base_commit":{"sha":c.selected_base},"merge_base_commit":{"sha":c.merge_base}}),
            ),
            ("GET", _) if path.contains("/check-runs?") => {
                let checks = self.checks.lock().unwrap();
                Ok(json!({"total_count":checks.len(),"check_runs":*checks}))
            }
            ("POST", "/repos/fixture/app/check-runs") => {
                let mut value = body.unwrap().clone();
                value["id"] = json!(99);
                value["app"] = json!({"id":1});
                self.checks.lock().unwrap().push(value.clone());
                Ok(value)
            }
            ("GET", _) => self
                .objects
                .get(path)
                .cloned()
                .ok_or(SetupError::Unavailable),
            _ => panic!("Unexpected fixture request"),
        }
    }
}
struct NativeWorker {
    root: PathBuf,
    result: Arc<Mutex<Option<PublicBundle>>>,
    revoke: AtomicBool,
}
impl AppPrWorker for NativeWorker {
    fn engine_digest(&self) -> &str {
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
    }
    fn analyze(&self, r: &PrWorkerRequest) -> Result<PublicBundle> {
        let bundle = analyze_pr_objects(r, &self.root.join("analysis"))?.into_bundle()?;
        if self.revoke.load(Ordering::SeqCst) {
            rusqlite::Connection::open(self.root.join("state.db"))
                .unwrap()
                .execute("UPDATE app_execution_grants SET enabled=0", [])
                .unwrap();
        }
        *self.result.lock().unwrap() = Some(bundle.clone());
        Ok(bundle)
    }
}
struct Readback {
    result: Arc<Mutex<Option<PublicBundle>>>,
    fail: AtomicBool,
}
impl AppReportReadback for Readback {
    fn readback(&self, receipt: &SignedAppReceipt) -> Result<PublicBundle> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(SetupError::Unavailable);
        }
        let result = self.result.lock().unwrap().clone().unwrap();
        assert_eq!(result.manifest.public_report_id, receipt.report_id());
        Ok(result)
    }
}
struct Harness {
    f: Fixture,
    dispatcher: AppDispatcher,
    api: Arc<Api>,
    readback: Arc<Readback>,
    worker: Arc<NativeWorker>,
}
impl Harness {
    fn new() -> Self {
        let f = Fixture::new();
        let queue = f.queue();
        let key = signer();
        let storage =
            || crate::Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap();
        let inbox = Arc::new(AppWebhookInbox::new(storage(), 1, vec![b'x'; 32]).unwrap());
        let body=serde_json::to_vec(&json!({"action":"opened","installation":{"id":2,"app_id":1},"repository":{"id":3},"number":7,"pull_request":{"number":7}})).unwrap();
        let hkey = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &[b'x'; 32]);
        let sig = ring::hmac::sign(&hkey, &body)
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        inbox
            .accept(
                "00000000-0000-0000-0000-000000000001",
                "pull_request",
                &format!("sha256={sig}"),
                &body,
            )
            .unwrap();
        let api = Arc::new(Api {
            objects: super::objects::tests::fixture_replies(&f),
            comparison: f.request.input.comparison.clone(),
            calls: AtomicUsize::new(0),
            checks: Mutex::new(vec![]),
        });
        let ephemeral = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .unwrap();
        assert!(ephemeral.status.success());
        let mut broker =
            GitHubInstallationBroker::new(1, "client123".into(), &ephemeral.stdout).unwrap();
        broker.transport = Box::new(api.clone());
        let result = Arc::new(Mutex::new(None));
        let worker = Arc::new(NativeWorker {
            root: f.root.clone(),
            result: result.clone(),
            revoke: AtomicBool::new(false),
        });
        let readback = Arc::new(Readback {
            result,
            fail: AtomicBool::new(false),
        });
        let reports = Arc::new(
            AppReportStore::new(
                f.root.join("reports"),
                storage(),
                queue.clone(),
                key.public_key().to_vec(),
                1,
            )
            .unwrap(),
        );
        let dispatcher = AppDispatcher::new(AppDispatchConfig {
            queue,
            inbox,
            broker: Arc::new(broker),
            worker: worker.clone(),
            reports,
            readback: readback.clone(),
            signer: key,
            report_origin: "https://reports.example.test".into(),
        })
        .unwrap();
        Self {
            f,
            dispatcher,
            api,
            readback,
            worker,
        }
    }
    fn sql(&self, s: &str) {
        rusqlite::Connection::open(self.f.root.join("state.db"))
            .unwrap()
            .execute(s, [])
            .unwrap();
    }
}
#[test]
fn notification_to_native_analysis_signed_report_and_check_requires_successful_readback() {
    let h = Harness::new();
    h.sql("UPDATE app_execution_grants SET enabled=0");
    assert_eq!(h.dispatcher.tick().unwrap(), DispatchOutcome::Inactive);
    assert_eq!(h.api.calls.load(Ordering::SeqCst), 0);
    h.sql("UPDATE app_execution_grants SET enabled=1");
    assert_eq!(h.dispatcher.tick().unwrap(), DispatchOutcome::Analyzed);
    assert!(h.api.checks.lock().unwrap().is_empty());
    h.readback.fail.store(true, Ordering::SeqCst);
    assert_eq!(h.dispatcher.tick().unwrap(), DispatchOutcome::Deferred);
    assert!(h.api.checks.lock().unwrap().is_empty());
    h.sql("UPDATE app_analysis_jobs SET available_at=0");
    h.readback.fail.store(false, Ordering::SeqCst);
    assert_eq!(h.dispatcher.tick().unwrap(), DispatchOutcome::Delivered);
    assert_eq!(h.dispatcher.tick().unwrap(), DispatchOutcome::Idle);
    let checks = h.api.checks.lock().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["head_sha"], h.f.request.input.comparison.head);
    assert_eq!(checks[0]["conclusion"], "neutral");
    let url = checks[0]["details_url"].as_str().unwrap();
    assert!(url.contains("mode=focus") && url.contains("design=hld") && url.contains("bundle="));
    assert!(
        checks[0]["output"]["summary"]
            .as_str()
            .unwrap()
            .contains(url)
    );
}
#[test]
fn consent_revoked_during_native_analysis_never_stages_or_delivers() {
    let h = Harness::new();
    h.worker.revoke.store(true, Ordering::SeqCst);
    assert!(h.dispatcher.tick().is_err());
    assert_eq!(h.dispatcher.tick().unwrap(), DispatchOutcome::Inactive);
    assert!(h.api.checks.lock().unwrap().is_empty());
    let db = rusqlite::Connection::open(h.f.root.join("state.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM app_report_intents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
