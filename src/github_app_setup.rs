//! Inactive-by-default App onboarding identity boundary.
//!
//! This module saves inactive scope/context and supports explicit bounded source
//! preview with an operator-configured isolated worker. The optional execution
//! integration supplies an opt-in paused host dispatcher and separate report
//! listener. No execution grants or production consent writer are created.
//! A host must explicitly supply a trusted GitHub web-auth provider to mount it.
//! The existing local device-login broker is not a hosted identity authority.
use crate::Storage;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

mod architecture_preview;
#[cfg(feature = "github-app")]
mod budget;
#[cfg(feature = "github-app")]
mod consent;
#[cfg(feature = "github-app")]
pub use budget::AppRequestBudget;
#[cfg(feature = "github-app")]
pub use consent::{PublicationConsentInput, PublicationOperatorPolicy};
mod http;
mod scan_worker;
mod scope;
mod webhook;
pub use architecture_preview::{
    ArchitectureConfirmation, ArchitectureDecision, ArchitecturePreview, ScanInput,
};
pub use scan_worker::{
    AppScanWorker, PreviewFile, SandboxPreviewWorker, ScanRequest, ScanResult, analyze_preview,
};
pub use scope::{
    InventoryEntry, SavedScope, ScopeDocument, ScopeInput, ScopeInventory, ScopePreview,
};
pub use webhook::{AppWebhookInbox, WebhookNotification, WebhookReceipt, webhook_router};
#[cfg(feature = "github-app")]
mod dispatch;
#[cfg(feature = "github-app")]
mod execution;
#[cfg(feature = "github-app")]
mod pr_worker;
#[cfg(feature = "github-app")]
mod report_host;
#[cfg(feature = "github-app")]
pub use dispatch::{AppDispatchConfig, AppDispatcher, DispatchOutcome};
#[cfg(feature = "github-app")]
pub use pr_worker::{
    AppPrWorker, GitObject, PrWorkerOutput, PrWorkerRequest, SandboxPrWorker, analyze_pr_objects,
};
#[cfg(feature = "github-app")]
pub use report_host::{AppReportReadback, AppReportStore, HttpsReportReadback, app_report_router};
#[cfg(feature = "github-app")]
mod installation;
#[cfg(feature = "github-app")]
mod provider;
#[cfg(feature = "github-app")]
pub use execution::{
    AnalysisInput, AnalysisLease, AppCheckDelivery, AppExecutionProvider, AppExecutionQueue,
    AppReceiptSigner, SignedAppReceipt,
};
pub use http::{disabled_router, router};
#[cfg(feature = "github-app")]
pub use installation::{
    CurrentPullRequest, GitHubInstallationBroker, InstallationCredential, InstallationSession,
};
#[cfg(feature = "github-app")]
pub use provider::GitHubWebProvider;

const STATE_TTL: i64 = 600;
const SESSION_TTL: i64 = 3600;
const MAX_STATES: i64 = 1024;
const MAX_SESSIONS: usize = 1024;
const MAX_REPOSITORIES: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    Invalid,
    Authentication,
    Denied,
    Unavailable,
    Capacity,
    Conflict,
}
type Result<T> = std::result::Result<T, SetupError>;

#[derive(Clone)]
pub struct SetupConfig {
    app_id: i64,
    client_id: String,
    origin: String,
}
impl SetupConfig {
    pub fn new(app_id: i64, client_id: String, origin: String) -> Result<Self> {
        let authority = origin.strip_prefix("https://").ok_or(SetupError::Invalid)?;
        if app_id <= 0
            || client_id.is_empty()
            || client_id.len() > 64
            || !client_id.bytes().all(|b| b.is_ascii_alphanumeric())
            || authority.is_empty()
            || authority.len() > 253
            || !authority
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
            || authority
                .split('.')
                .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
        {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            app_id,
            client_id,
            origin,
        })
    }
    pub fn callback_url(&self) -> String {
        format!("{}/v1/github-app/callback", self.origin)
    }
}

