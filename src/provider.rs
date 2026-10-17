use crate::{JobLane, Scheduler, Storage};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

const MAX_WEBHOOK_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AllowedRepository {
    pub provider: String,
    pub external_id: String,
    pub repository_id: i64,
    pub scope_id: i64,
    pub mirror: PathBuf,
    pub configured_remote: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchLimits {
    pub timeout_ms: u64,
    pub max_objects: usize,
    pub max_bytes: u64,
    pub concurrency_limit: usize,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            max_objects: 10_000,
            max_bytes: 1024 * 1024 * 1024,
            concurrency_limit: 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    pub repository_id: i64,
    pub mirror: PathBuf,
    pub configured_remote: String,
    pub base_commit: String,
    pub head_commit: String,
    pub limits: FetchLimits,
}

pub trait RestrictedFetcher {
    fn fetch(&self, request: &FetchRequest) -> Result<(), String>;
}

pub struct CommandRestrictedFetcher {
    git_binary: PathBuf,
    active: AtomicUsize,
}

impl Default for CommandRestrictedFetcher {
    fn default() -> Self {
        Self::new("git")
    }
}

impl CommandRestrictedFetcher {
    pub fn new(git_binary: impl Into<PathBuf>) -> Self {
        Self {
            git_binary: git_binary.into(),
            active: AtomicUsize::new(0),
        }
    }
}

impl RestrictedFetcher for CommandRestrictedFetcher {
    fn fetch(&self, request: &FetchRequest) -> Result<(), String> {
        validate_fetch_request(request)?;
        let slot = self
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < request.limits.concurrency_limit).then_some(active + 1)
            });
        if slot.is_err() {
            return Err("fetch concurrency limit reached; retry later".into());
        }
        let _slot = FetchSlot(&self.active);
        let starting_bytes = directory_bytes(&request.mirror, 200_000)?;
        if starting_bytes > request.limits.max_bytes {
            return Err("mirror already exceeds fetch byte budget".into());
        }
        let mut child = Command::new(&self.git_binary)
            .arg("-C")
            .arg(&request.mirror)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "protocol.file.allow=never",
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                "--depth=1",
                "--",
            ])
            .arg(&request.configured_remote)
            .arg(&request.base_commit)
            .arg(&request.head_commit)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("SSH_ASKPASS", "")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start restricted fetch: {error}"))?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("cannot inspect fetch: {error}"))?
            {
                break status;
            }
            if started.elapsed() >= Duration::from_millis(request.limits.timeout_ms) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("fetch timed out".into());
            }
            if directory_bytes(&request.mirror, 200_000)? > request.limits.max_bytes {
                let _ = child.kill();
                let _ = child.wait();
                return Err("fetch exceeded byte budget".into());
            }
            thread::sleep(Duration::from_millis(10));
        };
        let mut stderr = Vec::new();
        if let Some(stream) = child.stderr.take() {
            use std::io::Read as _;
            stream
                .take(64 * 1024)
                .read_to_end(&mut stderr)
                .map_err(|error| format!("cannot read fetch error: {error}"))?;
        }
        if !status.success() {
            let detail = String::from_utf8_lossy(&stderr);
            let detail = bounded_reason(detail.trim());
            if detail.to_ascii_lowercase().contains("rate limit")
                || detail.to_ascii_lowercase().contains("retry-after")
            {
                return Err(format!("provider throttled: {detail}"));
            }
            return Err(format!("fetch failed: {detail}"));
        }
        let (objects, bytes) = count_objects(&self.git_binary, &request.mirror)?;
        if objects > request.limits.max_objects || bytes > request.limits.max_bytes {
            return Err(format!(
                "fetch budget exceeded: {objects} objects, {bytes} bytes"
            ));
        }
        Ok(())
    }
}

struct FetchSlot<'a>(&'a AtomicUsize);
impl Drop for FetchSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEvent {
    pub provider: String,
    pub delivery_id: String,
    pub repository_external_id: String,
    pub provider_pr_id: String,
    pub generation: i64,
    pub base_commit: String,
    pub head_commit: String,
    pub target_branch: String,
    pub state: String,
    pub priority: i64,
}

