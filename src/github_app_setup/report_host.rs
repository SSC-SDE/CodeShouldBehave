//! Cookie-free immutable report serving. Never mount on the authentication origin.
use super::*;
use crate::public_report::{
    PublicBundle, PublicManifest, canonical_json, read_json_file, sha256, verify_bundle,
    write_bundle,
};
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
const MAX_REPORTS: i64 = 100;
const RETENTION: i64 = 7 * 24 * 3600;
pub struct AppReportStore {
    root: PathBuf,
    storage: Mutex<Storage>,
    queue: Arc<AppExecutionQueue>,
    key: Vec<u8>,
    app_id: i64,
    publication: Mutex<()>,
}
impl AppReportStore {
    pub fn new(
        root: PathBuf,
        storage: Storage,
        queue: Arc<AppExecutionQueue>,
        key: Vec<u8>,
        app_id: i64,
    ) -> Result<Self> {
        if key.len() != 32 || app_id <= 0 || !root.is_absolute() {
            return Err(SetupError::Invalid);
        }
        if !root.exists() {
            let mut d = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                d.mode(0o700);
            }
            d.create(&root).map_err(|_| SetupError::Unavailable)?;
        }
        if root.is_symlink()
            || !root.is_dir()
            || std::fs::canonicalize(&root).map_err(|_| SetupError::Unavailable)? != root
        {
            return Err(SetupError::Invalid);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if root
                .metadata()
                .map_err(|_| SetupError::Unavailable)?
                .permissions()
                .mode()
                & 0o077
                != 0
            {
                return Err(SetupError::Denied);
            }
        }
        let reports = root.join("reviews");
        if !reports.exists() {
            std::fs::create_dir(&reports).map_err(|_| SetupError::Unavailable)?;
        }
        if reports.is_symlink() {
            return Err(SetupError::Denied);
        }
        queue.audit_receipts(&key)?;
        Ok(Self {
            root,
            storage: Mutex::new(storage),
            queue,
            key,
            app_id,
            publication: Mutex::new(()),
        })
    }
    /// Stage verified bytes before the signed queue intent. A stranded directory is
    /// not visible: every route also requires a signed, authorized catalog record.
    pub fn stage_bytes(&self, bundle: &PublicBundle) -> Result<()> {
        let _guard = self
            .publication
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        bundle.validate().map_err(|_| SetupError::Invalid)?;
        let reports = self.root.join("reviews");
        let target = reports.join(&bundle.manifest.public_report_id);
        if target.exists() {
            let existing = verify_bundle(&target).map_err(|_| SetupError::Invalid)?;
            if canonical_json(&existing).map_err(|_| SetupError::Invalid)?
                != canonical_json(&bundle.manifest).map_err(|_| SetupError::Invalid)?
            {
                return Err(SetupError::Conflict);
            }
            return Ok(());
        }
        if std::fs::read_dir(&reports)
            .map_err(|_| SetupError::Unavailable)?
            .take(MAX_REPORTS as usize)
            .count()
            >= MAX_REPORTS as usize
        {
            return Err(SetupError::Capacity);
        }
        let temporary = reports.join(format!("pending-{}", secret()?));
        write_bundle(bundle, &temporary).map_err(|_| SetupError::Unavailable)?;
        let result = (|| {
            for path in bundle
                .files
                .keys()
                .map(String::as_str)
                .chain(std::iter::once("manifest.json"))
            {
                std::fs::File::open(temporary.join(path))
                    .and_then(|f| f.sync_all())
                    .map_err(|_| SetupError::Unavailable)?;
            }
            // Persist nested directory entries before making the root reachable.
            let mut directories = std::collections::BTreeSet::from([temporary.clone()]);
            for path in bundle.files.keys() {
                let mut parent = temporary.join(path).parent().map(FsPath::to_path_buf);
                while let Some(dir) = parent {
                    if dir == temporary {
                        break;
                    }
                    directories.insert(dir.clone());
                    parent = dir.parent().map(FsPath::to_path_buf);
                }
            }
            for dir in directories.iter().rev() {
                std::fs::File::open(dir)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| SetupError::Unavailable)?;
            }
            std::fs::rename(&temporary, &target).map_err(|_| SetupError::Unavailable)?;
            std::fs::File::open(&reports)
                .and_then(|f| f.sync_all())
                .map_err(|_| SetupError::Unavailable)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&temporary);
        }
        result
    }
    /// Recover only bytes matching the already persisted exact signed intent.
    pub fn expose(&self, job: &str, provider: &dyn AppExecutionProvider) -> Result<()> {
        let signed = self.queue.authorize_publication(job, &self.key, provider)?;
        if signed.app_id() != self.app_id {
            return Err(SetupError::Denied);
        }
        let manifest = verify_bundle(&self.root.join("reviews").join(signed.report_id()))
            .map_err(|_| SetupError::Unavailable)?;
        if sha256(&canonical_json(&manifest).map_err(|_| SetupError::Invalid)?)
            != signed.manifest_digest()
        {
            return Err(SetupError::Invalid);
        }
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        s.connection.execute("INSERT INTO app_report_publications(app_id,job_id,report_id,manifest_digest,expires_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(job_id) DO NOTHING",params![self.app_id,job,signed.report_id(),signed.manifest_digest(),now()?+RETENTION]).map_err(db)?;
        drop(s);
        self.manifest(signed.report_id()).map(|_| ())
    }
    fn manifest(&self, id: &str) -> Result<PublicManifest> {
        if !valid_secret(id) {
            return Err(SetupError::Invalid);
        }
        let (job, digest): (String, String) = {
            let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            s.connection.query_row("SELECT job_id,manifest_digest FROM app_report_publications WHERE app_id=?1 AND report_id=?2 AND withdrawn=0 AND expires_at>?3",params![self.app_id,id,now()?],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db)?.ok_or(SetupError::Denied)?
        };
        let signed = self.queue.intent(&job, &self.key)?;
        if signed.report_id() != id || signed.manifest_digest() != digest {
            return Err(SetupError::Denied);
        }
        let value = read_json_file(
            &self.root.join("reviews").join(id).join("manifest.json"),
            2 * 1024 * 1024,
        )
        .map_err(|_| SetupError::Unavailable)?;
        if sha256(&canonical_json(&value).map_err(|_| SetupError::Invalid)?) != digest {
            return Err(SetupError::Invalid);
        }
        serde_json::from_value(value).map_err(|_| SetupError::Invalid)
    }
    pub fn withdraw(&self, job: &str) -> Result<()> {
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        s.connection
            .execute(
                "UPDATE app_report_publications SET withdrawn=1 WHERE app_id=?1 AND job_id=?2",
                params![self.app_id, job],
            )
            .map_err(db)?;
        Ok(())
    }
    /// Withdraw before deletion. Only known report identities are removed; never
    /// restore a catalog from directory contents. Unreferenced staging requires
    /// a full day of age and absence of a live analysis lease or signed intent.
    pub fn maintain(&self, limit: usize) -> Result<usize> {
        self.maintain_at(limit, now()?)
    }
    fn maintain_at(&self, limit: usize, at: i64) -> Result<usize> {
        if limit == 0 || limit > 10 {
            return Err(SetupError::Invalid);
        }
        let _guard = self
            .publication
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        self.queue.audit_receipts(&self.key)?;
        let records: Vec<(String, String, bool)> = {
            let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            s.connection.execute("UPDATE app_report_publications SET withdrawn=1 WHERE app_id=?1 AND expires_at<=?2",params![self.app_id,at]).map_err(db)?;
            let mut q=s.connection.prepare("SELECT job_id,report_id,withdrawn FROM app_report_publications WHERE app_id=?1 ORDER BY report_id LIMIT 10001").map_err(db)?;
            q.query_map([self.app_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(db)?
                .collect::<std::result::Result<_, _>>()
                .map_err(db)?
        };
        if records.len() > 10000 {
            return Err(SetupError::Capacity);
        }
        let mut removed = 0;
        for (job, id, withdrawn) in records {
            if !valid_secret(&id) {
                return Err(SetupError::Invalid);
            }
            let denied = match self.queue.intent(&job, &self.key) {
                Ok(_) => false,
                Err(SetupError::Denied) => true,
                Err(e) => return Err(e),
            };
            if !withdrawn && !denied {
                continue;
            }
            self.withdraw(&job)?;
            let path = self.root.join("reviews").join(id);
            if path.exists() || path.is_symlink() {
                safe_remove_report(&path)?;
                removed += 1;
                if removed == limit {
                    break;
                }
            }
        }
        if removed < limit {
            let entries = std::fs::read_dir(self.root.join("reviews"))
                .map_err(|_| SetupError::Unavailable)?;
            for entry in entries.take(MAX_REPORTS as usize + 1) {
                let entry = entry.map_err(|_| SetupError::Unavailable)?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !valid_secret(name) && !name.strip_prefix("pending-").is_some_and(valid_secret) {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .map_err(|_| SetupError::Unavailable)?
                    .modified()
                    .map_err(|_| SetupError::Unavailable)?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| SetupError::Invalid)?
                    .as_secs();
                if modified.saturating_add(86400) > at as u64 {
                    continue;
                }
                let path = entry.path();
                let job = if valid_secret(name) {
                    Some(self.manifest_job_for_cleanup(&path, name)?)
                } else {
                    None
                };
                if self.queue.report_is_recoverable(name, job.as_deref())? {
                    continue;
                }
                // A withdrawn catalog is already authoritative; unregistered bytes
                // never had public authority and cannot become a recovery source.
                safe_remove_report(&path)?;
                removed += 1;
                if removed == limit {
                    break;
                }
            }
        }
        Ok(removed)
    }
    fn manifest_job_for_cleanup(&self, path: &FsPath, id: &str) -> Result<String> {
        let m = verify_bundle(path).map_err(|_| SetupError::Invalid)?;
        if m.public_report_id != id {
            return Err(SetupError::Invalid);
        }
        Ok(m.analysis_review_id)
    }
    fn index(&self) -> Result<Vec<u8>> {
        let ids: Vec<String> = {
            let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            let mut q=s.connection.prepare("SELECT report_id FROM app_report_publications WHERE app_id=?1 AND withdrawn=0 AND expires_at>?2 ORDER BY report_id LIMIT 101").map_err(db)?;
            q.query_map(params![self.app_id, now()?], |r| r.get(0))
                .map_err(db)?
                .collect::<std::result::Result<_, _>>()
                .map_err(db)?
        };
        if ids.len() > MAX_REPORTS as usize {
            return Err(SetupError::Capacity);
        }
        let mut records = vec![];
        for id in ids {
            let m = match self.manifest(&id) {
                Ok(m) => m,
                Err(SetupError::Denied) => continue,
                Err(e) => return Err(e),
            };
            let bytes = canonical_json(&m).map_err(|_| SetupError::Invalid)?;
            records.push(serde_json::json!({"public_report_id":m.public_report_id,"analysis_review_id":m.analysis_review_id,"repository":m.repository,"pr_number":m.pr_number,"selected_baseline":m.selected_baseline,"baseline":m.baseline,"proposal":m.proposal,"scope_version":m.scope_version,"manifest":format!("reviews/{id}/manifest.json"),"manifest_sha256":sha256(&bytes),"manifest_bytes":bytes.len()}));
        }
        canonical_json(&serde_json::json!({"schema_version":"csb-public-index-v1","generated_at":utc(now()?),"reports":records})).map_err(|_|SetupError::Invalid)
    }
    fn file(&self, id: &str, path: &str) -> Result<(Vec<u8>, &'static str)> {
        let m = self.manifest(id)?;
        if path == "manifest.json" {
            return Ok((
                canonical_json(&m).map_err(|_| SetupError::Invalid)?,
                "application/json",
            ));
        }
        let item = m
            .files
            .iter()
            .find(|f| f.path == path)
            .ok_or(SetupError::Denied)?;
        let target = self.root.join("reviews").join(id).join(path);
        let bytes = bounded_file(&target, item.bytes)?;
        if bytes.len() != item.bytes || sha256(&bytes) != item.sha256 {
            return Err(SetupError::Invalid);
        }
        Ok((
            bytes,
            if item.kind == "json" {
                "application/json"
            } else {
                "text/plain; charset=utf-8"
            },
        ))
    }
}
fn safe_remove_report(path: &FsPath) -> Result<()> {
    // Validate all entries before removal; symlinks and special files are never traversed.
    if path.is_symlink() || !path.is_dir() {
        return Err(SetupError::Denied);
    }
    let mut pending = vec![path.to_path_buf()];
    let mut count = 0;
    let mut size = 0u64;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).map_err(|_| SetupError::Unavailable)? {
            let entry = entry.map_err(|_| SetupError::Unavailable)?;
            count += 1;
            if count > 2048 {
                return Err(SetupError::Capacity);
            }
            let kind = entry.file_type().map_err(|_| SetupError::Unavailable)?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                size = size
                    .saturating_add(entry.metadata().map_err(|_| SetupError::Unavailable)?.len());
            } else {
                return Err(SetupError::Denied);
            }
            if size > 8 * 1024 * 1024 {
                return Err(SetupError::Capacity);
            }
        }
    }
    std::fs::remove_dir_all(path).map_err(|_| SetupError::Unavailable)?;
    std::fs::File::open(path.parent().ok_or(SetupError::Invalid)?)
        .and_then(|f| f.sync_all())
        .map_err(|_| SetupError::Unavailable)
}
fn bounded_file(path: &FsPath, limit: usize) -> Result<Vec<u8>> {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        if matches!(part, std::path::Component::ParentDir) {
            return Err(SetupError::Denied);
        }
        prefix.push(part);
        if prefix.is_symlink() {
            return Err(SetupError::Denied);
        }
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| SetupError::Unavailable)?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err(SetupError::Capacity);
    }
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|_| SetupError::Unavailable)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SetupError::Unavailable)?;
    if bytes.len() > limit {
        return Err(SetupError::Capacity);
    }
    Ok(bytes)
}
fn utc(at: i64) -> String {
    let days = at.div_euclid(86400);
    let mut remaining = days;
    let mut year = 1970;
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    while remaining >= if leap(year) { 366 } else { 365 } {
        remaining -= if leap(year) { 366 } else { 365 };
        year += 1;
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
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
    let mut month = 0;
    while remaining >= months[month] {
        remaining -= months[month];
        month += 1;
    }
    format!(
        "{year:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        month + 1,
        remaining + 1,
        (at % 86400) / 3600,
        (at % 3600) / 60,
        at % 60
    )
}
fn response(bytes: Vec<u8>, kind: &'static str) -> Response {
    let mut r = Response::new(Body::from(bytes));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, kind.parse().unwrap());
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert("content-security-policy","default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; font-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'".parse().unwrap());
    r
}
fn denied() -> Response {
    (
        StatusCode::NOT_FOUND,
        "Report unavailable, expired or withdrawn",
    )
        .into_response()
}
pub fn app_report_router(store: Arc<AppReportStore>) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Redirect::temporary("/workspace/") }),
        )
        .route(
            "/workspace/",
            get(|| async { response(workspace_html().into_bytes(), "text/html; charset=utf-8") }),
        )
        .route(
            "/assets/{name}",
            get(|Path(name): Path<String>| async move {
                asset(&name)
                    .map(|(b, k)| response(b.to_vec(), k))
                    .unwrap_or_else(denied)
            }),
        )
        .route(
            "/index.json",
            get(|State(store): State<Arc<AppReportStore>>| async move {
                tokio::task::spawn_blocking(move || {
                    store
                        .index()
                        .map(|b| response(b, "application/json"))
                        .unwrap_or_else(|_| denied())
                })
                .await
                .unwrap_or_else(|_| denied())
            }),
        )
        .route(
            "/reviews/{id}/{*path}",
            get(
                |State(store): State<Arc<AppReportStore>>,
                 Path((id, path)): Path<(String, String)>| async move {
                    tokio::task::spawn_blocking(move || {
                        store
                            .file(&id, &path)
                            .map(|(b, k)| response(b, k))
                            .unwrap_or_else(|_| denied())
                    })
                    .await
                    .unwrap_or_else(|_| denied())
                },
            ),
        )
        .with_state(store)
}
fn workspace_html() -> String {
    include_str!("../workspace.html").replace("<meta name=\"color-scheme\" content=\"dark\">","<meta name=\"color-scheme\" content=\"dark\"><meta name=\"csb-public-static\" content=\"/index.json\">")
        .replace("<script src=\"/assets/workspace.js\"></script>","<script src=\"/assets/public-workspace.js\"></script><script src=\"/assets/workspace.js\"></script>")
        .replace("href=\"/workspace\"","href=\"/workspace/\"")
}
fn asset(name: &str) -> Option<(&'static [u8], &'static str)> {
    Some(match name {
        "workspace.js" => (include_bytes!("../workspace.js"), "text/javascript"),
        "public-workspace.js" => (include_bytes!("../public-workspace.js"), "text/javascript"),
        "workspace-preferences.js" => (
            include_bytes!("../workspace-preferences.js"),
            "text/javascript",
        ),
        "workspace-registry.json" => (
            include_bytes!("../workspace-registry.json"),
            "application/json",
        ),
        "workspace.css" => (include_bytes!("../workspace.css"), "text/css"),
        "mermaid.js" => (
            include_bytes!("../../docs/prototypes/vendor/mermaid.tiny-12.1.0.js"),
            "text/javascript",
        ),
        "inter.ttf" => (include_bytes!("../fonts/inter.ttf"), "font/ttf"),
        "jetbrains-mono.ttf" => (include_bytes!("../fonts/jetbrains-mono.ttf"), "font/ttf"),
        "inter-OFL.txt" => (include_bytes!("../fonts/inter-OFL.txt"), "text/plain"),
        "jetbrains-mono-OFL.txt" => (
            include_bytes!("../fonts/jetbrains-mono-OFL.txt"),
            "text/plain",
        ),
        "mermaid-LICENSE.txt" => (
            include_bytes!("../../docs/prototypes/vendor/mermaid-LICENSE.txt"),
            "text/plain",
        ),
        _ => return None,
    })
}
pub trait AppReportReadback: Send + Sync {
    fn readback(&self, receipt: &SignedAppReceipt) -> Result<PublicBundle>;
}
pub struct HttpsReportReadback {
    #[cfg(test)]
    test_base: Option<String>,
    origin: String,
    agent: ureq::Agent,
}
impl HttpsReportReadback {
    pub fn new(origin: String) -> Result<Self> {
        SetupConfig::new(1, "report".into(), origin.clone())?;
        Ok(Self {
            #[cfg(test)]
            test_base: None,
            origin,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .https_only(true)
                    .proxy(None)
                    .max_redirects(0)
                    .max_redirects_will_error(true)
                    .timeout_global(Some(std::time::Duration::from_secs(5)))
                    .max_response_header_size(16384)
                    .build(),
            ),
        })
    }
    fn get(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        #[cfg(test)]
        let base = self.test_base.as_ref().unwrap_or(&self.origin);
        #[cfg(not(test))]
        let base = &self.origin;
        let mut r = self
            .agent
            .get(format!("{base}{path}"))
            .header("Accept", "application/json, text/plain")
            .call()
            .map_err(|_| SetupError::Unavailable)?;
        if r.status().as_u16() != 200 || r.headers().contains_key("set-cookie") {
            return Err(SetupError::Denied);
        }
        let bytes = r
            .body_mut()
            .with_config()
            .limit(limit as u64 + 1)
            .read_to_vec()
            .map_err(|_| SetupError::Unavailable)?;
        if bytes.len() > limit {
            return Err(SetupError::Capacity);
        }
        Ok(bytes)
    }
}
impl AppReportReadback for HttpsReportReadback {
    fn readback(&self, receipt: &SignedAppReceipt) -> Result<PublicBundle> {
        if !receipt
            .focus_url()
            .starts_with(&format!("{}/workspace/#", self.origin))
            || !valid_secret(receipt.report_id())
        {
            return Err(SetupError::Denied);
        }
        let started = std::time::Instant::now();
        let prefix = format!("/reviews/{}/", receipt.report_id());
        let bytes = self.get(&format!("{prefix}manifest.json"), 2 * 1024 * 1024)?;
        if sha256(&bytes) != receipt.manifest_digest() {
            return Err(SetupError::Invalid);
        }
        let manifest: PublicManifest =
            serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
        if canonical_json(&manifest).map_err(|_| SetupError::Invalid)? != bytes
            || manifest.public_report_id != receipt.report_id()
            || manifest.files.len() >= 1000
            || manifest.files.iter().map(|f| f.bytes).sum::<usize>() + bytes.len() > 8 * 1024 * 1024
        {
            return Err(SetupError::Capacity);
        }
        // The browser discovers bundles through the catalog. Reachable chunks
        // alone do not establish that the check link can actually open a review.
        let index: serde_json::Value =
            serde_json::from_slice(&self.get("/index.json", 2 * 1024 * 1024)?)
                .map_err(|_| SetupError::Invalid)?;
        verify_index_entry(&index, &manifest, &bytes)?;
        let mut files = BTreeMap::new();
        for batch in manifest.files.chunks(8) {
            if started.elapsed() > std::time::Duration::from_secs(60) {
                return Err(SetupError::Unavailable);
            }
            let received = std::thread::scope(|scope| {
                let handles = batch
                    .iter()
                    .map(|f| {
                        scope.spawn(|| {
                            if f.path
                                .split('/')
                                .any(|p| p.is_empty() || matches!(p, "." | ".."))
                                || f.path.contains(['%', '?', '#', '\\', ':'])
                                || f.path.starts_with('/')
                            {
                                return Err(SetupError::Invalid);
                            }
                            let bytes = self.get(&format!("{prefix}{}", f.path), f.bytes)?;
                            if bytes.len() != f.bytes || sha256(&bytes) != f.sha256 {
                                return Err(SetupError::Invalid);
                            }
                            Ok((f.path.clone(), bytes))
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|h| h.join().map_err(|_| SetupError::Unavailable)?)
                    .collect::<Result<Vec<_>>>()
            })?;
            files.extend(received);
        }
        // Ensure the delivered UI bytes match this tested build, not just a reachable HTML page.
        if self.get("/workspace/", 512 * 1024)? != workspace_html().as_bytes() {
            return Err(SetupError::Invalid);
        }
        for name in [
            "workspace.js",
            "public-workspace.js",
            "workspace-preferences.js",
            "workspace-registry.json",
            "workspace.css",
            "mermaid.js",
            "inter.ttf",
            "jetbrains-mono.ttf",
        ] {
            let (expected, _) = asset(name).ok_or(SetupError::Invalid)?;
            if self.get(&format!("/assets/{name}"), 4 * 1024 * 1024)? != expected {
                return Err(SetupError::Invalid);
            }
        }
        let bundle = PublicBundle { manifest, files };
        bundle.validate().map_err(|_| SetupError::Invalid)?;
        Ok(bundle)
    }
}

fn verify_index_entry(index: &serde_json::Value, m: &PublicManifest, bytes: &[u8]) -> Result<()> {
    if index["schema_version"] != "csb-public-index-v1" {
        return Err(SetupError::Invalid);
    }
    let rows = index["reports"].as_array().ok_or(SetupError::Invalid)?;
    if rows.len() > MAX_REPORTS as usize {
        return Err(SetupError::Capacity);
    }
    let matches: Vec<_> = rows
        .iter()
        .filter(|r| r["public_report_id"] == m.public_report_id)
        .collect();
    let expected = serde_json::json!({"public_report_id":m.public_report_id,"analysis_review_id":m.analysis_review_id,"repository":m.repository,"pr_number":m.pr_number,"selected_baseline":m.selected_baseline,"baseline":m.baseline,"proposal":m.proposal,"scope_version":m.scope_version,"manifest":format!("reviews/{}/manifest.json", m.public_report_id),"manifest_sha256":sha256(bytes),"manifest_bytes":bytes.len()});
    if matches.len() != 1 || *matches[0] != expected {
        return Err(SetupError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::pipeline_tests::{Fixture, Provider, engine, signer};
    use super::*;
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;
    #[tokio::test]
    async fn routes_require_signed_catalog_and_immediately_fence_revocation_and_tamper() {
        let f = Fixture::new();
        let queue = f.queue();
        let provider = Provider(f.request.input.comparison.clone());
        let key = signer();
        let bundle = f.bundle();
        let store = Arc::new(
            AppReportStore::new(
                f.root.join("reports"),
                Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
                queue.clone(),
                key.public_key().to_vec(),
                1,
            )
            .unwrap(),
        );
        store.stage_bytes(&bundle).unwrap();
        let router = app_report_router(store.clone());
        let path = format!(
            "/reviews/{}/manifest.json",
            bundle.manifest.public_report_id
        );
        let read = |path: String| {
            router
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        };
        assert_eq!(
            read(path.clone()).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let id = queue.reconcile(2, 3, 7, &provider).unwrap().unwrap();
        let lease = queue.claim().unwrap().unwrap();
        queue
            .stage(
                &lease,
                &bundle,
                &engine(),
                "https://reports.example.test",
                &key,
                &provider,
            )
            .unwrap();
        store.expose(&id, &provider).unwrap();
        let catalog: Value = serde_json::from_slice(&store.index().unwrap()).unwrap();
        let manifest_bytes = canonical_json(&bundle.manifest).unwrap();
        verify_index_entry(&catalog, &bundle.manifest, &manifest_bytes).unwrap();
        let mut bad = catalog.clone();
        bad["reports"] = serde_json::json!([]);
        assert!(verify_index_entry(&bad, &bundle.manifest, &manifest_bytes).is_err());
        let mut bad = catalog.clone();
        bad["reports"][0]["proposal"] = serde_json::json!("a".repeat(40));
        assert!(verify_index_entry(&bad, &bundle.manifest, &manifest_bytes).is_err());
        let mut bad = catalog.clone();
        bad["reports"]
            .as_array_mut()
            .unwrap()
            .push(catalog["reports"][0].clone());
        assert!(verify_index_entry(&bad, &bundle.manifest, &manifest_bytes).is_err());
        // Restart recovery reads the same signed intent and never rewrites its catalog identity.
        let recovered = AppReportStore::new(
            f.root.join("reports"),
            Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
            queue.clone(),
            key.public_key().to_vec(),
            1,
        )
        .unwrap();
        recovered.expose(&id, &provider).unwrap();
        assert_eq!(
            recovered
                .file(&bundle.manifest.public_report_id, "manifest.json")
                .unwrap()
                .0,
            manifest_bytes
        );
        let result = read(path.clone()).await.unwrap();
        assert_eq!(result.status(), StatusCode::OK);
        assert!(!result.headers().contains_key("set-cookie"));
        assert_eq!(result.headers()["cache-control"], "no-store");
        assert_eq!(
            read(format!(
                "/reviews/{}/unknown.json",
                bundle.manifest.public_report_id
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::NOT_FOUND
        );
        let source = &bundle.manifest.routes.file_sources[0].path;
        std::fs::write(
            store
                .root
                .join("reviews")
                .join(&bundle.manifest.public_report_id)
                .join(source),
            b"tampered",
        )
        .unwrap();
        assert_eq!(
            read(format!(
                "/reviews/{}/{source}",
                bundle.manifest.public_report_id
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::NOT_FOUND
        );
        std::fs::write(
            store
                .root
                .join("reviews")
                .join(&bundle.manifest.public_report_id)
                .join(source),
            &bundle.files[source],
        )
        .unwrap();
        store.withdraw(&id).unwrap();
        assert_eq!(
            read(path.clone()).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert!(store.expose(&id, &provider).is_err());
        let c = rusqlite::Connection::open(f.root.join("state.db")).unwrap();
        assert!(
            c.execute("UPDATE app_report_publications SET withdrawn=0", [])
                .is_err()
        );
        let index = read("/index.json".into()).await.unwrap();
        let bytes = axum::body::to_bytes(index.into_body(), 100000)
            .await
            .unwrap();
        let index: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(index["reports"].as_array().unwrap().is_empty());
        assert_eq!(store.maintain(1).unwrap(), 1);
        assert!(
            !store
                .root
                .join("reviews")
                .join(&bundle.manifest.public_report_id)
                .exists()
        );
        assert!(store.expose(&id, &provider).is_err());
        let orphan = store
            .root
            .join("reviews")
            .join(format!("pending-{}", "e".repeat(64)));
        std::fs::create_dir(&orphan).unwrap();
        std::fs::write(orphan.join("partial.json"), b"{}").unwrap();
        std::fs::File::open(&orphan)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)),
            )
            .unwrap();
        assert_eq!(store.maintain(1).unwrap(), 1);
        assert!(!orphan.exists());
        assert_eq!(queue.audit_receipts(key.public_key()).unwrap(), 1);
        assert!(queue.audit_receipts(signer().public_key()).is_err());
        c.execute_batch("DROP TRIGGER app_intent_immutable; UPDATE app_report_intents SET receipt_digest=printf('%064d',0);").unwrap();
        assert!(queue.audit_receipts(key.public_key()).is_err());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_loopback_readback_checks_all_chunks_and_workspace_then_denies_revoked_scope() {
        let f = Fixture::new();
        let queue = f.queue();
        let provider = Provider(f.request.input.comparison.clone());
        let key = signer();
        let bundle = f.bundle();
        let store = Arc::new(
            AppReportStore::new(
                f.root.join("reports"),
                Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
                queue.clone(),
                key.public_key().to_vec(),
                1,
            )
            .unwrap(),
        );
        let id = queue.reconcile(2, 3, 7, &provider).unwrap().unwrap();
        let lease = queue.claim().unwrap().unwrap();
        store.stage_bytes(&bundle).unwrap();
        let receipt = queue
            .stage(
                &lease,
                &bundle,
                &engine(),
                "https://reports.example.test",
                &key,
                &provider,
            )
            .unwrap();
        store.expose(&id, &provider).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server =
            tokio::spawn(axum::serve(listener, app_report_router(store.clone())).into_future());
        // Only the test routes transport to local HTTP; production requires HTTPS.
        let reader = HttpsReportReadback {
            origin: "https://reports.example.test".into(),
            test_base: Some(format!("http://{address}")),
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .proxy(None)
                    .max_redirects(0)
                    .timeout_global(Some(std::time::Duration::from_secs(3)))
                    .build(),
            ),
        };
        let receipt_copy = receipt.clone();
        let verified = tokio::task::spawn_blocking(move || reader.readback(&receipt_copy))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(verified.files, bundle.files);
        queue
            .confirm_readback(&id, key.public_key(), &verified, &provider)
            .unwrap();
        assert_eq!(
            store
                .maintain_at(1, now().unwrap() + RETENTION + 1)
                .unwrap(),
            1
        );
        assert!(
            store
                .file(&bundle.manifest.public_report_id, "manifest.json")
                .is_err()
        );
        assert!(store.expose(&id, &provider).is_err());
        let c = rusqlite::Connection::open(f.root.join("state.db")).unwrap();
        c.execute(
            "INSERT INTO app_repository_blocks VALUES(1,2,3,'private',1)",
            [],
        )
        .unwrap();
        assert!(
            store
                .file(&bundle.manifest.public_report_id, "manifest.json")
                .is_err()
        );
        assert!(store.expose(&id, &provider).is_err());
        assert!(queue.intent(&id, key.public_key()).is_err());
        server.abort();
    }
    #[test]
    fn date_and_https_configuration_are_strict() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1709251200), "2024-03-01T00:00:00Z");
        for origin in [
            "http://reports.example.test",
            "https://user@host",
            "https://host/path",
        ] {
            assert!(HttpsReportReadback::new(origin.into()).is_err());
        }
        assert!(workspace_html().contains("csb-public-static"));
        assert!(asset("../app.sqlite").is_none());
    }
}
