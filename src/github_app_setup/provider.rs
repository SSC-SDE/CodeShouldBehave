//! Hosted web OAuth and explicit scoped-blob adapter. Fixed GitHub endpoints only.
use super::*;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const API: &str = "https://api.github.com";
const EXCHANGE: &str = "https://github.com/login/oauth/access_token";
const MAX_REQUESTS: usize = 32;
const MAX_BODY: u64 = 2 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);

// Never derive Debug: forms and Authorization headers contain credentials.
struct ProviderRequest<'a> {
    path: &'a str,
    token: Option<&'a str>,
    form: Option<Vec<(&'a str, &'a str)>>,
}
trait Transport: Send + Sync {
    fn request(&self, request: ProviderRequest<'_>, timeout: Duration) -> Result<Value>;
}
struct Https {
    agent: ureq::Agent,
}
impl Https {
    fn new() -> Self {
        Self {
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .https_only(true)
                    .proxy(None)
                    .max_redirects(0)
                    .max_redirects_will_error(true)
                    .max_response_header_size(16 * 1024)
                    .http_status_as_error(false)
                    .timeout_global(Some(Duration::from_secs(10)))
                    .build(),
            ),
        }
    }
}
impl Transport for Https {
    fn request(&self, r: ProviderRequest<'_>, timeout: Duration) -> Result<Value> {
        let response = if let Some(form) = r.form {
            if r.path != EXCHANGE || r.token.is_some() {
                return Err(SetupError::Invalid);
            }
            self.agent
                .post(EXCHANGE)
                .header("Accept", "application/json")
                .header("User-Agent", "CodeShouldBehave-App-Setup")
                .config()
                .timeout_global(Some(timeout.min(Duration::from_secs(10))))
                .build()
                .send_form(form)
        } else {
            if r.path != "/user"
                && !r.path.starts_with("/user/installations?")
                && !r.path.starts_with("/user/installations/")
                && !metadata_path(r.path)
                && !blob_path(r.path)
            {
                return Err(SetupError::Invalid);
            }
            let mut auth = ureq::http::HeaderValue::from_str(&format!(
                "Bearer {}",
                r.token.ok_or(SetupError::Authentication)?
            ))
            .map_err(|_| SetupError::Authentication)?;
            auth.set_sensitive(true);
            self.agent
                .get(format!("{API}{}", r.path))
                .header("Authorization", auth)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "CodeShouldBehave-App-Setup")
                .config()
                .timeout_global(Some(timeout.min(Duration::from_secs(10))))
                .build()
                .call()
        }
        .map_err(|_| SetupError::Unavailable)?;
        decode(response)
    }
}
fn decode(mut response: ureq::http::Response<ureq::Body>) -> Result<Value> {
    match response.status().as_u16() {
        200 => {}
        401 => return Err(SetupError::Authentication),
        403 => return Err(SetupError::Denied),
        429 => return Err(SetupError::Capacity),
        _ => return Err(SetupError::Unavailable),
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|_| SetupError::Unavailable)?;
    serde_json::from_slice(&bytes).map_err(|_| SetupError::Unavailable)
}
struct Pass<'a> {
    transport: &'a dyn Transport,
    start: Instant,
    requests: usize,
    bytes: usize,
}
impl<'a> Pass<'a> {
    fn new(transport: &'a dyn Transport) -> Self {
        Self {
            transport,
            start: Instant::now(),
            requests: 0,
            bytes: 0,
        }
    }
    fn request(&mut self, r: ProviderRequest<'_>) -> Result<Value> {
        let remaining = DEADLINE
            .checked_sub(self.start.elapsed())
            .ok_or(SetupError::Unavailable)?;
        if self.requests >= MAX_REQUESTS {
            return Err(SetupError::Capacity);
        }
        self.requests += 1;
        let value = self.transport.request(r, remaining)?;
        self.bytes = self
            .bytes
            .checked_add(
                serde_json::to_vec(&value)
                    .map_err(|_| SetupError::Unavailable)?
                    .len(),
            )
            .ok_or(SetupError::Capacity)?;
        if self.bytes > 16 * 1024 * 1024 {
            return Err(SetupError::Capacity);
        }
        if self.start.elapsed() >= DEADLINE {
            return Err(SetupError::Unavailable);
        }
        Ok(value)
    }
    fn get(&mut self, path: &str, token: &str) -> Result<Value> {
        self.request(ProviderRequest {
            path,
            token: Some(token),
            form: None,
        })
    }
    fn pages(&mut self, path: &str, key: &str, token: &str, limit: usize) -> Result<Vec<Value>> {
        let mut rows = vec![];
        let mut expected = None;
        let mut seen = BTreeSet::new();
        for page in 1..=MAX_REQUESTS {
            let value = self.get(&format!("{path}?per_page=100&page={page}"), token)?;
            let total = value
                .get("total_count")
                .and_then(Value::as_u64)
                .ok_or(SetupError::Unavailable)?;
            if total > limit as u64 {
                return Err(SetupError::Capacity);
            }
            if expected.is_some_and(|n| n != total) {
                return Err(SetupError::Unavailable);
            }
            expected = Some(total);
            let entries = value
                .get(key)
                .and_then(Value::as_array)
                .ok_or(SetupError::Unavailable)?;
            if entries.len() > 100 || (entries.is_empty() && rows.len() < total as usize) {
                return Err(SetupError::Unavailable);
            }
            for entry in entries {
                if !seen.insert(id(entry, "id")?) {
                    return Err(SetupError::Unavailable);
                }
                rows.push(entry.clone());
            }
            if rows.len() == total as usize {
                return Ok(rows);
            }
            if rows.len() > total as usize {
                return Err(SetupError::Unavailable);
            }
        }
        Err(SetupError::Capacity)
    }
}
struct Permit<'a>(&'a AtomicUsize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
/// Concrete optional HTTPS provider. Construct with the operator's client secret
/// from a server-side secret store. No requests occur until the host invokes it.
/// No private key, installation token, webhook or publication is used here.
pub struct GitHubWebProvider {
    config: SetupConfig,
    client_secret: String,
    transport: Arc<dyn Transport>,
    active: AtomicUsize,
}
impl GitHubWebProvider {
    pub fn new(config: SetupConfig, client_secret: String) -> Result<Self> {
        if client_secret.len() < 16
            || client_secret.len() > 256
            || !client_secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            config,
            client_secret,
            transport: Arc::new(Https::new()),
            active: AtomicUsize::new(0),
        })
    }
    fn permit(&self) -> Result<Permit<'_>> {
        self.active
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                if n < 4 { Some(n + 1) } else { None }
            })
            .map_err(|_| SetupError::Capacity)?;
        Ok(Permit(&self.active))
    }
}
impl GitHubSetupProvider for GitHubWebProvider {
    fn exchange_code(&self, code: &str, callback: &str, verifier: &str) -> Result<UserCredential> {
        if callback != self.config.callback_url()
            || !valid_secret(verifier)
            || code.is_empty()
            || code.len() > 512
        {
            return Err(SetupError::Invalid);
        }
        let _permit = self.permit()?;
        let mut pass = Pass::new(self.transport.as_ref());
        let value = pass.request(ProviderRequest {
            path: EXCHANGE,
            token: None,
            form: Some(vec![
                ("client_id", &self.config.client_id),
                ("client_secret", &self.client_secret),
                ("code", code),
                ("redirect_uri", callback),
                ("code_verifier", verifier),
            ]),
        })?;
        if value.get("error").is_some()
            || value.get("token_type").and_then(Value::as_str) != Some("bearer")
        {
            return Err(SetupError::Authentication);
        }
        let token = value
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or(SetupError::Authentication)?;
        if !token.starts_with("ghu_") {
            return Err(SetupError::Authentication);
        }
        // A non-expiring App token still gets a short local lease; never store refresh tokens.
        let ttl = match value.get("expires_in") {
            None => SESSION_TTL,
            Some(n) => n
                .as_i64()
                .filter(|n| *n > 0 && *n <= 28800)
                .ok_or(SetupError::Authentication)?,
        };
        let expires = now()?.checked_add(ttl).ok_or(SetupError::Authentication)?;
        let credential = UserCredential::verified(token.into(), 1, expires)?;
        let user = pass.get("/user", credential.token())?;
        UserCredential::verified(token.into(), id(&user, "id")?, expires)
    }
    fn scoped_sources(
        &self,
        credential: &UserCredential,
        repo: &RepositoryChoice,
        files: &[InventoryEntry],
    ) -> Result<Vec<PreviewFile>> {
        use base64::Engine;
        if credential.expires_at_s <= now()? || !slug(&repo.owner) || !slug(&repo.name) {
            return Err(SetupError::Authentication);
        }
        if files.len() > scan_worker::MAX_SOURCE_FILES
            || files.iter().any(|f| {
                f.kind != "file"
                    || !scope::oid(&f.oid)
                    || crate::sensitive_path(&f.path)
                    || !scan_worker::supported_source(&f.path)
            })
        {
            return Err(SetupError::Invalid);
        }
        let _permit = self.permit()?;
        let mut pass = Pass::new(self.transport.as_ref());
        let mut result = Vec::new();
        for file in files {
            let data = pass.get(
                &format!("/repos/{}/{}/git/blobs/{}", repo.owner, repo.name, file.oid),
                credential.token(),
            )?;
            let size = data
                .get("size")
                .and_then(Value::as_u64)
                .ok_or(SetupError::Unavailable)?;
            if size > scan_worker::MAX_SOURCE_BYTES as u64 {
                return Err(SetupError::Capacity);
            }
            if data.get("sha").and_then(Value::as_str) != Some(&file.oid)
                || data.get("encoding").and_then(Value::as_str) != Some("base64")
            {
                return Err(SetupError::Unavailable);
            }
            let encoded = data
                .get("content")
                .and_then(Value::as_str)
                .ok_or(SetupError::Unavailable)?;
            if encoded.len() > 512 * 1024 {
                return Err(SetupError::Capacity);
            }
            let compact = encoded
                .bytes()
                .filter(|c| !matches!(c, b'\r' | b'\n'))
                .collect::<Vec<_>>();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(compact)
                .map_err(|_| SetupError::Unavailable)?;
            if bytes.len() as u64 != size || bytes.contains(&0) {
                return Err(SetupError::Unavailable);
            }
            let content = String::from_utf8(bytes).map_err(|_| SetupError::Unavailable)?;
            result.push(PreviewFile {
                path: file.path.clone(),
                oid: file.oid.clone(),
                content: Some(content),
            });
        }
        Ok(result)
    }
    fn scope_inventory(
        &self,
        credential: &UserCredential,
        repository: &RepositoryChoice,
    ) -> Result<ScopeInventory> {
        if credential.expires_at_s <= now()? || !slug(&repository.owner) || !slug(&repository.name)
        {
            return Err(SetupError::Authentication);
        }
        let _permit = self.permit()?;
        let mut pass = Pass::new(self.transport.as_ref());
        let root = format!("/repos/{}/{}", repository.owner, repository.name);
        let metadata = pass.get(&root, credential.token())?;
        if id(&metadata, "id")? != repository.repository_id
            || metadata.get("private").and_then(Value::as_bool) != Some(false)
            || metadata
                .pointer("/permissions/admin")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(SetupError::Denied);
        }
        let branch = metadata
            .get("default_branch")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .ok_or(SetupError::Unavailable)?;
        // Git reference and Git commit APIs return metadata, unlike the ordinary
        // commits endpoint which can include file patches. Never request blobs.
        let reference = pass.get(
            &format!("{root}/git/ref/heads/{}", encode(branch)),
            credential.token(),
        )?;
        if reference.pointer("/object/type").and_then(Value::as_str) != Some("commit") {
            return Err(SetupError::Unavailable);
        }
        let commit = reference
            .pointer("/object/sha")
            .and_then(Value::as_str)
            .filter(|s| scope::oid(s))
            .ok_or(SetupError::Unavailable)?;
        let commit_data = pass.get(&format!("{root}/git/commits/{commit}"), credential.token())?;
        if commit_data.get("sha").and_then(Value::as_str) != Some(commit) {
            return Err(SetupError::Unavailable);
        }
        let tree = commit_data
            .pointer("/tree/sha")
            .and_then(Value::as_str)
            .filter(|s| scope::oid(s))
            .ok_or(SetupError::Unavailable)?;
        let tree_data = pass.get(
            &format!("{root}/git/trees/{tree}?recursive=1"),
            credential.token(),
        )?;
        if tree_data.get("sha").and_then(Value::as_str) != Some(tree)
            || tree_data.get("truncated").and_then(Value::as_bool) != Some(false)
        {
            return Err(SetupError::Unavailable);
        }
        let entries = tree_data
            .get("tree")
            .and_then(Value::as_array)
            .ok_or(SetupError::Unavailable)?;
        if entries.len() > 2000 {
            return Err(SetupError::Capacity);
        }
        let entries = entries
            .iter()
            .map(|entry| {
                let kind = match (
                    entry.get("type").and_then(Value::as_str),
                    entry.get("mode").and_then(Value::as_str),
                ) {
                    (Some("blob"), Some("100644" | "100755")) => "file",
                    (Some("blob"), Some("120000")) => "symlink",
                    (Some("tree"), Some("040000")) => "directory",
                    (Some("commit"), Some("160000")) => "submodule",
                    _ => return Err(SetupError::Unavailable),
                };
                Ok(InventoryEntry {
                    path: entry
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or(SetupError::Unavailable)?
                        .into(),
                    kind: kind.into(),
                    oid: entry
                        .get("sha")
                        .and_then(Value::as_str)
                        .ok_or(SetupError::Unavailable)?
                        .into(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut inventory = ScopeInventory {
            repository_id: repository.repository_id,
            default_branch: branch.into(),
            commit: commit.into(),
            tree: tree.into(),
            entries,
        };
        inventory.validate(repository.repository_id)?;
        Ok(inventory)
    }
    fn current_access(&self, credential: &UserCredential) -> Result<AccessSnapshot> {
        if credential.expires_at_s <= now()? {
            return Err(SetupError::Authentication);
        }
        let _permit = self.permit()?;
        let mut pass = Pass::new(self.transport.as_ref());
        let user = pass.get("/user", credential.token())?;
        let user_id = id(&user, "id")?;
        if user_id != credential.user_id {
            return Err(SetupError::Authentication);
        }
        let installations = pass.pages(
            "/user/installations",
            "installations",
            credential.token(),
            100,
        )?;
        let mut repositories = vec![];
        for installation in installations {
            let app_id = id(&installation, "app_id")?;
            if app_id != self.config.app_id {
                continue;
            }
            let suspension = installation
                .get("suspended_at")
                .ok_or(SetupError::Unavailable)?;
            if !suspension.is_null() {
                continue;
            }
            let permissions = installation
                .get("permissions")
                .and_then(Value::as_object)
                .ok_or(SetupError::Unavailable)?;
            let readable = |key: &str| {
                matches!(
                    permissions.get(key).and_then(Value::as_str),
                    Some("read" | "write")
                )
            };
            if !readable("metadata")
                || !readable("contents")
                || !readable("pull_requests")
                || permissions.get("checks").and_then(Value::as_str) != Some("write")
            {
                continue;
            }
            let installation_id = id(&installation, "id")?;
            let entries = pass.pages(
                &format!("/user/installations/{installation_id}/repositories"),
                "repositories",
                credential.token(),
                MAX_REPOSITORIES.saturating_sub(repositories.len()),
            )?;
            for repo in entries {
                let repository_id = id(&repo, "id")?;
                if repo.get("private").and_then(Value::as_bool) != Some(false)
                    || repo.get("visibility").and_then(Value::as_str) != Some("public")
                    || repo.pointer("/permissions/admin").and_then(Value::as_bool) != Some(true)
                {
                    continue;
                }
                let owner = repo
                    .pointer("/owner/login")
                    .and_then(Value::as_str)
                    .filter(|s| slug(s))
                    .ok_or(SetupError::Unavailable)?
                    .to_string();
                let name = repo
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| slug(s))
                    .ok_or(SetupError::Unavailable)?
                    .to_string();
                repositories.push(RepositoryAccess {
                    app_id,
                    installation_id,
                    repository_id,
                    owner,
                    name,
                    public: true,
                    suspended: false,
                    administrator: true,
                    contents_read: true,
                    pull_requests_read: true,
                    checks_write: true,
                });
            }
        }
        Ok(AccessSnapshot {
            user_id,
            repositories,
        })
    }
}
fn id(value: &Value, key: &str) -> Result<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or(SetupError::Unavailable)
}

#[cfg(test)]
mod tests;

fn metadata_path(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() < 4
        || !parts[0].is_empty()
        || parts[1] != "repos"
        || !slug(parts[2])
        || !slug(parts[3])
    {
        return false;
    }
    match &parts[4..] {
        [] => true,
        ["git", "ref", "heads", branch] => {
            !branch.is_empty() && !branch.contains(['?', '#']) && branch.len() <= 768
        }
        ["git", "commits", sha] => scope::oid(sha),
        ["git", "trees", sha] => sha.strip_suffix("?recursive=1").is_some_and(scope::oid),
        _ => false,
    }
}

fn blob_path(path: &str) -> bool {
    let p = path.split('/').collect::<Vec<_>>();
    matches!(p.as_slice(),["","repos",owner,name,"git","blobs",oid] if slug(owner)&&slug(name)&&scope::oid(oid))
}