impl ProviderEvent {
    pub fn canonical_body(&self) -> Vec<u8> {
        let mut output = b"flowdiff-provider-event-v1".to_vec();
        for value in [
            &self.provider,
            &self.delivery_id,
            &self.repository_external_id,
            &self.provider_pr_id,
            &self.generation.to_string(),
            &self.base_commit,
            &self.head_commit,
            &self.target_branch,
            &self.state,
            &self.priority.to_string(),
        ] {
            output.extend_from_slice(&(value.len() as u64).to_le_bytes());
            output.extend_from_slice(value.as_bytes());
        }
        output
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntakeOutcome {
    Scheduled { review_id: String, job_id: i64 },
    Duplicate,
    Stale,
    Closed,
    FetchFailed { reason: String },
}

pub struct ProviderIntake<'a, F: RestrictedFetcher> {
    storage: &'a mut Storage,
    fetcher: &'a F,
    allowlist: BTreeMap<(String, String), AllowedRepository>,
    secret: Vec<u8>,
    fetch_limits: FetchLimits,
}

impl<'a, F: RestrictedFetcher> ProviderIntake<'a, F> {
    pub fn new(
        storage: &'a mut Storage,
        fetcher: &'a F,
        repositories: impl IntoIterator<Item = AllowedRepository>,
        secret: &[u8],
        fetch_limits: FetchLimits,
    ) -> Result<Self, String> {
        if secret.len() < 32 {
            return Err("Webhook secret must contain at least 32 bytes".into());
        }
        if fetch_limits.timeout_ms == 0
            || fetch_limits.timeout_ms > 300_000
            || fetch_limits.max_objects == 0
            || fetch_limits.max_objects > 1_000_000
            || fetch_limits.max_bytes == 0
            || fetch_limits.concurrency_limit == 0
            || fetch_limits.concurrency_limit > 32
        {
            return Err("Invalid fetch limits".into());
        }
        let mut allowlist = BTreeMap::new();
        for repository in repositories {
            validate_label("provider", &repository.provider)?;
            validate_label("external repository identity", &repository.external_id)?;
            if repository.configured_remote.is_empty() || repository.mirror.as_os_str().is_empty() {
                return Err("Allowed repository requires a configured mirror and remote".into());
            }
            let key = (repository.provider.clone(), repository.external_id.clone());
            if allowlist.insert(key, repository).is_some() {
                return Err("Duplicate allowed repository".into());
            }
        }
        Ok(Self {
            storage,
            fetcher,
            allowlist,
            secret: secret.to_vec(),
            fetch_limits,
        })
    }

