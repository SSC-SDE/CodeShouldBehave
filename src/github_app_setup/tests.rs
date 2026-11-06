use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

#[derive(Clone)]
struct FakeProvider {
    access: Arc<Mutex<AccessSnapshot>>,
    exchanges: Arc<AtomicUsize>,
    fail_exchange: bool,
    hold: Arc<Mutex<Option<Arc<Hold>>>>,
    inventory: Arc<Mutex<ScopeInventory>>,
    inventory_calls: Arc<AtomicUsize>,
    sources: Arc<Mutex<BTreeMap<String, String>>>,
    source_reads: Arc<Mutex<Vec<String>>>,
}
struct Hold {
    entered: std::sync::Barrier,
    resume: std::sync::Barrier,
}
impl GitHubSetupProvider for FakeProvider {
    fn exchange_code(&self, _: &str, callback: &str, verifier: &str) -> Result<UserCredential> {
        assert!(valid_secret(verifier));
        assert_eq!(
            callback,
            "https://setup.example.test/v1/github-app/callback"
        );
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        if self.fail_exchange {
            return Err(SetupError::Unavailable);
        }
        UserCredential::verified("synthetic_user_token_only".into(), 101, now()? + 7200)
    }
    fn scope_inventory(&self, _: &UserCredential, _: &RepositoryChoice) -> Result<ScopeInventory> {
        self.inventory_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.inventory.lock().unwrap().clone())
    }
    fn scoped_sources(
        &self,
        _: &UserCredential,
        _: &RepositoryChoice,
        entries: &[InventoryEntry],
    ) -> Result<Vec<PreviewFile>> {
        let sources = self.sources.lock().unwrap();
        entries
            .iter()
            .map(|f| {
                self.source_reads.lock().unwrap().push(f.path.clone());
                Ok(PreviewFile {
                    path: f.path.clone(),
                    oid: f.oid.clone(),
                    content: Some(sources.get(&f.path).ok_or(SetupError::Unavailable)?.clone()),
                })
            })
            .collect()
    }
    fn current_access(&self, _: &UserCredential) -> Result<AccessSnapshot> {
        let hold = self.hold.lock().unwrap().take();
        if let Some(hold) = hold {
            hold.entered.wait();
            hold.resume.wait();
        }
        Ok(self.access.lock().unwrap().clone())
    }
}
struct Fixture {
    root: std::path::PathBuf,
    host: Arc<AppSetupHost<FakeProvider>>,
    provider: FakeProvider,
}
impl Fixture {
    fn new() -> Self {
        Self::with_failure(false)
    }
    fn with_failure(fail_exchange: bool) -> Self {
        let root = std::env::temp_dir().join(format!("csb-app-setup-{}", secret().unwrap()));
        let provider = FakeProvider {
            access: Arc::new(Mutex::new(AccessSnapshot {
                user_id: 101,
                repositories: vec![repo()],
            })),
            exchanges: Arc::new(AtomicUsize::new(0)),
            fail_exchange,
            hold: Arc::new(Mutex::new(None)),
            inventory: Arc::new(Mutex::new(sample_inventory())),
            inventory_calls: Arc::new(AtomicUsize::new(0)),
            sources: Arc::new(Mutex::new(BTreeMap::new())),
            source_reads: Arc::new(Mutex::new(vec![])),
        };
        let host = Arc::new(AppSetupHost::new(
            Storage::open(&root.join("state.db"), &root.join("artifacts")).unwrap(),
            provider.clone(),
            config(),
        ));
        Self {
            root,
            host,
            provider,
        }
    }
    fn login(&self) -> String {
        let start = self.host.begin_login(Some(20)).unwrap();
        self.host
            .finish_login(state(&start), &start.browser_cookie, "code")
            .unwrap()
            .session_cookie
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn config() -> SetupConfig {
    SetupConfig::new(10, "client123".into(), "https://setup.example.test".into()).unwrap()
}
fn repo() -> RepositoryAccess {
    RepositoryAccess {
        app_id: 10,
        installation_id: 20,
        repository_id: 30,
        owner: "owner".into(),
        name: "repo".into(),
        public: true,
        suspended: false,
        administrator: true,
        contents_read: true,
        pull_requests_read: true,
        checks_write: true,
    }
}
fn state(start: &LoginStart) -> &str {
    start.authorize_url.split("&state=").nth(1).unwrap()
}
fn count(f: &Fixture) -> i64 {
    f.host
        .storage
        .lock()
        .unwrap()
        .connection
        .query_row("SELECT count(*) FROM app_setup_repositories", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn login_is_bound_to_browser_and_state_is_single_use() {
    let f = Fixture::new();
    let start = f.host.begin_login(Some(20)).unwrap();
    assert_eq!(
        f.host
            .finish_login(state(&start), &secret().unwrap(), "code")
            .err(),
        Some(SetupError::Authentication)
    );
    let session = f
        .host
        .finish_login(state(&start), &start.browser_cookie, "code")
        .unwrap();
    assert_eq!(
        f.host.repositories(&session.session_cookie).unwrap().len(),
        1
    );
    assert_eq!(
        f.host
            .finish_login(state(&start), &start.browser_cookie, "code")
            .err(),
        Some(SetupError::Authentication)
    );
    assert_eq!(f.provider.exchanges.load(Ordering::SeqCst), 1);
}
#[test]
fn expired_state_never_exchanges_and_exchange_failure_cannot_replay() {
    let f = Fixture::with_failure(true);
    let at = now().unwrap();
    let expired = f.host.begin_login_at(None, at - STATE_TTL).unwrap();
    assert_eq!(
        f.host
            .finish_login(state(&expired), &expired.browser_cookie, "code")
            .err(),
        Some(SetupError::Authentication)
    );
    assert_eq!(f.provider.exchanges.load(Ordering::SeqCst), 0);
    let start = f.host.begin_login(None).unwrap();
    assert_eq!(
        f.host
            .finish_login(state(&start), &start.browser_cookie, "code")
            .err(),
        Some(SetupError::Unavailable)
    );
    assert_eq!(
        f.host
            .finish_login(state(&start), &start.browser_cookie, "code")
            .err(),
        Some(SetupError::Authentication)
    );
}
#[test]
fn installation_hint_is_not_authority() {
    let f = Fixture::new();
    let start = f.host.begin_login(Some(999)).unwrap();
    assert_eq!(
        f.host
            .finish_login(state(&start), &start.browser_cookie, "code")
            .err(),
        Some(SetupError::Denied)
    );
    assert_eq!(count(&f), 0);
}
#[test]
fn selection_revalidates_removal_and_does_not_cross_installations() {
    let f = Fixture::new();
    let token = f.login();
    assert_eq!(
        f.host.select_repository(&token, 21, 30).err(),
        Some(SetupError::Denied)
    );
    f.provider.access.lock().unwrap().repositories.clear();
    assert_eq!(
        f.host.select_repository(&token, 20, 30).err(),
        Some(SetupError::Denied)
    );
    assert_eq!(count(&f), 0);
}
#[test]
fn all_permission_boundaries_are_enforced_on_current_access() {
    let f = Fixture::new();
    let token = f.login();
    for change in 0..7 {
        let mut r = repo();
        match change {
            0 => r.public = false,
            1 => r.administrator = false,
            2 => r.suspended = true,
            3 => r.app_id = 99,
            4 => r.contents_read = false,
            5 => r.pull_requests_read = false,
            _ => r.checks_write = false,
        };
        f.provider.access.lock().unwrap().repositories = vec![r];
        assert!(f.host.repositories(&token).unwrap().is_empty());
        assert_eq!(
            f.host.select_repository(&token, 20, 30).err(),
            Some(SetupError::Denied)
        );
    }
    assert_eq!(count(&f), 0);
}
#[test]
fn numeric_identity_is_bound_to_session_not_browser_or_login_name() {
    let f = Fixture::new();
    let token = f.login();
    f.provider.access.lock().unwrap().user_id = 102;
    assert_eq!(
        f.host.repositories(&token).err(),
        Some(SetupError::Authentication)
    );
    assert_eq!(
        f.host.select_repository(&token, 20, 30).err(),
        Some(SetupError::Authentication)
    );
}
#[test]
fn selection_is_idempotent_and_cannot_activate() {
    let f = Fixture::new();
    let token = f.login();
    f.host.select_repository(&token, 20, 30).unwrap();
    f.provider.access.lock().unwrap().repositories[0].name = "renamed".into();
    let selected = f.host.select_repository(&token, 20, 30).unwrap();
    assert_eq!(selected.name, "renamed");
    assert_eq!(selected.state, "installed_unconfigured");
    assert_eq!(count(&f), 1);
    let storage = f.host.storage.lock().unwrap();
    let row: (i64, String) = storage
        .connection
        .query_row(
            "SELECT selected_by_user_id,state FROM app_setup_repositories",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (101, "installed_unconfigured".into()));
    assert!(
        storage
            .connection
            .execute("UPDATE app_setup_repositories SET state='enabled'", [])
            .is_err()
    );
}
#[test]
fn sessions_expire_logout_and_restart_without_persisting_credentials() {
    let f = Fixture::new();
    let token = f.login();
    assert_eq!(
        f.host
            .repositories_at(&token, now().unwrap() + SESSION_TTL + 1)
            .err(),
        Some(SetupError::Authentication)
    );
    let token = f.login();
    f.host.logout(&token).unwrap();
    assert_eq!(
        f.host.repositories(&token).err(),
        Some(SetupError::Authentication)
    );
    let token = f.login();
    f.host.select_repository(&token, 20, 30).unwrap();
    let restarted = AppSetupHost::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        f.provider.clone(),
        config(),
    );
    assert_eq!(
        restarted.repositories(&token).err(),
        Some(SetupError::Authentication)
    );
    assert_eq!(count(&f), 1);
}
#[test]
fn oauth_state_storage_contains_only_digests() {
    let f = Fixture::new();
    let start = f.host.begin_login(None).unwrap();
    let stored: (String, String) = f
        .host
        .storage
        .lock()
        .unwrap()
        .connection
        .query_row(
            "SELECT state_digest,browser_digest FROM app_setup_oauth_states",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (digest(state(&start)), digest(&start.browser_cookie))
    );
    assert_ne!(stored.0, state(&start));
    assert_ne!(stored.1, start.browser_cookie);
}
#[test]
fn provider_incomplete_or_duplicate_inventory_fails_closed() {
    let f = Fixture::new();
    let token = f.login();
    f.provider.access.lock().unwrap().repositories.push(repo());
    assert_eq!(
        f.host.repositories(&token).err(),
        Some(SetupError::Unavailable)
    );
    f.provider.access.lock().unwrap().repositories = vec![repo(); MAX_REPOSITORIES + 1];
    assert_eq!(
        f.host.repositories(&token).err(),
        Some(SetupError::Capacity)
    );
}
#[test]
fn origin_configuration_rejects_credentials_paths_http_and_query() {
    for origin in [
        "http://setup.test",
        "https://user:pass@setup.test",
        "https://setup.test/path",
        "https://setup.test?x=y",
        "https://setup.test#x",
        "https://-setup.test",
    ] {
        assert!(SetupConfig::new(10, "client123".into(), origin.into()).is_err());
    }
}

async fn body(response: axum::response::Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}
fn request(
    method: &str,
    path: &str,
    token: &str,
    origin: Option<&str>,
    payload: &str,
) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("cookie", format!("__Host-csb-app-session={token}"));
    if let Some(o) = origin {
        req = req.header("origin", o);
    }
    req.body(Body::from(payload.to_owned())).unwrap()
}
#[tokio::test]
async fn disabled_routes_never_start_authorization() {
    let app = disabled_router();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/github-app/capability")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(value["available"], false);
    assert_eq!(value["activation"], "off");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/github-app/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get("location").is_none());
}
#[tokio::test]
async fn http_rejects_cross_origin_duplicate_cookie_and_browser_identity() {
    let f = Fixture::new();
    let token = f.login();
    let app = router(f.host.clone());
    for origin in [None, Some("https://attacker.test"), Some("null")] {
        let response = app
            .clone()
            .oneshot(request(
                "POST",
                "/v1/github-app/repositories",
                &token,
                origin,
                r#"{"installation_id":20,"repository_id":30}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/repositories",
            &token,
            Some("https://setup.example.test"),
            r#"{"installation_id":20,"repository_id":30,"user_id":101}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let mut req = request("GET", "/v1/github-app/repositories", &token, None, "");
    req.headers_mut().append(
        "cookie",
        format!("__Host-csb-app-session={token}").parse().unwrap(),
    );
    assert_eq!(
        app.oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(count(&f), 0);
}
#[tokio::test]
async fn http_login_callback_and_logout_keep_tokens_out_of_json() {
    let f = Fixture::new();
    let app = router(f.host.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/github-app/login?installation_id=20")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    let state = location.split("&state=").nth(1).unwrap();
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Secure; HttpOnly; SameSite=Lax"));
    let browser = cookie.split(';').next().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/github-app/callback?state={state}&code=code"))
                .header("cookie", browser)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        "/github-app/setup?installation_id=20"
    );
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    let session = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| v.to_str().unwrap().strip_prefix("__Host-csb-app-session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let payload = body(response).await;
    assert!(!payload.contains(&session));
    assert!(!payload.contains("synthetic_user_token_only"));
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/repositories",
            &session,
            Some("https://setup.example.test"),
            r#"{"installation_id":20,"repository_id":30}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body(response).await.contains("\"scan_started\":false"));
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/logout",
            &session,
            Some("https://setup.example.test"),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        app.oneshot(request(
            "GET",
            "/v1/github-app/repositories",
            &session,
            None,
            ""
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn pkce_matches_rfc7636_vector_and_never_places_verifier_in_redirect() {
    assert_eq!(
        pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let f = Fixture::new();
    let start = f.host.begin_login(None).unwrap();
    assert!(start.authorize_url.contains("code_challenge_method=S256"));
    assert!(
        start
            .authorize_url
            .contains(&pkce_challenge(&start.browser_cookie))
    );
    assert!(!start.authorize_url.contains(&start.browser_cookie));
}

#[test]
fn logout_during_provider_lookup_fences_selection_commit() {
    let f = Fixture::new();
    let token = f.login();
    let hold = Arc::new(Hold {
        entered: std::sync::Barrier::new(2),
        resume: std::sync::Barrier::new(2),
    });
    *f.provider.hold.lock().unwrap() = Some(hold.clone());
    let host = f.host.clone();
    let worker_token = token.clone();
    let thread = std::thread::spawn(move || host.select_repository(&worker_token, 20, 30));
    hold.entered.wait();
    f.host.logout(&token).unwrap();
    hold.resume.wait();
    assert_eq!(
        thread.join().unwrap().err(),
        Some(SetupError::Authentication)
    );
    assert_eq!(count(&f), 0);
}

fn sample_inventory() -> ScopeInventory {
    ScopeInventory {
        repository_id: 30,
        default_branch: "main".into(),
        commit: "a".repeat(40),
        tree: "b".repeat(40),
        entries: [
            ("src", "directory"),
            ("src/lib.rs", "file"),
            ("Cargo.toml", "file"),
            ("README.md", "file"),
            (".env", "file"),
            ("linked.rs", "symlink"),
            ("vendor", "submodule"),
        ]
        .into_iter()
        .map(|(path, kind)| InventoryEntry {
            path: path.into(),
            kind: kind.into(),
            oid: "c".repeat(40),
        })
        .collect(),
    }
}
fn input(revision: i64) -> ScopeInput {
    ScopeInput {
        installation_id: 20,
        repository_id: 30,
        commit: "a".repeat(40),
        expected_revision: revision,
        include: vec![".".into()],
        exclude: vec![],
    }
}
fn selected(f: &Fixture) -> String {
    let token = f.login();
    f.host.select_repository(&token, 20, 30).unwrap();
    token
}
fn saved_count(f: &Fixture) -> i64 {
    f.host
        .storage
        .lock()
        .unwrap()
        .connection
        .query_row("SELECT count(*) FROM app_setup_scopes", [], |r| r.get(0))
        .unwrap()
}
#[test]
fn scope_saves_exact_metadata_boundary_without_activation_and_survives_restart() {
    let f = Fixture::new();
    let token = selected(&f);
    assert_eq!(f.provider.inventory_calls.load(Ordering::SeqCst), 0);
    let preview = f.host.preview_scope(&token, input(0)).unwrap();
    assert_eq!(
        preview.document.files,
        vec!["Cargo.toml", "README.md", "src/lib.rs"]
    );
    assert_eq!(preview.document.rust_candidates, 1);
    assert_eq!(preview.document.unsupported_files, 1);
    assert_eq!(preview.document.automatically_excluded, 3);
    assert_eq!(preview.activation, "off");
    let saved = f
        .host
        .confirm_scope(&token, 20, 30, &preview.preview_id, 0, true)
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.document, preview.document);
    assert!(!saved.document.source_scanned);
    assert!(!saved.document.architecture_confirmed);
    assert!(!saved.document.publication_consent);
    assert_eq!(
        f.host
            .saved_scope(&token, 20, 30)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    let restarted = AppSetupHost::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        f.provider.clone(),
        config(),
    );
    let start = restarted.begin_login(None).unwrap();
    let session = restarted
        .finish_login(state(&start), &start.browser_cookie, "code")
        .unwrap();
    assert_eq!(
        restarted
            .saved_scope(&session.session_cookie, 20, 30)
            .unwrap()
            .unwrap()
            .document,
        preview.document
    );
    let storage = f.host.storage.lock().unwrap();
    assert!(
        storage
            .connection
            .execute("UPDATE app_setup_scopes SET activation='on'", [])
            .is_err()
    );
    let repo_state: String = storage
        .connection
        .query_row("SELECT state FROM app_setup_repositories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(repo_state, "installed_unconfigured");
}
#[test]
fn preview_requires_known_paths_and_nonempty_allowed_files() {
    let f = Fixture::new();
    let token = selected(&f);
    for root in [
        "../secret",
        "src/./lib.rs",
        "/src",
        ".git/config",
        "missing",
        ".env",
        "linked.rs",
        "vendor",
    ] {
        let mut q = input(0);
        q.include = vec![root.into()];
        assert_eq!(
            f.host.preview_scope(&token, q).err(),
            Some(SetupError::Invalid),
            "{root}"
        );
    }
    let mut q = input(0);
    q.exclude = vec![".".into()];
    assert_eq!(
        f.host.preview_scope(&token, q).err(),
        Some(SetupError::Invalid)
    );
    let mut q = input(0);
    q.exclude = vec!["src".into()];
    let p = f.host.preview_scope(&token, q).unwrap();
    assert!(!p.document.files.iter().any(|p| p.starts_with("src/")));
    assert_eq!(saved_count(&f), 0);
}
#[test]
fn stale_commit_tree_preview_and_replayed_confirmation_are_rejected() {
    let f = Fixture::new();
    let token = selected(&f);
    let old = f.host.preview_scope(&token, input(0)).unwrap();
    f.provider.inventory.lock().unwrap().commit = "d".repeat(40);
    assert_eq!(
        f.host
            .confirm_scope(&token, 20, 30, &old.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(
        f.host.preview_scope(&token, input(0)).err(),
        Some(SetupError::Conflict)
    );
    f.provider.inventory.lock().unwrap().commit = "a".repeat(40);
    let fresh = f.host.preview_scope(&token, input(0)).unwrap();
    assert_eq!(
        f.host
            .confirm_scope(&token, 20, 30, &old.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(
        f.host
            .confirm_scope(&token, 20, 30, &fresh.preview_id, 0, false)
            .err(),
        Some(SetupError::Invalid)
    );
    f.host
        .confirm_scope(&token, 20, 30, &fresh.preview_id, 0, true)
        .unwrap();
    assert_eq!(
        f.host
            .confirm_scope(&token, 20, 30, &fresh.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(
        f.host.preview_scope(&token, input(0)).err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(saved_count(&f), 1);
}
#[test]
fn permission_revocation_blocks_preview_confirmation_and_saved_scope_reads() {
    let f = Fixture::new();
    let token = selected(&f);
    let p = f.host.preview_scope(&token, input(0)).unwrap();
    f.provider.access.lock().unwrap().repositories.clear();
    assert_eq!(
        f.host
            .confirm_scope(&token, 20, 30, &p.preview_id, 0, true)
            .err(),
        Some(SetupError::Denied)
    );
    assert_eq!(
        f.host.saved_scope(&token, 20, 30).err(),
        Some(SetupError::Denied)
    );
    assert_eq!(
        f.host.scope_inventory(&token, 20, 30).err(),
        Some(SetupError::Denied)
    );
    assert_eq!(saved_count(&f), 0);
}
#[test]
fn previews_cannot_cross_repository_user_or_expiry_boundary() {
    let f = Fixture::new();
    let token = selected(&f);
    let p = f.host.preview_scope(&token, input(0)).unwrap();
    assert_eq!(
        f.host
            .confirm_scope(&token, 21, 30, &p.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    let other = secret().unwrap();
    f.host.sessions.lock().unwrap().insert(
        digest(&other),
        Session {
            credential: UserCredential::verified(
                "synthetic_second_user".into(),
                102,
                now().unwrap() + 3600,
            )
            .unwrap(),
            expires_at_s: now().unwrap() + 3600,
        },
    );
    assert_eq!(
        f.host
            .confirm_scope(&other, 20, 30, &p.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    f.host
        .storage
        .lock()
        .unwrap()
        .connection
        .execute("UPDATE app_setup_scope_previews SET expires_at_s=0", [])
        .unwrap();
    assert_eq!(
        f.host
            .confirm_scope(&token, 20, 30, &p.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(saved_count(&f), 0);
}
#[test]
fn concurrent_admin_confirmation_requires_reload_instead_of_overwrite() {
    let f = Fixture::new();
    let a = selected(&f);
    let p = f.host.preview_scope(&a, input(0)).unwrap();
    let b = secret().unwrap();
    f.host.sessions.lock().unwrap().insert(
        digest(&b),
        Session {
            credential: UserCredential::verified(
                "synthetic_second_user".into(),
                102,
                now().unwrap() + 3600,
            )
            .unwrap(),
            expires_at_s: now().unwrap() + 3600,
        },
    );
    f.provider.access.lock().unwrap().user_id = 102;
    let second = f.host.preview_scope(&b, input(0)).unwrap();
    f.provider.access.lock().unwrap().user_id = 101;
    f.host
        .confirm_scope(&a, 20, 30, &p.preview_id, 0, true)
        .unwrap();
    f.provider.access.lock().unwrap().user_id = 102;
    assert_eq!(
        f.host
            .confirm_scope(&b, 20, 30, &second.preview_id, 0, true)
            .err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(saved_count(&f), 1);
}
#[test]
fn incomplete_or_duplicate_metadata_cannot_form_a_scope() {
    let f = Fixture::new();
    let token = selected(&f);
    let duplicate = f.provider.inventory.lock().unwrap().entries[0].clone();
    f.provider.inventory.lock().unwrap().entries.push(duplicate);
    assert_eq!(
        f.host.scope_inventory(&token, 20, 30).err(),
        Some(SetupError::Unavailable)
    );
    f.provider.inventory.lock().unwrap().entries = vec![
        InventoryEntry {
            path: "src".into(),
            kind: "directory".into(),
            oid: "b".repeat(40)
        };
        2001
    ];
    assert_eq!(
        f.host.scope_inventory(&token, 20, 30).err(),
        Some(SetupError::Capacity)
    );
}
#[tokio::test]
async fn setup_page_and_scope_http_journey_preserve_off_boundary() {
    let f = Fixture::new();
    let token = selected(&f);
    let app = router(f.host.clone());
    let page = app
        .clone()
        .oneshot(request("GET", "/github-app/setup", &token, None, ""))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert!(
        page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("script-src 'self'")
    );
    let text = body(page).await;
    assert!(text.contains("Confirm scope · keep checks off"));
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/inventory",
            &token,
            Some("https://setup.example.test"),
            r#"{"installation_id":20,"repository_id":30}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(value["source_scanned"], false);
    assert_eq!(value["activation"], "off");
    let payload=serde_json::json!({"installation_id":20,"repository_id":30,"commit":"a".repeat(40),"expected_revision":0,"include":["src"],"exclude":[]}).to_string();
    let denied = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/scope/preview",
            &token,
            Some("https://evil.test"),
            &payload,
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/scope/preview",
            &token,
            Some("https://setup.example.test"),
            &payload,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    let payload = serde_json::json!({"installation_id":20,"repository_id":30,"preview_id":preview["preview_id"],"expected_revision":0,"acknowledge_limits":true});
    let mut tampered = payload.clone();
    tampered["activate"] = serde_json::json!(true);
    let rejected = app
        .clone()
        .oneshot(request(
            "POST",
            "/v1/github-app/scope/confirm",
            &token,
            Some("https://setup.example.test"),
            &tampered.to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let saved = app
        .oneshot(request(
            "POST",
            "/v1/github-app/scope/confirm",
            &token,
            Some("https://setup.example.test"),
            &payload.to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    assert!(body(saved).await.contains("\"activation\":\"off\""));
}

struct FixtureScanWorker;
impl AppScanWorker for FixtureScanWorker {
    fn preview(&self, request: &ScanRequest) -> Result<ScanResult> {
        analyze_preview(
            request,
            &std::env::temp_dir().join(format!("csb-fixture-worker-{}", secret()?)),
        )
    }
}
fn git_blob(source: &str) -> String {
    use std::io::Write;
    let mut p = std::process::Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    String::from_utf8(p.wait_with_output().unwrap().stdout)
        .unwrap()
        .trim()
        .into()
}
fn scanning_fixture() -> (Fixture, String) {
    let mut f = Fixture::new();
    Arc::get_mut(&mut f.host).unwrap().scan_worker = Some(Arc::new(FixtureScanWorker));
    let contents: BTreeMap<String, String> = BTreeMap::from([
        (
            "Cargo.toml".into(),
            "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n".into(),
        ),
        (
            "src/lib.rs".into(),
            "// SOURCE_CONTENT_SENTINEL\nuse axum::Router;\npub fn service() {}\n".into(),
        ),
    ]);
    for entry in &mut f.provider.inventory.lock().unwrap().entries {
        if let Some(source) = contents.get(&entry.path) {
            entry.oid = git_blob(source);
        }
    }
    *f.provider.sources.lock().unwrap() = contents;
    let token = selected(&f);
    let preview = f.host.preview_scope(&token, input(0)).unwrap();
    f.host
        .confirm_scope(&token, 20, 30, &preview.preview_id, 0, true)
        .unwrap();
    (f, token)
}
fn scan_input() -> ScanInput {
    ScanInput {
        installation_id: 20,
        repository_id: 30,
        scope_revision: 1,
        allow_selected_source_scan: true,
    }
}
fn architecture_input(preview: &ArchitecturePreview) -> ArchitectureConfirmation {
    ArchitectureConfirmation {
        installation_id: 20,
        repository_id: 30,
        scope_revision: 1,
        preview_id: preview.preview_id.clone(),
        components: preview
            .result
            .components
            .iter()
            .map(|c| ArchitectureDecision {
                id: c.id.clone(),
                name: c.name.clone(),
                roots: c.roots.clone(),
                decision: crate::ComponentDecision::Confirmed,
            })
            .collect(),
        accept_known_gaps: true,
        defer_integrations: true,
    }
}
#[test]
fn source_preview_uses_existing_detectors_and_accepts_inactive_context_without_source_storage() {
    let (f, token) = scanning_fixture();
    let preview = f.host.preview_architecture(&token, scan_input()).unwrap();
    assert_eq!(preview.result.components.len(), 1);
    assert!(
        preview.result.components[0]
            .stack_candidates
            .iter()
            .any(|s| s.contains("Axum"))
    );
    assert_eq!(
        *f.provider.source_reads.lock().unwrap(),
        vec!["Cargo.toml", "src/lib.rs"]
    );
    let encoded: String = f
        .host
        .storage
        .lock()
        .unwrap()
        .connection
        .query_row("SELECT document FROM app_architecture_previews", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!encoded.contains("SOURCE_CONTENT_SENTINEL"));
    let result = f
        .host
        .confirm_architecture(&token, architecture_input(&preview))
        .unwrap();
    assert_eq!(result["activation"], "off");
    assert_eq!(result["publication_consent"], false);
    assert_eq!(result["context"]["usable"], true);
    assert_eq!(
        result["context"]["confirmation"]["actor"],
        "github-user-101"
    );
    assert!(f.host.architecture_status(&token, 20, 30).unwrap()["architecture_confirmed"] == true);
    assert_eq!(
        f.host
            .confirm_architecture(&token, architecture_input(&preview))
            .err(),
        Some(SetupError::Conflict)
    );
    let dbh = f.host.storage.lock().unwrap();
    let (off, no_export): (String, i64) = dbh
        .connection
        .query_row(
            "SELECT activation,publication_consent FROM app_architecture_acceptances",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((off.as_str(), no_export), ("off", 0));
}
#[test]
fn scans_require_explicit_consent_current_scope_access_and_configured_worker() {
    let (mut f, token) = scanning_fixture();
    let mut request = scan_input();
    request.allow_selected_source_scan = false;
    assert_eq!(
        f.host.preview_architecture(&token, request).err(),
        Some(SetupError::Invalid)
    );
    let mut request = scan_input();
    request.scope_revision = 99;
    assert_eq!(
        f.host.preview_architecture(&token, request).err(),
        Some(SetupError::Conflict)
    );
    f.provider.access.lock().unwrap().repositories[0].administrator = false;
    assert_eq!(
        f.host.preview_architecture(&token, scan_input()).err(),
        Some(SetupError::Denied)
    );
    f.provider.access.lock().unwrap().repositories[0].administrator = true;
    Arc::get_mut(&mut f.host).unwrap().scan_worker = None;
    assert_eq!(
        f.host.preview_architecture(&token, scan_input()).err(),
        Some(SetupError::Unavailable)
    );
    assert!(f.provider.source_reads.lock().unwrap().is_empty());
}
#[test]
fn architecture_rejects_outside_roots_missing_gap_choices_and_stale_or_foreign_previews() {
    let (f, token) = scanning_fixture();
    let preview = f.host.preview_architecture(&token, scan_input()).unwrap();
    let mut invalid = architecture_input(&preview);
    invalid.components[0].roots = vec!["absent".into()];
    invalid.components[0].decision = crate::ComponentDecision::Corrected;
    assert_eq!(
        f.host.confirm_architecture(&token, invalid).err(),
        Some(SetupError::Invalid)
    );
    let mut invalid = architecture_input(&preview);
    invalid.accept_known_gaps = false;
    assert_eq!(
        f.host.confirm_architecture(&token, invalid).err(),
        Some(SetupError::Invalid)
    );
    let mut invalid = architecture_input(&preview);
    invalid.defer_integrations = false;
    assert_eq!(
        f.host.confirm_architecture(&token, invalid).err(),
        Some(SetupError::Invalid)
    );
    let mut invalid = architecture_input(&preview);
    invalid.repository_id = 999;
    assert_eq!(
        f.host.confirm_architecture(&token, invalid).err(),
        Some(SetupError::Conflict)
    );
    f.provider.inventory.lock().unwrap().commit = "d".repeat(40);
    assert_eq!(
        f.host
            .confirm_architecture(&token, architecture_input(&preview))
            .err(),
        Some(SetupError::Conflict)
    );
}
#[test]
fn preview_hash_mismatch_cannot_publish_architecture() {
    let (f, token) = scanning_fixture();
    f.provider
        .sources
        .lock()
        .unwrap()
        .insert("src/lib.rs".into(), "pub fn changed(){}".into());
    assert_eq!(
        f.host.preview_architecture(&token, scan_input()).err(),
        Some(SetupError::Invalid)
    );
    let count: i64 = f
        .host
        .storage
        .lock()
        .unwrap()
        .connection
        .query_row("SELECT count(*) FROM app_architecture_previews", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn architecture_confirm_rechecks_revocation_replacement_scope_and_expiry() {
    let (f, token) = scanning_fixture();
    let first = f.host.preview_architecture(&token, scan_input()).unwrap();
    let second = f.host.preview_architecture(&token, scan_input()).unwrap();
    assert_eq!(
        f.host
            .confirm_architecture(&token, architecture_input(&first))
            .err(),
        Some(SetupError::Conflict)
    );
    f.provider.access.lock().unwrap().repositories[0].public = false;
    assert_eq!(
        f.host
            .confirm_architecture(&token, architecture_input(&second))
            .err(),
        Some(SetupError::Denied)
    );
    f.provider.access.lock().unwrap().repositories[0].public = true;
    f.host
        .storage
        .lock()
        .unwrap()
        .connection
        .execute("UPDATE app_architecture_previews SET expires_at_s=0", [])
        .unwrap();
    assert_eq!(
        f.host
            .confirm_architecture(&token, architecture_input(&second))
            .err(),
        Some(SetupError::Conflict)
    );
    let third = f.host.preview_architecture(&token, scan_input()).unwrap();
    let scope = f.host.preview_scope(&token, input(1)).unwrap();
    f.host
        .confirm_scope(&token, 20, 30, &scope.preview_id, 1, true)
        .unwrap();
    assert_eq!(
        f.host
            .confirm_architecture(&token, architecture_input(&third))
            .err(),
        Some(SetupError::Conflict)
    );
}

#[tokio::test]
async fn architecture_routes_require_origin_and_consent_and_return_inactive_status() {
    let (f, token) = scanning_fixture();
    let app = router(f.host.clone());
    let send = |path: &str, value: serde_json::Value, origin: &str| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("origin", origin)
            .header("cookie", format!("__Host-csb-app-session={token}"))
            .body(Body::from(value.to_string()))
            .unwrap()
    };
    let body = serde_json::json!({"installation_id":20,"repository_id":30,"scope_revision":1,"allow_selected_source_scan":true});
    let denied = app
        .clone()
        .oneshot(send(
            "/v1/github-app/architecture/preview",
            body.clone(),
            "https://wrong.test",
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert!(f.provider.source_reads.lock().unwrap().is_empty());
    let response = app
        .clone()
        .oneshot(send(
            "/v1/github-app/architecture/preview",
            body,
            "https://setup.example.test",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let components=value["result"]["components"].as_array().unwrap().iter().map(|c|serde_json::json!({"id":c["id"],"name":c["name"],"roots":c["roots"],"decision":"confirmed"})).collect::<Vec<_>>();
    let mut confirm = serde_json::json!({"installation_id":20,"repository_id":30,"scope_revision":1,"preview_id":value["preview_id"],"components":components,"accept_known_gaps":true,"defer_integrations":true,"activate":true});
    assert_eq!(
        app.clone()
            .oneshot(send(
                "/v1/github-app/architecture/confirm",
                confirm.clone(),
                "https://setup.example.test"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    confirm.as_object_mut().unwrap().remove("activate");
    let result = app
        .oneshot(send(
            "/v1/github-app/architecture/confirm",
            confirm,
            "https://setup.example.test",
        ))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let output: serde_json::Value =
        serde_json::from_slice(&result.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(output["activation"], "off");
    assert_eq!(output["publication_consent"], false);
}

fn inbox(f: &Fixture) -> AppWebhookInbox {
    AppWebhookInbox::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        10,
        vec![9; 32],
    )
    .unwrap()
}
fn signed(value: &serde_json::Value) -> (Vec<u8>, String) {
    let body = serde_json::to_vec(value).unwrap();
    let signature = format!(
        "sha256={}",
        crate::provider::hmac_sha256(&[9; 32], &body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    (body, signature)
}
fn webhook_pr() -> serde_json::Value {
    serde_json::json!({"action":"opened","installation":{"id":20},"repository":{"id":30},"number":24,"pull_request":{"number":24,"body":"DO_NOT_STORE_PR_BODY","head":{"sha":"a".repeat(40)}}})
}
#[test]
fn signed_inbox_is_idempotent_restartable_and_does_not_create_analysis() {
    let f = Fixture::new();
    selected(&f);
    let queue = inbox(&f);
    let (body, signature) = signed(&webhook_pr());
    let id = "00000000-0000-0000-0000-000000000001";
    assert_eq!(
        queue.accept(id, "pull_request", "sha256=00", &body).err(),
        Some(SetupError::Authentication)
    );
    assert!(
        !queue
            .accept(id, "pull_request", &signature, &body)
            .unwrap()
            .duplicate
    );
    assert!(
        queue
            .accept(id, "pull_request", &signature, &body)
            .unwrap()
            .duplicate
    );
    let mut altered = webhook_pr();
    altered["action"] = serde_json::json!("closed");
    let (bad, sig) = signed(&altered);
    assert_eq!(
        queue.accept(id, "pull_request", &sig, &bad).err(),
        Some(SetupError::Conflict)
    );
    drop(queue);
    let queue = inbox(&f);
    let notification = queue.claim().unwrap().unwrap();
    assert_eq!(notification.pr_number, Some(24));
    assert!(queue.claim().unwrap().is_none());
    queue.finish(&notification).unwrap();
    assert_eq!(
        queue.finish(&notification).err(),
        Some(SetupError::Conflict)
    );
    let dbh = f.host.storage.lock().unwrap();
    let count: i64 = dbh
        .connection
        .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let digest: String = dbh
        .connection
        .query_row("SELECT body_digest FROM app_webhook_inbox", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(digest.len(), 64);
    assert!(f.provider.source_reads.lock().unwrap().is_empty());
}
#[test]
fn uninstall_fences_leases_source_and_cached_context_even_if_provider_is_delayed() {
    let (f, token) = scanning_fixture();
    let queue = inbox(&f);
    let (body, signature) = signed(&webhook_pr());
    queue
        .accept(
            "00000000-0000-0000-0000-000000000001",
            "pull_request",
            &signature,
            &body,
        )
        .unwrap();
    let lease = queue.claim().unwrap().unwrap();
    let (body, signature) =
        signed(&serde_json::json!({"action":"deleted","installation":{"id":20,"app_id":10}}));
    queue
        .accept(
            "00000000-0000-0000-0000-000000000002",
            "installation",
            &signature,
            &body,
        )
        .unwrap();
    assert_eq!(queue.finish(&lease).err(), Some(SetupError::Conflict));
    assert!(queue.claim().unwrap().is_none());
    assert!(f.host.repositories(&token).unwrap().is_empty());
    assert_eq!(
        f.host.preview_architecture(&token, scan_input()).err(),
        Some(SetupError::Denied)
    );
    assert_eq!(
        f.host.architecture_status(&token, 20, 30).err(),
        Some(SetupError::Denied)
    );
    assert!(f.provider.source_reads.lock().unwrap().is_empty());
    let (body, signature) =
        signed(&serde_json::json!({"action":"unsuspend","installation":{"id":20,"app_id":10}}));
    queue
        .accept(
            "00000000-0000-0000-0000-000000000003",
            "installation",
            &signature,
            &body,
        )
        .unwrap();
    assert!(f.host.repositories(&token).unwrap().is_empty());
}
#[test]
fn removal_is_repository_scoped_and_expired_notification_leases_are_fenced() {
    let f = Fixture::new();
    let token = selected(&f);
    let queue = inbox(&f);
    let (body, signature) = signed(&webhook_pr());
    queue
        .accept(
            "00000000-0000-0000-0000-000000000001",
            "pull_request",
            &signature,
            &body,
        )
        .unwrap();
    let first = queue.claim().unwrap().unwrap();
    f.host
        .storage
        .lock()
        .unwrap()
        .connection
        .execute("UPDATE app_webhook_inbox SET lease_expires_at_s=0", [])
        .unwrap();
    let replacement = queue.claim().unwrap().unwrap();
    assert_ne!(first.lease_id, replacement.lease_id);
    assert_eq!(queue.finish(&first).err(), Some(SetupError::Conflict));
    let (body, signature) = signed(
        &serde_json::json!({"action":"removed","installation":{"id":20},"repositories_removed":[{"id":30}]}),
    );
    queue
        .accept(
            "00000000-0000-0000-0000-000000000002",
            "installation_repositories",
            &signature,
            &body,
        )
        .unwrap();
    assert_eq!(queue.finish(&replacement).err(), Some(SetupError::Conflict));
    assert_eq!(
        f.host.select_repository(&token, 20, 30).err(),
        Some(SetupError::Denied)
    );
}
#[test]
fn foreign_app_unknown_installation_and_wrong_check_owner_are_rejected() {
    let f = Fixture::new();
    selected(&f);
    let queue = inbox(&f);
    for payload in [
        serde_json::json!({"action":"created","installation":{"id":20,"app_id":99}}),
        serde_json::json!({"action":"created","installation":{"id":99,"app_id":10}}),
    ] {
        let (body, signature) = signed(&payload);
        assert_eq!(
            queue
                .accept(
                    "00000000-0000-0000-0000-000000000001",
                    "installation",
                    &signature,
                    &body
                )
                .err(),
            Some(SetupError::Denied)
        );
    }
    let (body, signature) = signed(
        &serde_json::json!({"action":"rerequested","installation":{"id":20},"repository":{"id":30},"check_run":{"app":{"id":99}}}),
    );
    assert_eq!(
        queue
            .accept(
                "00000000-0000-0000-0000-000000000001",
                "check_run",
                &signature,
                &body
            )
            .err(),
        Some(SetupError::Denied)
    );
}

#[test]
fn inbox_capacity_does_not_prevent_durable_uninstall_block() {
    let f = Fixture::new();
    let token = selected(&f);
    let queue = inbox(&f);
    f.host.storage.lock().unwrap().connection.execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO app_webhook_inbox(app_id,delivery_id,event,body_digest,installation_id,action,received_at_s) SELECT 10,printf('fixture-%d',i),'pull_request',printf('%064d',i),20,'opened',0 FROM n",[]).unwrap();
    let (body, signature) =
        signed(&serde_json::json!({"action":"deleted","installation":{"id":20,"app_id":10}}));
    assert_eq!(
        queue
            .accept(
                "00000000-0000-0000-0000-000000000001",
                "installation",
                &signature,
                &body
            )
            .err(),
        Some(SetupError::Capacity)
    );
    assert!(f.host.repositories(&token).unwrap().is_empty());
    assert!(queue.claim().unwrap().is_none());
}
#[tokio::test]
async fn webhook_http_validates_raw_signature_and_duplicate_headers() {
    let f = Fixture::new();
    selected(&f);
    let app = webhook_router(Arc::new(inbox(&f)));
    let (body, signature) = signed(&webhook_pr());
    let request = |duplicate: bool| {
        let mut b = Request::builder()
            .method("POST")
            .uri("/v1/github-app/webhook")
            .header("x-github-delivery", "00000000-0000-0000-0000-000000000001")
            .header("x-github-event", "pull_request")
            .header("x-hub-signature-256", &signature);
        if duplicate {
            b = b.header("x-github-event", "installation");
        }
        b.body(Body::from(body.clone())).unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request(true)).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.oneshot(request(false)).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
}
