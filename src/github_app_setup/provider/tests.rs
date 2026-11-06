use super::*;
use serde_json::json;
use std::collections::VecDeque;
struct Stub {
    responses: Mutex<VecDeque<(String, Result<Value>)>>,
}
impl Transport for Stub {
    fn request(&self, r: ProviderRequest<'_>, _: Duration) -> Result<Value> {
        let (expected, response) = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider request");
        assert_eq!(r.path, expected);
        if r.path == EXCHANGE {
            assert!(r.token.is_none());
            let form = r.form.unwrap();
            assert!(
                form.iter()
                    .any(|(k, v)| *k == "code_verifier" && valid_secret(v))
            );
            assert!(
                form.iter().any(|(k, v)| *k == "redirect_uri"
                    && *v == "https://setup.test/v1/github-app/callback")
            );
        } else {
            assert_eq!(r.token, Some("ghu_synthetic_token_only"));
            assert!(r.form.is_none());
        }
        response
    }
}
fn provider(responses: Vec<(&str, Result<Value>)>) -> GitHubWebProvider {
    let mut p = GitHubWebProvider::new(
        SetupConfig::new(10, "client123".into(), "https://setup.test".into()).unwrap(),
        "synthetic_secret_only".into(),
    )
    .unwrap();
    p.transport = Arc::new(Stub {
        responses: Mutex::new(responses.into_iter().map(|(p, v)| (p.into(), v)).collect()),
    });
    p
}
fn credential() -> UserCredential {
    UserCredential::verified(
        "ghu_synthetic_token_only".into(),
        101,
        now().unwrap() + 3600,
    )
    .unwrap()
}
fn installation() -> Value {
    json!({"id":20,"app_id":10,"suspended_at":null,"permissions":{"metadata":"read","contents":"read","pull_requests":"read","checks":"write"}})
}
fn repo() -> Value {
    json!({"id":30,"name":"repo","owner":{"login":"owner"},"private":false,"visibility":"public","permissions":{"admin":true}})
}
#[test]
fn web_exchange_uses_pkce_and_current_numeric_user() {
    let p = provider(vec![
        (
            EXCHANGE,
            Ok(
                json!({"access_token":"ghu_synthetic_token_only","token_type":"bearer","expires_in":28800,"refresh_token":"ignored"}),
            ),
        ),
        ("/user", Ok(json!({"id":101}))),
    ]);
    let c = p
        .exchange_code("code", &p.config.callback_url(), &secret().unwrap())
        .unwrap();
    assert_eq!(c.user_id, 101);
    assert_eq!(c.token(), "ghu_synthetic_token_only");
    assert!(c.expires_at_s > now().unwrap());
}
#[test]
fn exchange_errors_and_wrong_token_types_do_not_authenticate() {
    for value in [
        json!({"error":"bad_verification_code"}),
        json!({"access_token":"synthetic_pat_only","token_type":"bearer"}),
        json!({"access_token":"ghu_synthetic_token_only","token_type":"bearer","expires_in":0}),
    ] {
        let p = provider(vec![(EXCHANGE, Ok(value))]);
        assert_eq!(
            p.exchange_code("code", &p.config.callback_url(), &secret().unwrap())
                .err(),
            Some(SetupError::Authentication)
        );
    }
}
#[test]
fn installed_repository_intersection_requires_current_admin() {
    let mut nonadmin = repo();
    nonadmin["id"] = json!(31);
    nonadmin["permissions"]["admin"] = json!(false);
    let p = provider(vec![
        ("/user", Ok(json!({"id":101}))),
        (
            "/user/installations?per_page=100&page=1",
            Ok(json!({"total_count":1,"installations":[installation()]})),
        ),
        (
            "/user/installations/20/repositories?per_page=100&page=1",
            Ok(json!({"total_count":2,"repositories":[repo(),nonadmin]})),
        ),
    ]);
    let access = p.current_access(&credential()).unwrap();
    assert_eq!(access.user_id, 101);
    assert_eq!(access.repositories.len(), 1);
    assert_eq!(access.repositories[0].repository_id, 30);
}
#[test]
fn suspension_wrong_app_and_missing_checks_prevent_repository_requests() {
    for variant in 0..3 {
        let mut entry = installation();
        match variant {
            0 => entry["suspended_at"] = json!("2026-11-06"),
            1 => entry["app_id"] = json!(99),
            _ => entry["permissions"]["checks"] = json!("read"),
        };
        let p = provider(vec![
            ("/user", Ok(json!({"id":101}))),
            (
                "/user/installations?per_page=100&page=1",
                Ok(json!({"total_count":1,"installations":[entry]})),
            ),
        ]);
        assert!(
            p.current_access(&credential())
                .unwrap()
                .repositories
                .is_empty()
        );
    }
}
#[test]
fn changed_totals_duplicate_rows_and_truncated_pages_fail_closed() {
    for second in [
        json!({"total_count":3,"installations":[installation()]}),
        json!({"total_count":2,"installations":[installation()]}),
        json!({"total_count":2,"installations":[]}),
    ] {
        let stub = Stub {
            responses: Mutex::new(VecDeque::from([
                (
                    "/list?per_page=100&page=1".into(),
                    Ok(json!({"total_count":2,"installations":[installation()]})),
                ),
                ("/list?per_page=100&page=2".into(), Ok(second)),
            ])),
        };
        assert_eq!(
            Pass::new(&stub)
                .pages("/list", "installations", "ghu_synthetic_token_only", 100)
                .err(),
            Some(SetupError::Unavailable)
        );
    }
}
#[test]
fn provider_request_and_parallel_operation_caps_are_hard_limits() {
    let p = provider(vec![]);
    let _a = p.permit().unwrap();
    let _b = p.permit().unwrap();
    let _c = p.permit().unwrap();
    let d = p.permit().unwrap();
    assert_eq!(p.permit().err(), Some(SetupError::Capacity));
    drop(d);
    assert!(p.permit().is_ok());
    let mut pass = Pass::new(p.transport.as_ref());
    pass.requests = MAX_REQUESTS;
    assert_eq!(
        pass.get("/user", "ghu_synthetic_token_only").err(),
        Some(SetupError::Capacity)
    );
}
#[test]
fn network_client_refuses_redirects_and_ambient_proxy() {
    let https = Https::new();
    let config = https.agent.config();
    assert!(config.https_only());
    assert_eq!(config.max_redirects(), 0);
    assert!(config.max_redirects_will_error());
    assert!(config.proxy().is_none());
    assert_eq!(config.max_response_header_size(), 16 * 1024);
}
#[test]
fn transport_rejects_arbitrary_endpoints_before_network() {
    let transport = Https::new();
    for path in [
        "https://attacker.test/token",
        "//attacker.test",
        "/repos/owner/repo/contents/private",
    ] {
        assert_eq!(
            transport
                .request(
                    ProviderRequest {
                        path,
                        token: Some("synthetic"),
                        form: None
                    },
                    Duration::from_secs(1)
                )
                .err(),
            Some(SetupError::Invalid)
        );
    }
}

