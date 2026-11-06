//! Credential broker: fixed API origin, repository-limited tokens, no worker credentials.
#[cfg(test)]
mod dispatch_tests;
mod objects;
use super::{Result, SetupError, slug};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

// Neither credentials nor key material implement Debug/Serialize.
pub struct InstallationCredential {
    token: String,
    installation: i64,
    repository: i64,
    expires: i64,
}
pub struct GitHubInstallationBroker {
    app_id: i64,
    issuer: String,
    key: RsaKeyPair,
    transport: Box<dyn Transport>,
    admission: Mutex<()>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CurrentPullRequest {
    pub repository_id: i64,
    pub number: i64,
    pub selected_base: String,
    pub merge_base: String,
    pub head: String,
    pub open: bool,
    pub draft: bool,
}
trait Transport: Send + Sync {
    fn request(&self, method: &str, path: &str, token: &str, body: Option<&Value>)
    -> Result<Value>;
}
struct Https(ureq::Agent);
impl Transport for Https {
    fn request(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let mut auth = ureq::http::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| SetupError::Authentication)?;
        auth.set_sensitive(true);
        let url = format!("https://api.github.com{path}");
        let mut response = match method {
            "GET" => self
                .0
                .get(url)
                .header("Authorization", auth)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "CodeShouldBehave-App")
                .call(),
            "POST" => self
                .0
                .post(url)
                .header("Authorization", auth)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "CodeShouldBehave-App")
                .header("Content-Type", "application/json")
                .send(
                    serde_json::to_vec(body.ok_or(SetupError::Invalid)?)
                        .map_err(|_| SetupError::Invalid)?,
                ),
            _ => return Err(SetupError::Invalid),
        }
        .map_err(|_| SetupError::Unavailable)?;
        match response.status().as_u16() {
            200 | 201 => {}
            401 => return Err(SetupError::Authentication),
            403 | 404 => return Err(SetupError::Denied),
            429 => return Err(SetupError::Capacity),
            _ => return Err(SetupError::Unavailable),
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_to_vec()
            .map_err(|_| SetupError::Unavailable)?;
        serde_json::from_slice(&bytes).map_err(|_| SetupError::Unavailable)
    }
}
impl GitHubInstallationBroker {
    /// Call only from the secret-store boundary, never with repository-controlled bytes.
    pub fn new(app_id: i64, client_id: String, pem: &[u8]) -> Result<Self> {
        if app_id <= 0
            || client_id.is_empty()
            || client_id.len() > 64
            || !client_id.bytes().all(|b| b.is_ascii_alphanumeric())
            || pem.len() > 16384
        {
            return Err(SetupError::Invalid);
        }
        let text = std::str::from_utf8(pem)
            .map_err(|_| SetupError::Invalid)?
            .trim();
        let (label, pkcs8) = if text.starts_with(&format!("-----BEGIN {}-----", "RSA PRIVATE KEY"))
        {
            ("RSA PRIVATE KEY", false)
        } else {
            ("PRIVATE KEY", true)
        };
        let encoded = text
            .strip_prefix(&format!("-----BEGIN {label}-----"))
            .and_then(|s| s.strip_suffix(&format!("-----END {label}-----")))
            .ok_or(SetupError::Invalid)?;
        let der = STANDARD
            .decode(
                encoded
                    .chars()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect::<String>(),
            )
            .map_err(|_| SetupError::Invalid)?;
        let key = if pkcs8 {
            RsaKeyPair::from_pkcs8(&der)
        } else {
            RsaKeyPair::from_der(&der)
        }
        .map_err(|_| SetupError::Invalid)?;
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .https_only(true)
                .proxy(None)
                .max_redirects(0)
                .max_redirects_will_error(true)
                .max_response_header_size(16384)
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(10)))
                .build(),
        );
        Ok(Self {
            app_id,
            issuer: client_id,
            key,
            transport: Box::new(Https(agent)),
            admission: Mutex::new(()),
        })
    }
    fn jwt(&self, at: i64) -> Result<String> {
        if !(60..=i64::MAX - 600).contains(&at) {
            return Err(SetupError::Invalid);
        }
        let payload = json!({"iat": at - 60, "exp": at + 540, "iss": self.issuer});
        let message = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).map_err(|_| SetupError::Invalid)?)
        );
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                message.as_bytes(),
                &mut signature,
            )
            .map_err(|_| SetupError::Unavailable)?;
        Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
    /// No cache: issue only after the caller's durable consent/revocation gate.
    /// Token lifetime is capped at 60s locally even when GitHub grants an hour.
    pub fn issue(&self, installation: i64, repository: i64) -> Result<InstallationCredential> {
        let _guard = self
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        self.issue_at(installation, repository, super::now()?)
    }
    fn issue_at(
        &self,
        installation: i64,
        repository: i64,
        at: i64,
    ) -> Result<InstallationCredential> {
        if installation <= 0 || repository <= 0 {
            return Err(SetupError::Invalid);
        }
        let jwt = self.jwt(at)?;
        let start = Instant::now();
        let info = self.transport.request(
            "GET",
            &format!("/app/installations/{installation}"),
            &jwt,
            None,
        )?;
        if info["id"].as_i64() != Some(installation)
            || info["app_id"].as_i64() != Some(self.app_id)
            || info.get("suspended_at") != Some(&Value::Null)
            || !permissions(&info["permissions"], false)
        {
            return Err(SetupError::Denied);
        }
        let request = json!({"repository_ids":[repository],"permissions":{
            "contents":"read","pull_requests":"read","checks":"write"}});
        let response = self.transport.request(
            "POST",
            &format!("/app/installations/{installation}/access_tokens"),
            &jwt,
            Some(&request),
        )?;
        if start.elapsed() >= Duration::from_secs(25) {
            return Err(SetupError::Unavailable);
        }
        let repos = response["repositories"]
            .as_array()
            .ok_or(SetupError::Denied)?;
        if repos.len() != 1
            || repos[0]["id"].as_i64() != Some(repository)
            || !permissions(&response["permissions"], true)
        {
            return Err(SetupError::Denied);
        }
        let token = response["token"]
            .as_str()
            .filter(|s| {
                s.len() >= 16
                    && s.len() <= 16384
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
            })
            .ok_or(SetupError::Authentication)?
            .to_owned();
        let expiry = utc_seconds(
            response["expires_at"]
                .as_str()
                .ok_or(SetupError::Authentication)?,
        )?;
        if expiry <= at + 90 || expiry > at + 3660 {
            return Err(SetupError::Authentication);
        }
        Ok(InstallationCredential {
            token,
            installation,
            repository,
            expires: (at + 60).min(expiry - 30),
        })
    }
    /// Fresh PR and merge-base data, never webhook-supplied revisions. Fork analysis
    /// is deferred until the scoped fork-object adapter is separately qualified.
    pub fn current_pr(
        &self,
        credential: &InstallationCredential,
        owner: &str,
        name: &str,
        number: i64,
    ) -> Result<CurrentPullRequest> {
        let _guard = self
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        if !slug(owner) || !slug(name) || number <= 0 {
            return Err(SetupError::Invalid);
        }
        credential.check(super::now()?)?;
        let prefix = format!("/repos/{owner}/{name}");
        let repo = self
            .transport
            .request("GET", &prefix, &credential.token, None)?;
        if repo["id"].as_i64() != Some(credential.repository)
            || repo["private"].as_bool() != Some(false)
            || repo["archived"].as_bool() != Some(false)
        {
            return Err(SetupError::Denied);
        }
        let pr = self.transport.request(
            "GET",
            &format!("{prefix}/pulls/{number}"),
            &credential.token,
            None,
        )?;
        if pr["number"].as_i64() != Some(number)
            || pr["base"]["repo"]["id"].as_i64() != Some(credential.repository)
            || pr["head"]["repo"]["id"].as_i64() != Some(credential.repository)
            || pr["head"]["repo"]["private"].as_bool() != Some(false)
        {
            return Err(SetupError::Denied);
        }
        let base = oid(&pr["base"]["sha"])?;
        let head = oid(&pr["head"]["sha"])?;
        let open = match pr["state"].as_str() {
            Some("open") => true,
            Some("closed") => false,
            _ => return Err(SetupError::Invalid),
        };
        let compare = self.transport.request(
            "GET",
            &format!("{prefix}/compare/{base}...{head}?per_page=1"),
            &credential.token,
            None,
        )?;
        if oid(&compare["base_commit"]["sha"])? != base {
            return Err(SetupError::Conflict);
        }
        credential.check(super::now()?)?;
        Ok(CurrentPullRequest {
            repository_id: credential.repository,
            number,
            selected_base: base,
            merge_base: oid(&compare["merge_base_commit"]["sha"])?,
            head,
            open,
            draft: pr["draft"].as_bool().ok_or(SetupError::Invalid)?,
        })
    }
}
impl InstallationCredential {
    fn check(&self, at: i64) -> Result<()> {
        if self.installation <= 0 || at >= self.expires {
            Err(SetupError::Authentication)
        } else {
            Ok(())
        }
    }
}
fn permissions(value: &Value, exact: bool) -> bool {
    let Some(p) = value.as_object() else {
        return false;
    };
    p.get("contents").and_then(Value::as_str) == Some("read")
        && p.get("pull_requests").and_then(Value::as_str) == Some("read")
        && p.get("checks").and_then(Value::as_str) == Some("write")
        && (!exact
            || p.iter().all(|(k, v)| {
                matches!(k.as_str(), "contents" | "pull_requests" | "checks")
                    || k == "metadata" && v == "read"
            }))
}
fn oid(value: &Value) -> Result<String> {
    let s = value.as_str().ok_or(SetupError::Invalid)?;
    if s.len() != 40
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(SetupError::Invalid);
    }
    Ok(s.to_owned())
}
// GitHub's documented UTC seconds format; reject offsets/fractions rather than guess.
fn utc_seconds(s: &str) -> Result<i64> {
    if s.len() != 20
        || !s.is_ascii()
        || &s[4..5] != "-"
        || &s[7..8] != "-"
        || &s[10..11] != "T"
        || &s[13..14] != ":"
        || &s[16..17] != ":"
        || &s[19..] != "Z"
    {
        return Err(SetupError::Invalid);
    }
    if s.bytes()
        .enumerate()
        .any(|(i, b)| ![4, 7, 10, 13, 16, 19].contains(&i) && !b.is_ascii_digit())
    {
        return Err(SetupError::Invalid);
    }
    let n = |a, b| s[a..b].parse::<i64>().map_err(|_| SetupError::Invalid);
    let (y, m, d, h, min, sec) = (
        n(0, 4)?,
        n(5, 7)?,
        n(8, 10)?,
        n(11, 13)?,
        n(14, 16)?,
        n(17, 19)?,
    );
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&min)
        || !(0..=59).contains(&sec)
    {
        return Err(SetupError::Invalid);
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if d < 1 || d > days[(m - 1) as usize] {
        return Err(SetupError::Invalid);
    }
    let before =
        |year: i64| 365 * (year - 1) + (year - 1) / 4 - (year - 1) / 100 + (year - 1) / 400;
    Ok(
        (before(y) - before(1970) + days[..(m - 1) as usize].iter().sum::<i64>() + d - 1) * 86400
            + h * 3600
            + min * 60
            + sec,
    )
}
/// Short-lived broker session. Workers receive AnalysisInput, never this object.
pub struct InstallationSession<'a> {
    broker: &'a GitHubInstallationBroker,
    credential: InstallationCredential,
    owner: String,
    name: String,
}
impl GitHubInstallationBroker {
    pub fn session(
        &self,
        installation: i64,
        repository: &crate::public_report::PublicRepository,
    ) -> Result<InstallationSession<'_>> {
        if !slug(&repository.owner) || !slug(&repository.name) {
            return Err(SetupError::Invalid);
        }
        let id: i64 = repository.id.try_into().map_err(|_| SetupError::Invalid)?;
        let credential = self.issue(installation, id)?;
        Ok(InstallationSession {
            broker: self,
            credential,
            owner: repository.owner.clone(),
            name: repository.name.clone(),
        })
    }
}
impl super::execution::AppExecutionProvider for InstallationSession<'_> {
    fn current_pr(
        &self,
        installation: i64,
        repository: i64,
        pr: i64,
    ) -> Result<CurrentPullRequest> {
        if installation != self.credential.installation || repository != self.credential.repository
        {
            return Err(SetupError::Denied);
        }
        self.broker
            .current_pr(&self.credential, &self.owner, &self.name, pr)
    }
}
impl super::execution::AppCheckDelivery for InstallationSession<'_> {
    fn find(&self, repository: i64, head: &str, payload: &Value) -> Result<Option<i64>> {
        let _guard = self
            .broker
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        self.credential.check(super::now()?)?;
        if repository != self.credential.repository || oid(&json!(head))? != head {
            return Err(SetupError::Denied);
        }
        let path = format!(
            "/repos/{}/{}/commits/{head}/check-runs?check_name=CodeShouldBehave%20%2F%20Architecture&filter=all&per_page=100",
            self.owner, self.name
        );
        let response = self
            .broker
            .transport
            .request("GET", &path, &self.credential.token, None)?;
        let checks = response["check_runs"]
            .as_array()
            .ok_or(SetupError::Invalid)?;
        if checks.len() >= 100 || response["total_count"].as_u64() != Some(checks.len() as u64) {
            return Err(SetupError::Capacity);
        }
        let mut found = None;
        for check in checks {
            if check["app"]["id"].as_i64() != Some(self.broker.app_id)
                || check["external_id"] != payload["external_id"]
            {
                continue;
            }
            if found.is_some() {
                return Err(SetupError::Conflict);
            }
            found = Some(check_matches(check, payload, self.broker.app_id)?);
        }
        self.credential.check(super::now()?)?;
        Ok(found)
    }
    fn create(&self, repository: i64, payload: &Value) -> Result<i64> {
        let _guard = self
            .broker
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        self.credential.check(super::now()?)?;
        if repository != self.credential.repository
            || payload["name"] != "CodeShouldBehave / Architecture"
            || payload["status"] != "completed"
            || !matches!(payload["conclusion"].as_str(), Some("success" | "neutral"))
        {
            return Err(SetupError::Denied);
        }
        oid(&payload["head_sha"])?;
        let path = format!("/repos/{}/{}/check-runs", self.owner, self.name);
        let result =
            self.broker
                .transport
                .request("POST", &path, &self.credential.token, Some(payload))?;
        check_matches(&result, payload, self.broker.app_id)
    }
}
fn check_matches(check: &Value, payload: &Value, app: i64) -> Result<i64> {
    if check["app"]["id"].as_i64() != Some(app)
        || [
            "name",
            "head_sha",
            "external_id",
            "details_url",
            "status",
            "conclusion",
        ]
        .iter()
        .any(|key| check[key] != payload[key])
        || check["output"]["title"] != payload["output"]["title"]
        || check["output"]["summary"] != payload["output"]["summary"]
    {
        return Err(SetupError::Conflict);
    }
    check["id"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or(SetupError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        responses: Mutex<std::collections::VecDeque<Value>>,
        requests: Mutex<Vec<(String, String, Option<Value>)>>,
    }
    impl Transport for std::sync::Arc<Fake> {
        fn request(
            &self,
            method: &str,
            path: &str,
            token: &str,
            body: Option<&Value>,
        ) -> Result<Value> {
            assert!(!token.is_empty());
            self.requests
                .lock()
                .unwrap()
                .push((method.into(), path.into(), body.cloned()));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(SetupError::Unavailable)
        }
    }
    #[test]
    fn rsa_jwt_repo_limited_issuance_and_check_ownership() {
        // Ephemeral test key; OpenSSL is a test prerequisite, not a runtime dependency.
        let key = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .unwrap();
        assert!(key.status.success());
        let mut broker = GitHubInstallationBroker::new(1, "client123".into(), &key.stdout).unwrap();
        let at = utc_seconds("2026-01-01T00:00:00Z").unwrap();
        let jwt = broker.jwt(at).unwrap();
        let parts: Vec<_> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims, json!({"iat":at-60,"exp":at+540,"iss":"client123"}));
        use ring::signature::{KeyPair, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};
        UnparsedPublicKey::new(
            &RSA_PKCS1_2048_8192_SHA256,
            broker.key.public_key().as_ref(),
        )
        .verify(
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
        )
        .unwrap();
        let permissions =
            json!({"contents":"read","pull_requests":"read","checks":"write","metadata":"read"});
        let info = json!({"id":2,"app_id":1,"suspended_at":null,"permissions":permissions});
        let response = json!({"repositories":[{"id":3}],"permissions":permissions,"token":"ghs_1_header.payload.signature","expires_at":"2026-01-01T01:00:00Z"});
        let fake = std::sync::Arc::new(Fake {
            responses: Mutex::new([info.clone(), response.clone()].into()),
            requests: Mutex::new(vec![]),
        });
        broker.transport = Box::new(fake.clone());
        let credential = broker.issue_at(2, 3, at).unwrap();
        assert_eq!(credential.expires, at + 60);
        let calls = fake.requests.lock().unwrap();
        assert_eq!(calls[1].0, "POST");
        assert_eq!(calls[1].1, "/app/installations/2/access_tokens");
        assert_eq!(
            calls[1].2.as_ref().unwrap(),
            &json!({"repository_ids":[3],"permissions":{"contents":"read","pull_requests":"read","checks":"write"}})
        );
        drop(calls);
        let mut wrong = response.clone();
        wrong["repositories"] = json!([{"id":3},{"id":4}]);
        fake.responses.lock().unwrap().extend([info.clone(), wrong]);
        assert!(broker.issue_at(2, 3, at).is_err());
        let mut expired = response.clone();
        expired["expires_at"] = json!("2026-01-01T00:00:30Z");
        fake.responses
            .lock()
            .unwrap()
            .extend([info.clone(), expired]);
        assert!(broker.issue_at(2, 3, at).is_err());
        let mut suspended = info;
        suspended["suspended_at"] = json!("2026-01-01T00:00:00Z");
        fake.responses.lock().unwrap().push_back(suspended);
        let count = fake.requests.lock().unwrap().len();
        assert!(broker.issue_at(2, 3, at).is_err());
        assert_eq!(fake.requests.lock().unwrap().len(), count + 1);
        let current_credential = InstallationCredential {
            token: "test-only-session".into(),
            installation: 2,
            repository: 3,
            expires: super::super::now().unwrap() + 60,
        };
        let repo = json!({"id":3,"private":false,"archived":false});
        let pr = json!({"number":7,"state":"open","draft":false,"base":{"sha":"a".repeat(40),"repo":{"id":3}},"head":{"sha":"b".repeat(40),"repo":{"id":3,"private":false}}});
        fake.responses.lock().unwrap().extend([repo.clone(),pr.clone(),json!({"base_commit":{"sha":"a".repeat(40)},"merge_base_commit":{"sha":"c".repeat(40)}})]);
        let current = broker
            .current_pr(&current_credential, "owner", "repo", 7)
            .unwrap();
        assert_eq!(current.selected_base, "a".repeat(40));
        assert_eq!(current.merge_base, "c".repeat(40));
        assert_eq!(current.head, "b".repeat(40));
        let mut fork = pr;
        fork["head"]["repo"]["id"] = json!(4);
        fake.responses.lock().unwrap().extend([repo.clone(), fork]);
        assert_eq!(
            broker.current_pr(&current_credential, "owner", "repo", 7),
            Err(SetupError::Denied)
        );
        let mut private = repo;
        private["private"] = json!(true);
        fake.responses.lock().unwrap().push_back(private);
        assert_eq!(
            broker.current_pr(&current_credential, "owner", "repo", 7),
            Err(SetupError::Denied)
        );
        let payload = json!({"name":"CodeShouldBehave / Architecture","head_sha":"a".repeat(40),"external_id":"test","details_url":"https://example.test/","status":"completed","conclusion":"neutral","output":{"title":"Ready","summary":"Test"}});
        let mut check = payload.clone();
        check["id"] = json!(4);
        check["app"] = json!({"id":1});
        assert_eq!(check_matches(&check, &payload, 1), Ok(4));
        check["app"]["id"] = json!(9);
        assert_eq!(
            check_matches(&check, &payload, 1),
            Err(SetupError::Conflict)
        );
    }
    #[test]
    fn strict_expiry_and_permissions() {
        assert_eq!(utc_seconds("1970-01-01T00:00:00Z").unwrap(), 0);
        assert_eq!(utc_seconds("2024-03-01T00:00:00Z").unwrap(), 1709251200);
        for bad in [
            "2025-02-29T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:00:60Z",
            "2026-01-01T00:00:00+00:00",
        ] {
            assert!(utc_seconds(bad).is_err());
        }
        let mut p =
            json!({"contents":"read","pull_requests":"read","checks":"write","metadata":"read"});
        assert!(permissions(&p, true));
        p["administration"] = json!("write");
        assert!(!permissions(&p, true));
        p["contents"] = json!("write");
        assert!(!permissions(&p, false));
        let c = InstallationCredential {
            token: "secret".into(),
            installation: 1,
            repository: 2,
            expires: 100,
        };
        assert!(c.check(99).is_ok());
        assert_eq!(c.check(100), Err(SetupError::Authentication));
    }
    #[test]
    fn bad_keys_rejected_without_network() {
        assert!(GitHubInstallationBroker::new(1, "abc".into(), b"not a key").is_err());
    }
}