// Deliberately no Debug/Serialize: access tokens may never enter browser responses or logs.
#[derive(Clone)]
pub struct UserCredential {
    token: String,
    user_id: i64,
    expires_at_s: i64,
}
impl UserCredential {
    /// Construct only after web code exchange and a fresh authenticated GET /user.
    pub fn verified(token: String, user_id: i64, expires_at_s: i64) -> Result<Self> {
        if user_id <= 0
            || expires_at_s <= 0
            || token.len() < 16
            || token.len() > 1024
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            token,
            user_id,
            expires_at_s,
        })
    }
    /// Only a trusted server-side provider uses this, never the HTTP layer.
    pub fn token(&self) -> &str {
        &self.token
    }
}

#[derive(Clone)]
pub struct RepositoryAccess {
    pub app_id: i64,
    pub installation_id: i64,
    pub repository_id: i64,
    pub owner: String,
    pub name: String,
    pub public: bool,
    pub suspended: bool,
    pub administrator: bool,
    pub contents_read: bool,
    pub pull_requests_read: bool,
    pub checks_write: bool,
}
#[derive(Clone)]
pub struct AccessSnapshot {
    pub user_id: i64,
    pub repositories: Vec<RepositoryAccess>,
}

pub trait GitHubSetupProvider: Send + Sync + 'static {
    /// Server-side fixed github.com code exchange, exact redirect URI, then GET /user.
    /// Send verifier as code_verifier. Must use bounded HTTPS, no redirects,
    /// no token/body logging and fail closed.
    fn exchange_code(&self, code: &str, callback: &str, verifier: &str) -> Result<UserCredential>;
    /// Fresh GET /user plus user-installation-repository intersection. Validate the
    /// App ID and suspension, and obtain current admin permission from GitHub.
    /// Bounded pagination must fail if incomplete, never treat a partial page as complete.
    fn current_access(&self, credential: &UserCredential) -> Result<AccessSnapshot>;
    /// Explicit scan consent must be verified by the host before calling this.
    /// Only these immutable file entries may be read; never enumerate more paths.
    fn scoped_sources(
        &self,
        _credential: &UserCredential,
        _repository: &RepositoryChoice,
        _files: &[InventoryEntry],
    ) -> Result<Vec<PreviewFile>> {
        Err(SetupError::Unavailable)
    }
    /// Metadata only: exact default-branch commit/tree and a complete bounded
    /// path inventory. Never fetch blob content or execute repository code.
    fn scope_inventory(
        &self,
        _credential: &UserCredential,
        _repository: &RepositoryChoice,
    ) -> Result<ScopeInventory> {
        Err(SetupError::Unavailable)
    }
}