#[test]
fn response_status_size_and_malformed_body_fail_without_provider_text() {
    for (status, expected) in [
        (302, SetupError::Unavailable),
        (401, SetupError::Authentication),
        (403, SetupError::Denied),
        (429, SetupError::Capacity),
        (500, SetupError::Unavailable),
    ] {
        let response = ureq::http::Response::builder()
            .status(status)
            .body(ureq::Body::builder().data(b"provider message with sensitive content".to_vec()))
            .unwrap();
        assert_eq!(decode(response).err(), Some(expected));
    }
    for content in [b"not json".to_vec(), vec![b' '; MAX_BODY as usize + 1]] {
        let response = ureq::http::Response::builder()
            .status(200)
            .body(ureq::Body::builder().data(content))
            .unwrap();
        assert_eq!(decode(response).err(), Some(SetupError::Unavailable));
    }
}

fn metadata_provider(tree_data: Value) -> GitHubWebProvider {
    let commit = "a".repeat(40);
    let tree = "b".repeat(40);
    let paths = [
        "/repos/owner/repo".to_string(),
        "/repos/owner/repo/git/ref/heads/main".into(),
        format!("/repos/owner/repo/git/commits/{commit}"),
        format!("/repos/owner/repo/git/trees/{tree}?recursive=1"),
    ];
    provider(vec![
        (
            &paths[0],
            Ok(
                json!({"id":30,"private":false,"permissions":{"admin":true},"default_branch":"main"}),
            ),
        ),
        (
            &paths[1],
            Ok(json!({"object":{"type":"commit","sha":commit}})),
        ),
        (&paths[2], Ok(json!({"sha":commit,"tree":{"sha":tree}}))),
        (&paths[3], Ok(tree_data)),
    ])
}
fn choice() -> RepositoryChoice {
    RepositoryChoice {
        installation_id: 20,
        repository_id: 30,
        owner: "owner".into(),
        name: "repo".into(),
        state: "installed_unconfigured",
    }
}
#[test]
fn metadata_inventory_pins_commit_tree_and_never_fetches_patches_or_blobs() {
    let p = metadata_provider(
        json!({"sha":"b".repeat(40),"truncated":false,"tree":[{"path":"src/lib.rs","type":"blob","mode":"100644","sha":"c".repeat(40)},{"path":"link","type":"blob","mode":"120000","sha":"d".repeat(40)}]}),
    );
    let inventory = p.scope_inventory(&credential(), &choice()).unwrap();
    assert_eq!(inventory.commit, "a".repeat(40));
    assert_eq!(inventory.entries.len(), 2);
    assert_eq!(inventory.entries[0].kind, "symlink");
    for path in [
        "/repos/owner/repo/commits/main",
        "/repos/owner/repo/contents/src/lib.rs",
        "/repos/owner/repo/git/blobs/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "/repos/owner/repo/git/trees/main?recursive=1",
        "/repos/owner/repo/git/ref/heads/main?redirect=https://evil.test",
    ] {
        assert!(!metadata_path(path), "{path}");
    }
}
#[test]
fn truncated_wrong_tree_and_unrecognized_modes_never_appear_complete() {
    for tree in [
        json!({"sha":"b".repeat(40),"truncated":true,"tree":[]}),
        json!({"sha":"c".repeat(40),"truncated":false,"tree":[]}),
        json!({"sha":"b".repeat(40),"truncated":false,"tree":[{"path":"src/lib.rs","type":"blob","mode":"unknown","sha":"c".repeat(40)}]}),
    ] {
        assert_eq!(
            metadata_provider(tree)
                .scope_inventory(&credential(), &choice())
                .err(),
            Some(SetupError::Unavailable)
        );
    }
}