    pub fn accept(
        &mut self,
        raw_body: &[u8],
        signature: &str,
        event: &ProviderEvent,
        now_ms: u64,
    ) -> Result<IntakeOutcome, String> {
        if raw_body.len() > MAX_WEBHOOK_BYTES {
            return Err("Webhook body exceeds 1 MiB".into());
        }
        if event.canonical_body() != raw_body {
            return Err("Parsed event does not match the signed body".into());
        }
        verify_signature(&self.secret, raw_body, signature)?;
        validate_event(event)?;
        let repository = self
            .allowlist
            .get(&(event.provider.clone(), event.repository_external_id.clone()))
            .ok_or("Repository is not allowlisted")?
            .clone();
        let digest = hex(Sha256::digest(raw_body).as_slice());
        let inserted = self.storage.connection.execute(
            "INSERT OR IGNORE INTO event_deliveries(provider,delivery_id,repository_id,accepted_at_ms,body_digest,state)
             VALUES (?1,?2,?3,?4,?5,'accepted')",
            params![event.provider,event.delivery_id,repository.repository_id,to_i64(now_ms)?,digest]
        ).map_err(|error| format!("Cannot persist webhook delivery: {error}"))? == 1;
        if !inserted {
            let existing: String = self
                .storage
                .connection
                .query_row(
                    "SELECT body_digest FROM event_deliveries WHERE provider=?1 AND delivery_id=?2",
                    params![event.provider, event.delivery_id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("Cannot read webhook delivery: {error}"))?;
            if existing != digest {
                return Err("Delivery identity was reused with different content".into());
            }
            return Ok(IntakeOutcome::Duplicate);
        }
        let current_generation: Option<i64> = self
            .storage
            .connection
            .query_row(
                "SELECT generation FROM pr_heads WHERE repository_id=?1 AND provider_pr_id=?2",
                params![repository.repository_id, event.provider_pr_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Cannot inspect current PR: {error}"))?;
        if current_generation.is_some_and(|generation| generation >= event.generation) {
            self.finish_delivery(event, "stale", None)?;
            return Ok(IntakeOutcome::Stale);
        }
        if event.state == "closed" {
            self.storage.connection.execute(
                "INSERT INTO pr_heads(repository_id,provider_pr_id,generation,head_commit,base_commit,observed_at_ms,current_review_id)
                 VALUES (?1,?2,?3,?4,?5,?6,NULL)
                 ON CONFLICT(repository_id,provider_pr_id) DO UPDATE SET generation=excluded.generation,
                   head_commit=excluded.head_commit,base_commit=excluded.base_commit,
                   observed_at_ms=excluded.observed_at_ms,current_review_id=NULL
                 WHERE excluded.generation>pr_heads.generation",
                params![repository.repository_id,event.provider_pr_id,event.generation,event.head_commit,event.base_commit,to_i64(now_ms)?]
            ).map_err(|error| format!("Cannot close PR: {error}"))?;
            self.record_revision(&repository, event, None, now_ms)?;
            self.storage.connection.execute(
                "UPDATE jobs SET state='cancelled',terminal_reason='PR closed',lease_owner=NULL,lease_expires_at_ms=NULL
                 WHERE state IN ('queued','retry') AND review_id IN
                   (SELECT id FROM reviews WHERE repository_id=?1) AND provider_pr_id=?2",
                params![repository.repository_id,event.provider_pr_id]
            ).map_err(|error| format!("Cannot cancel closed PR work: {error}"))?;
            self.finish_delivery(event, "closed", None)?;
            return Ok(IntakeOutcome::Closed);
        }
        let request = FetchRequest {
            repository_id: repository.repository_id,
            mirror: repository.mirror.clone(),
            configured_remote: repository.configured_remote.clone(),
            base_commit: event.base_commit.clone(),
            head_commit: event.head_commit.clone(),
            limits: self.fetch_limits,
        };
        if let Err(reason) = self.fetcher.fetch(&request) {
            let reason = bounded_reason(&reason);
            self.finish_delivery(event, "fetch_failed", Some(&reason))?;
            return Ok(IntakeOutcome::FetchFailed { reason });
        }
        let review_id = review_identity(repository.repository_id, event);
        {
            let mut scheduler = Scheduler::new(self.storage);
            scheduler.register_review(
                &review_id,
                repository.repository_id,
                repository.scope_id,
                &event.base_commit,
                &event.base_commit,
                &event.head_commit,
                "exact",
                now_ms,
            )?;
            scheduler.observe_pr(
                repository.repository_id,
                &event.provider_pr_id,
                event.generation,
                &review_id,
                &event.head_commit,
                &event.base_commit,
                now_ms,
            )?;
        }
        self.record_revision(&repository, event, Some(&review_id), now_ms)?;
        let (job_id, _) = Scheduler::new(self.storage).enqueue(
            &review_id,
            "analyze",
            event.priority,
            JobLane::Warm,
            &event.provider_pr_id,
            event.generation,
            now_ms,
        )?;
        self.finish_delivery(event, "scheduled", None)?;
        Ok(IntakeOutcome::Scheduled { review_id, job_id })
    }

    pub fn revoke_repository(
        &mut self,
        repository_id: i64,
        reason: &str,
        now_ms: u64,
    ) -> Result<usize, String> {
        if reason.is_empty() || reason.len() > 1024 {
            return Err("Invalid revocation reason".into());
        }
        let transaction = self
            .storage
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start revocation transaction: {error}"))?;
        transaction
            .execute(
                "UPDATE attempts SET finished_at_ms=?1,outcome='cancelled_by_revocation'
             WHERE finished_at_ms IS NULL AND job_id IN
               (SELECT j.id FROM jobs j JOIN reviews r ON r.id=j.review_id
                WHERE r.repository_id=?2 AND j.state='running')",
                params![to_i64(now_ms)?, repository_id],
            )
            .map_err(|error| format!("Cannot close revoked attempts: {error}"))?;
        let cancelled = transaction.execute(
            "UPDATE jobs SET state='cancelled',terminal_reason=?1,lease_owner=NULL,lease_expires_at_ms=NULL
             WHERE state IN ('queued','retry','running') AND review_id IN
               (SELECT id FROM reviews WHERE repository_id=?2)", params![reason,repository_id]
        ).map_err(|error| format!("Cannot cancel revoked work: {error}"))?;
        transaction
            .execute(
                "DELETE FROM current_results WHERE repository_id=?1",
                [repository_id],
            )
            .map_err(|error| format!("Cannot revoke current results: {error}"))?;
        transaction
            .execute(
                "UPDATE pr_heads SET current_review_id=NULL WHERE repository_id=?1",
                [repository_id],
            )
            .map_err(|error| format!("Cannot revoke PR heads: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit revocation: {error}"))?;
        Ok(cancelled)
    }

    fn record_revision(
        &self,
        repository: &AllowedRepository,
        event: &ProviderEvent,
        review: Option<&str>,
        now_ms: u64,
    ) -> Result<(), String> {
        self.storage.connection.execute(
            "INSERT INTO pr_revisions(repository_id,provider_pr_id,generation,review_id,base_commit,
             head_commit,target_branch,state,observed_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![repository.repository_id,event.provider_pr_id,event.generation,review,
                event.base_commit,event.head_commit,event.target_branch,event.state,to_i64(now_ms)?]
        ).map_err(|error| format!("Cannot record PR revision: {error}"))?;
        Ok(())
    }

    fn finish_delivery(
        &self,
        event: &ProviderEvent,
        state: &str,
        reason: Option<&str>,
    ) -> Result<(), String> {
        self.storage.connection.execute(
            "UPDATE event_deliveries SET state=?1,failure_reason=?2 WHERE provider=?3 AND delivery_id=?4",
            params![state,reason,event.provider,event.delivery_id]
        ).map_err(|error| format!("Cannot update webhook delivery: {error}"))?;
        Ok(())
    }
}

fn validate_event(event: &ProviderEvent) -> Result<(), String> {
    for (kind, value) in [
        ("provider", &event.provider),
        ("delivery identity", &event.delivery_id),
        ("repository identity", &event.repository_external_id),
        ("PR identity", &event.provider_pr_id),
        ("target branch", &event.target_branch),
    ] {
        validate_label(kind, value)?;
    }
    if event.generation < 0 || !matches!(event.state.as_str(), "open" | "draft" | "closed") {
        return Err("Invalid PR generation or state".into());
    }
    validate_commit(&event.base_commit)?;
    validate_commit(&event.head_commit)
}

fn review_identity(repository_id: i64, event: &ProviderEvent) -> String {
    let mut hash = Sha256::new();
    hash.update(b"flowdiff-provider-review-v1");
    hash.update(repository_id.to_le_bytes());
    hash.update(event.provider_pr_id.as_bytes());
    hash.update(event.generation.to_le_bytes());
    hash.update(event.base_commit.as_bytes());
    hash.update(event.head_commit.as_bytes());
    hash.update(event.target_branch.as_bytes());
    hex(hash.finalize().as_slice())
}

fn verify_signature(secret: &[u8], body: &[u8], supplied: &str) -> Result<(), String> {
    let expected = format!("sha256={}", hex(&hmac_sha256(secret, body)));
    if supplied.len() != expected.len() {
        return Err("Invalid webhook signature".into());
    }
    let difference = supplied
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |value, (left, right)| value | (left ^ right));
    if difference == 0 {
        Ok(())
    } else {
        Err("Invalid webhook signature".into())
    }
}

fn hmac_sha256(secret: &[u8], body: &[u8]) -> [u8; 32] {
    let mut key = [0_u8; 64];
    if secret.len() > 64 {
        key[..32].copy_from_slice(Sha256::digest(secret).as_slice());
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for index in 0..64 {
        inner_pad[index] ^= key[index];
        outer_pad[index] ^= key[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(body);
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn bounded_reason(reason: &str) -> String {
    reason.chars().take(1024).collect()
}
fn validate_fetch_request(request: &FetchRequest) -> Result<(), String> {
    validate_commit(&request.base_commit)?;
    validate_commit(&request.head_commit)?;
    if request.configured_remote.is_empty()
        || request.configured_remote.len() > 128
        || !request
            .configured_remote
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err("Configured remote must be a remote name, not a URL".into());
    }
    let metadata = fs::symlink_metadata(&request.mirror)
        .map_err(|error| format!("cannot inspect configured mirror: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Configured mirror must be a real directory".into());
    }
    if request.limits.timeout_ms == 0
        || request.limits.timeout_ms > 300_000
        || request.limits.max_objects == 0
        || request.limits.max_objects > 1_000_000
        || request.limits.max_bytes == 0
        || request.limits.concurrency_limit == 0
        || request.limits.concurrency_limit > 32
    {
        return Err("Invalid fetch limits".into());
    }
    Ok(())
}

fn directory_bytes(root: &Path, entry_limit: usize) -> Result<u64, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    while let Some(path) = pending.pop() {
        for entry in
            fs::read_dir(&path).map_err(|error| format!("cannot inspect mirror budget: {error}"))?
        {
            let entry = entry.map_err(|error| format!("cannot inspect mirror entry: {error}"))?;
            entries += 1;
            if entries > entry_limit {
                return Err("mirror entry budget exceeded".into());
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("cannot inspect mirror entry: {error}"))?;
            if metadata.file_type().is_symlink() {
                return Err("configured mirror contains a symlink".into());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or("mirror byte count overflow")?;
            }
        }
    }
    Ok(bytes)
}

fn count_objects(git_binary: &Path, mirror: &Path) -> Result<(usize, u64), String> {
    let output = Command::new(git_binary)
        .arg("-C")
        .arg(mirror)
        .args(["count-objects", "-v"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot count fetched objects: {error}"))?;
    if !output.status.success() {
        return Err("cannot count fetched objects".into());
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "invalid object-count output")?;
    let mut count = 0_u64;
    let mut in_pack = 0_u64;
    let mut size_kib = 0_u64;
    let mut size_pack_kib = 0_u64;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid object-count value")?;
        match key {
            "count" => count = value,
            "in-pack" => in_pack = value,
            "size" => size_kib = value,
            "size-pack" => size_pack_kib = value,
            _ => {}
        }
    }
    let objects = usize::try_from(count.checked_add(in_pack).ok_or("object count overflow")?)
        .map_err(|_| "object count exceeds platform range")?;
    let bytes = size_kib
        .checked_add(size_pack_kib)
        .and_then(|value| value.checked_mul(1024))
        .ok_or("object byte count overflow")?;
    Ok((objects, bytes))
}
fn validate_label(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(format!("Invalid {kind}"))
    } else {
        Ok(())
    }
}
fn validate_commit(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("Invalid commit identity".into())
    }
}
fn to_i64(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| "Time exceeds SQLite range".into())
}
fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut result, "{byte:02x}").unwrap();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[derive(Default)]
    struct FakeFetcher {
        requests: Mutex<Vec<FetchRequest>>,
        fail: AtomicBool,
    }
    impl RestrictedFetcher for FakeFetcher {
        fn fetch(&self, request: &FetchRequest) -> Result<(), String> {
            self.requests.lock().unwrap().push(request.clone());
            if self.fail.load(Ordering::Relaxed) {
                Err("provider throttled; retry after 30s".into())
            } else {
                Ok(())
            }
        }
    }