#[derive(Clone)]
struct Session {
    credential: UserCredential,
    expires_at_s: i64,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct RepositoryChoice {
    pub installation_id: i64,
    pub repository_id: i64,
    pub owner: String,
    pub name: String,
    pub state: &'static str,
}
// OAuth state and cookies are transport secrets; deliberately not Debug/Serialize.
pub struct LoginStart {
    pub authorize_url: String,
    pub browser_cookie: String,
}
pub struct LoginComplete {
    pub session_cookie: String,
    pub installation_hint: Option<i64>,
}

pub struct AppSetupHost<P> {
    storage: Mutex<Storage>,
    provider: P,
    config: SetupConfig,
    sessions: Mutex<BTreeMap<String, Session>>,
    scan_admission: Mutex<()>,
    scan_worker: Option<std::sync::Arc<dyn AppScanWorker>>,
    #[cfg(feature = "github-app")]
    publication_policy: Option<PublicationOperatorPolicy>,
}
impl<P: GitHubSetupProvider> AppSetupHost<P> {
    pub fn new(storage: Storage, provider: P, config: SetupConfig) -> Self {
        Self {
            storage: Mutex::new(storage),
            provider,
            config,
            sessions: Mutex::new(BTreeMap::new()),
            scan_worker: None,
            #[cfg(feature = "github-app")]
            publication_policy: None,
            scan_admission: Mutex::new(()),
        }
    }
    pub fn with_scan_worker(mut self, worker: std::sync::Arc<dyn AppScanWorker>) -> Self {
        self.scan_worker = Some(worker);
        self
    }
    pub fn publication_consent_available(&self) -> bool {
        #[cfg(feature = "github-app")]
        {
            self.publication_policy.is_some()
        }
        #[cfg(not(feature = "github-app"))]
        {
            false
        }
    }
    pub fn scan_available(&self) -> bool {
        self.scan_worker.is_some()
    }
    /// A post-install hint is bound to state, but never trusted as authorization.
    pub fn begin_login(&self, installation_hint: Option<i64>) -> Result<LoginStart> {
        self.begin_login_at(installation_hint, now()?)
    }
    fn begin_login_at(&self, hint: Option<i64>, now: i64) -> Result<LoginStart> {
        if hint.is_some_and(|id| id <= 0) {
            return Err(SetupError::Invalid);
        }
        let state = secret()?;
        let browser = secret()?;
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(
            "DELETE FROM app_setup_oauth_states WHERE expires_at_s <= ?1",
            [now],
        )
        .map_err(db)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM app_setup_oauth_states", [], |row| {
                row.get(0)
            })
            .map_err(db)?;
        if count >= MAX_STATES {
            return Err(SetupError::Capacity);
        }
        tx.execute(
            "INSERT INTO app_setup_oauth_states VALUES(?1,?2,?3,?4)",
            params![
                digest(&state),
                digest(&browser),
                hint,
                now.checked_add(STATE_TTL).ok_or(SetupError::Invalid)?
            ],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(LoginStart {
            authorize_url: format!(
                "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
                self.config.client_id,
                encode(&self.config.callback_url()),
                pkce_challenge(&browser),
                state
            ),
            browser_cookie: browser,
        })
    }
    pub fn finish_login(&self, state: &str, browser: &str, code: &str) -> Result<LoginComplete> {
        self.finish_login_at(state, browser, code, now()?)
    }
    fn finish_login_at(
        &self,
        state: &str,
        browser: &str,
        code: &str,
        at: i64,
    ) -> Result<LoginComplete> {
        if !valid_secret(state)
            || !valid_secret(browser)
            || code.is_empty()
            || code.len() > 512
            || !code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(SetupError::Authentication);
        }
        let hint: Option<i64> = {
            let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            let tx = storage
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            let row:Option<Option<i64>>=tx.query_row("SELECT installation_hint FROM app_setup_oauth_states WHERE state_digest=?1 AND browser_digest=?2 AND expires_at_s>?3",params![digest(state),digest(browser),at],|r|r.get(0)).optional().map_err(db)?;
            let hint = row.ok_or(SetupError::Authentication)?;
            tx.execute(
                "DELETE FROM app_setup_oauth_states WHERE state_digest=?1",
                [digest(state)],
            )
            .map_err(db)?;
            tx.commit().map_err(db)?;
            hint
        };
        // Consume state before network exchange. Failed exchange requires a new login.
        let credential = self
            .provider
            .exchange_code(code, &self.config.callback_url(), browser)?;
        if credential.expires_at_s <= at.max(now()?) {
            return Err(SetupError::Authentication);
        }
        let snapshot = self.provider.current_access(&credential)?;
        let choices = self.choices(&credential, snapshot)?;
        if choices.is_empty()
            || hint.is_some_and(|id| !choices.iter().any(|r| r.installation_id == id))
        {
            return Err(SetupError::Denied);
        }
        let at = at.max(now()?);
        if credential.expires_at_s <= at {
            return Err(SetupError::Authentication);
        }
        let token = secret()?;
        let mut sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        sessions.retain(|_, s| s.expires_at_s > at);
        if sessions.len() >= MAX_SESSIONS {
            return Err(SetupError::Capacity);
        }
        let expires_at_s = credential
            .expires_at_s
            .min(at.checked_add(SESSION_TTL).ok_or(SetupError::Invalid)?);
        sessions.insert(
            digest(&token),
            Session {
                credential,
                expires_at_s,
            },
        );
        Ok(LoginComplete {
            session_cookie: token,
            installation_hint: hint,
        })
    }
    fn session(&self, token: &str, at: i64) -> Result<Session> {
        if !valid_secret(token) {
            return Err(SetupError::Authentication);
        }
        let mut sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        sessions.retain(|_, s| s.expires_at_s > at);
        sessions
            .get(&digest(token))
            .cloned()
            .ok_or(SetupError::Authentication)
    }
    fn choices(
        &self,
        credential: &UserCredential,
        snapshot: AccessSnapshot,
    ) -> Result<Vec<RepositoryChoice>> {
        if snapshot.user_id != credential.user_id {
            return Err(SetupError::Authentication);
        }
        if snapshot.repositories.len() > MAX_REPOSITORIES {
            return Err(SetupError::Capacity);
        }
        let mut choices = BTreeMap::new();
        for r in snapshot.repositories {
            if r.app_id <= 0
                || r.installation_id <= 0
                || r.repository_id <= 0
                || !slug(&r.owner)
                || !slug(&r.name)
            {
                return Err(SetupError::Unavailable);
            }
            if r.app_id != self.config.app_id
                || r.suspended
                || !r.public
                || !r.administrator
                || !r.contents_read
                || !r.pull_requests_read
                || !r.checks_write
            {
                continue;
            }
            let key = (r.installation_id, r.repository_id);
            if choices
                .insert(
                    key,
                    RepositoryChoice {
                        installation_id: r.installation_id,
                        repository_id: r.repository_id,
                        owner: r.owner,
                        name: r.name,
                        state: "installed_unconfigured",
                    },
                )
                .is_some()
            {
                return Err(SetupError::Unavailable);
            }
        }
        Ok(choices.into_values().collect())
    }
    pub fn repositories(&self, token: &str) -> Result<Vec<RepositoryChoice>> {
        self.repositories_at(token, now()?)
    }
    fn repositories_at(&self, token: &str, at: i64) -> Result<Vec<RepositoryChoice>> {
        let session = self.session(token, at)?;
        let result = self.choices(
            &session.credential,
            self.provider.current_access(&session.credential)?,
        )?;
        self.session(token, at.max(now()?))?; // Fence logout during provider access.
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let mut allowed = Vec::new();
        for choice in result {
            match scope::unblocked(
                &storage.connection,
                self.config.app_id,
                choice.installation_id,
                choice.repository_id,
            ) {
                Ok(()) => allowed.push(choice),
                Err(SetupError::Denied) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(allowed)
    }
    pub fn select_repository(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<RepositoryChoice> {
        self.select_at(token, installation, repository, now()?)
    }
    fn select_at(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
        at: i64,
    ) -> Result<RepositoryChoice> {
        let session = self.session(token, at)?;
        let r = self
            .repositories_at(token, at)?
            .into_iter()
            .find(|r| r.installation_id == installation && r.repository_id == repository)
            .ok_or(SetupError::Denied)?;
        // Hold the session fence through commit so logout cannot race a write.
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        let commit_time = at.max(now()?);
        if sessions
            .get(&digest(token))
            .is_none_or(|s| s.expires_at_s <= commit_time)
        {
            return Err(SetupError::Authentication);
        }
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        scope::unblocked(
            &storage.connection,
            self.config.app_id,
            installation,
            repository,
        )?;
        storage.connection.execute("INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(app_id,installation_id,repository_id) DO UPDATE SET owner=excluded.owner,name=excluded.name,selected_by_user_id=excluded.selected_by_user_id,selected_at_s=excluded.selected_at_s",params![self.config.app_id,installation,repository,r.owner,r.name,session.credential.user_id,at]).map_err(db)?;
        Ok(r)
    }
    pub fn logout(&self, token: &str) -> Result<()> {
        self.sessions
            .lock()
            .map_err(|_| SetupError::Unavailable)?
            .remove(&digest(token));
        Ok(())
    }
}
fn db(_: rusqlite::Error) -> SetupError {
    SetupError::Unavailable
}
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SetupError::Unavailable)?
        .as_secs()
        .try_into()
        .map_err(|_| SetupError::Unavailable)
}
fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn valid_secret(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
fn secret() -> Result<String> {
    // Target host is Linux. OS CSPRNG only; no timestamp or pseudo-random fallback.
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| SetupError::Unavailable)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
// RFC 7636 S256: base64url encoding of a SHA-256 digest, without padding.
fn pkce_challenge(verifier: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let hash = Sha256::digest(verifier.as_bytes());
    let mut encoded = String::with_capacity(43);
    for chunk in hash.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        encoded.push(ALPHABET[a >> 2] as char);
        encoded.push(ALPHABET[((a & 3) << 4) | (b >> 4)] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[((b & 15) << 2) | (c >> 6)] as char);
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[c & 63] as char);
        }
    }
    encoded
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "github-app"))]
mod pipeline_tests;