    fn commit(digit: char) -> String {
        std::iter::repeat_n(digit, 40).collect()
    }

    fn event(delivery: &str, generation: i64, head: char) -> ProviderEvent {
        ProviderEvent {
            provider: "github".into(),
            delivery_id: delivery.into(),
            repository_external_id: "repo-1".into(),
            provider_pr_id: "42".into(),
            generation,
            base_commit: commit('1'),
            head_commit: commit(head),
            target_branch: "main".into(),
            state: "open".into(),
            priority: 10,
        }
    }

    fn signature(secret: &[u8], body: &[u8]) -> String {
        format!("sha256={}", hex(&hmac_sha256(secret, body)))
    }

    fn setup(name: &str) -> (Storage, AllowedRepository, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("flowdiff-provider-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut storage = Storage::open(&root.join("state.db"), &root.join("artifacts")).unwrap();
        let repository_id = storage
            .register_repository("tenant", "github", "acme", name)
            .unwrap();
        let scope_id = storage
            .register_scope(repository_id, "v1", b"scope")
            .unwrap();
        let allowed = AllowedRepository {
            provider: "github".into(),
            external_id: "repo-1".into(),
            repository_id,
            scope_id,
            mirror: root.join("mirror.git"),
            configured_remote: "origin".into(),
        };
        (storage, allowed, root)
    }

    #[test]
    fn signed_delivery_is_deduplicated_and_latest_generation_wins() {
        let (mut storage, allowed, root) = setup("latest");
        let fetcher = FakeFetcher::default();
        let secret = [7_u8; 32];
        let mut intake = ProviderIntake::new(
            &mut storage,
            &fetcher,
            [allowed.clone()],
            &secret,
            FetchLimits::default(),
        )
        .unwrap();
        let first = event("delivery-1", 1, '2');
        let body = first.canonical_body();
        let outcome = intake
            .accept(&body, &signature(&secret, &body), &first, 1)
            .unwrap();
        assert!(matches!(outcome, IntakeOutcome::Scheduled { .. }));
        assert_eq!(
            intake
                .accept(&body, &signature(&secret, &body), &first, 2)
                .unwrap(),
            IntakeOutcome::Duplicate
        );
        let stale = event("delivery-stale", 0, '3');
        let stale_body = stale.canonical_body();
        assert_eq!(
            intake
                .accept(&stale_body, &signature(&secret, &stale_body), &stale, 3)
                .unwrap(),
            IntakeOutcome::Stale
        );
        let latest = event("delivery-2", 2, '4');
        let latest_body = latest.canonical_body();
        assert!(matches!(
            intake
                .accept(&latest_body, &signature(&secret, &latest_body), &latest, 4)
                .unwrap(),
            IntakeOutcome::Scheduled { .. }
        ));
        let requests = fetcher.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(
            |request| request.configured_remote == "origin" && request.mirror == allowed.mirror
        ));
        drop(requests);
        assert!(
            intake
                .revoke_repository(allowed.repository_id, "scope grant revoked", 5)
                .unwrap()
                >= 1
        );
        let active: i64 = intake
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM jobs WHERE state IN ('queued','retry','running')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 0);
        let current: i64 = intake
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM pr_heads WHERE current_review_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current, 0);
        drop(intake);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_signature_unapproved_repository_and_body_mismatch_never_fetch() {
        let (mut storage, allowed, root) = setup("reject");
        let fetcher = FakeFetcher::default();
        let secret = [8_u8; 32];
        let mut intake = ProviderIntake::new(
            &mut storage,
            &fetcher,
            [allowed],
            &secret,
            FetchLimits::default(),
        )
        .unwrap();
        let valid = event("bad-signature", 1, '2');
        let body = valid.canonical_body();
        assert!(intake.accept(&body, "sha256=00", &valid, 1).is_err());
        assert!(
            intake
                .accept(b"different", &signature(&secret, b"different"), &valid, 1)
                .is_err()
        );
        let mut hostile = event("hostile", 1, '2');
        hostile.repository_external_id = "https://attacker.example/repo.git".into();
        let hostile_body = hostile.canonical_body();
        assert!(
            intake
                .accept(
                    &hostile_body,
                    &signature(&secret, &hostile_body),
                    &hostile,
                    1
                )
                .is_err()
        );
        assert!(fetcher.requests.lock().unwrap().is_empty());
        let deliveries: i64 = intake
            .storage
            .connection
            .query_row("SELECT count(*) FROM event_deliveries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(deliveries, 0);
        drop(intake);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fetch_failure_and_close_are_actionable_without_empty_results() {
        let (mut storage, allowed, root) = setup("failure");
        let fetcher = FakeFetcher::default();
        fetcher.fail.store(true, Ordering::Relaxed);
        let secret = [9_u8; 32];
        let mut intake = ProviderIntake::new(
            &mut storage,
            &fetcher,
            [allowed],
            &secret,
            FetchLimits::default(),
        )
        .unwrap();
        let failed = event("failed-fetch", 1, '2');
        let body = failed.canonical_body();
        let outcome = intake
            .accept(&body, &signature(&secret, &body), &failed, 1)
            .unwrap();
        assert!(
            matches!(outcome, IntakeOutcome::FetchFailed { reason } if reason.contains("throttled"))
        );
        let jobs: i64 = intake
            .storage
            .connection
            .query_row("SELECT count(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(jobs, 0);
        let state: (String, Option<String>) = intake.storage.connection.query_row(
            "SELECT state,failure_reason FROM event_deliveries WHERE delivery_id='failed-fetch'", [], |row| Ok((row.get(0)?,row.get(1)?))
        ).unwrap();
        assert_eq!(state.0, "fetch_failed");
        fetcher.fail.store(false, Ordering::Relaxed);
        let mut closed = event("closed", 2, '3');
        closed.state = "closed".into();
        let closed_body = closed.canonical_body();
        assert_eq!(
            intake
                .accept(&closed_body, &signature(&secret, &closed_body), &closed, 2)
                .unwrap(),
            IntakeOutcome::Closed
        );
        drop(intake);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    fn executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn command_fetcher_enforces_remote_timeout_object_and_throttle_limits() {
        let root =
            std::env::temp_dir().join(format!("flowdiff-command-fetcher-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("mirror")).unwrap();
        let script = root.join("fake-git");
        let request = |limits| FetchRequest {
            repository_id: 1,
            mirror: root.join("mirror"),
            configured_remote: "origin".into(),
            base_commit: commit('1'),
            head_commit: commit('2'),
            limits,
        };

        executable(
            &script,
            "#!/bin/sh\ncase \"$*\" in *count-objects*) echo 'count: 0'; echo 'size: 0'; echo 'in-pack: 99'; echo 'size-pack: 1';; esac\nexit 0\n",
        );
        let fetcher = CommandRestrictedFetcher::new(&script);
        let limits = FetchLimits {
            max_objects: 10,
            ..FetchLimits::default()
        };
        assert!(
            fetcher
                .fetch(&request(limits))
                .unwrap_err()
                .contains("budget exceeded")
        );

        executable(&script, "#!/bin/sh\nsleep 1\nexit 0\n");
        let limits = FetchLimits {
            timeout_ms: 20,
            ..FetchLimits::default()
        };
        assert_eq!(
            fetcher.fetch(&request(limits)).unwrap_err(),
            "fetch timed out"
        );

        executable(
            &script,
            "#!/bin/sh\necho 'rate limit retry-after: 30' >&2\nexit 1\n",
        );
        assert!(
            fetcher
                .fetch(&request(FetchLimits::default()))
                .unwrap_err()
                .contains("provider throttled")
        );

        let mut hostile = request(FetchLimits::default());
        hostile.configured_remote = "https://attacker.example/x".into();
        assert!(fetcher.fetch(&hostile).unwrap_err().contains("not a URL"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn command_fetcher_rejects_work_above_credentialed_concurrency() {
        use std::sync::Arc;
        let root =
            std::env::temp_dir().join(format!("flowdiff-fetch-concurrency-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("mirror")).unwrap();
        let script = root.join("fake-git");
        executable(
            &script,
            "#!/bin/sh\ncase \"$*\" in *count-objects*) echo 'count: 0'; echo 'size: 0'; echo 'in-pack: 0'; echo 'size-pack: 0'; exit 0;; esac\nsleep 0.2\nexit 0\n",
        );
        let fetcher = Arc::new(CommandRestrictedFetcher::new(&script));
        let limits = FetchLimits {
            concurrency_limit: 1,
            ..FetchLimits::default()
        };
        let request = FetchRequest {
            repository_id: 1,
            mirror: root.join("mirror"),
            configured_remote: "origin".into(),
            base_commit: commit('1'),
            head_commit: commit('2'),
            limits,
        };
        let running = {
            let fetcher = fetcher.clone();
            let request = request.clone();
            std::thread::spawn(move || fetcher.fetch(&request))
        };
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            fetcher
                .fetch(&request)
                .unwrap_err()
                .contains("concurrency limit")
        );
        running.join().unwrap().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
