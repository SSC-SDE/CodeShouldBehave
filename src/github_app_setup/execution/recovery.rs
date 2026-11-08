//! Exact offline snapshots, not signed-prefix rollback protection.
//!
//! All writers must remain stopped through snapshot capture, verification and copy.
//! Retain an anchor for the LATEST COMMITTED state independently of both DB and media.
//! Any later mutation invalidates that anchor's restore authority until recaptured.
//! The newest AVAILABLE backup is not necessarily the latest committed safety state.
//! External freshness/write fencing requires independent operator qualification;
//! a stale operator-trusted digest cannot prove that later revocations never existed.
//! Verification never opens Storage (which migrates), restores in place, or serves.
use super::*;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const ANCHOR_SCHEMA: &str = "csb-app-exact-recovery-v1";
const DATABASE: &str = "state.db";
const MANIFEST: &str = "artifacts.tsv";
const ANCHOR: &str = "anchor.json";
const ARTIFACTS: &str = "artifacts";
const MAX_ANCHOR: u64 = 65536;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppRecoverySnapshot {
    pub app_id: i64,
    pub created_at: i64,
    pub database_sha256: String,
    pub manifest_sha256: String,
    pub schema_version: i64,
    pub receipt_count: usize,
    pub receipt_tip: Option<String>,
    /// All columns of all App-scoped tables, including consent/revocation,
    /// publication tombstones, activation, lifetime reservations and attempt fences.
    pub app_state_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRecoveryAnchor {
    schema: String,
    snapshot: AppRecoverySnapshot,
    signature: String,
}
impl AppRecoveryAnchor {
    pub fn snapshot(&self) -> &AppRecoverySnapshot {
        &self.snapshot
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        canonical_json(self).map_err(|_| SetupError::Invalid)
    }
    /// Pin these exact bytes OUTSIDE the database and its backup storage.
    pub fn digest(&self) -> Result<String> {
        Ok(sha256(&self.bytes()?))
    }
    fn verify(&self, app: i64, key: &[u8]) -> Result<()> {
        if app <= 0 || self.schema != ANCHOR_SCHEMA || self.snapshot.app_id != app {
            return Err(SetupError::Denied);
        }
        let bytes = canonical_json(&(self.schema.as_str(), &self.snapshot))
            .map_err(|_| SetupError::Invalid)?;
        let signature = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|_| SetupError::Invalid)?;
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&bytes, &signature)
            .map_err(|_| SetupError::Denied)
    }
}

/// Evidence for this exact offline directory only. It is deliberately not a
/// serving/activation capability; normal fresh consent/provider fences still apply.
#[derive(Debug)]
pub struct VerifiedAppRecovery {
    pub directory: PathBuf,
    pub snapshot: AppRecoverySnapshot,
    pub artifact_count: usize,
}

pub struct AppOfflineRecovery;
impl AppOfflineRecovery {
    /// An anchor signature alone is insufficient: the caller MUST supply an
    /// independently retained digest covering the latest COMMITTED safety state,
    /// never a pin obtained from this backup or merely the latest available backup.
    /// Freshness cannot be established by this offline verifier; fail closed if
    /// any later mutation may exist outside the operator's independently fenced state.
    pub fn verify(
        directory: &Path,
        anchor_file: &Path,
        trusted_anchor_sha256: &str,
        app: i64,
        receipt_key: &[u8],
    ) -> Result<VerifiedAppRecovery> {
        if !valid_secret(trusted_anchor_sha256) {
            return Err(SetupError::Invalid);
        }
        let anchor_bytes = read_limited_regular(anchor_file, MAX_ANCHOR)?;
        if sha256(&anchor_bytes) != trusted_anchor_sha256 {
            return Err(SetupError::Denied);
        }
        let anchor: AppRecoveryAnchor =
            serde_json::from_slice(&anchor_bytes).map_err(|_| SetupError::Invalid)?;
        anchor.verify(app, receipt_key)?;
        no_symlinks(directory)?;
        if !fs::metadata(directory).map_err(io_error)?.is_dir() {
            return Err(SetupError::Invalid);
        }
        let database = directory.join(DATABASE);
        let manifest = directory.join(MANIFEST);
        let actual = inspect_snapshot(
            &database,
            &manifest,
            app,
            receipt_key,
            anchor.snapshot.created_at,
        )?;
        if actual != anchor.snapshot {
            return Err(SetupError::Denied);
        }
        let artifact_count =
            Storage::verify_backup(&database, &directory.join(ARTIFACTS), &manifest)
                .map_err(|_| SetupError::Invalid)?;
        // No WAL/journal may contribute hidden state, and verification must not
        // race an unquiesced writer. Immutable SQLite reads intentionally ignore WAL.
        reject_sidecars(&database)?;
        if file_digest(&database)? != anchor.snapshot.database_sha256
            || file_digest(&manifest)? != anchor.snapshot.manifest_sha256
        {
            return Err(SetupError::Conflict);
        }
        Ok(VerifiedAppRecovery {
            directory: directory.canonicalize().map_err(io_error)?,
            snapshot: actual,
            artifact_count,
        })
    }

    /// Verify, copy into a NEW directory only, then verify the copy. No overwrite,
    /// Storage::open, migration, host startup, grant changes or automatic serving.
    /// A failed copy remains non-authoritative and is never returned as verified.
    pub fn verify_and_copy(
        source: &Path,
        destination: &Path,
        anchor_file: &Path,
        trusted_anchor_sha256: &str,
        app: i64,
        receipt_key: &[u8],
    ) -> Result<VerifiedAppRecovery> {
        Self::verify(source, anchor_file, trusted_anchor_sha256, app, receipt_key)?;
        new_private_directory(destination)?;
        copy_regular(&source.join(DATABASE), &destination.join(DATABASE))?;
        copy_regular(&source.join(MANIFEST), &destination.join(MANIFEST))?;
        new_private_directory(&destination.join(ARTIFACTS))?;
        copy_manifest_artifacts(
            &source.join(ARTIFACTS),
            &destination.join(ARTIFACTS),
            &destination.join(MANIFEST),
        )?;
        sync_directory(destination)?;
        Self::verify(
            destination,
            anchor_file,
            trusted_anchor_sha256,
            app,
            receipt_key,
        )
    }
}

impl AppExecutionQueue {
    /// Only for an externally quiesced installation. The mutex serializes this
    /// process, NOT other processes or report/artifact writers. Retain the digest
    /// independently. Any later mutation invalidates this snapshot's authority
    /// to restore the latest committed safety state until a new anchor is captured.
    /// Referenced core artifacts are copied; report directories require separate
    /// immutable retention and still cannot be served without their normal checks.
    pub fn create_recovery_backup(
        &self,
        destination: &Path,
        artifact_root: &Path,
        signer: &AppReceiptSigner,
    ) -> Result<AppRecoveryAnchor> {
        new_private_directory(destination)?;
        let database = destination.join(DATABASE);
        let manifest = destination.join(MANIFEST);
        {
            let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            // Reuse the online backup primitive: hash/inspect the COMPLETED
            // consistent snapshot, never the independently changing live DB file.
            storage
                .create_consistent_backup(&database, &manifest)
                .map_err(|_| SetupError::Unavailable)?;
        }
        // Normalize ONLY the newly created backup to a standalone rollback-journal
        // database before hashing. No live source file is checkpointed or changed.
        {
            let backup = Connection::open(&database).map_err(db)?;
            backup
                .execute_batch("PRAGMA journal_mode=DELETE;")
                .map_err(db)?;
        }
        new_private_directory(&destination.join(ARTIFACTS))?;
        copy_manifest_artifacts(artifact_root, &destination.join(ARTIFACTS), &manifest)?;
        Storage::verify_backup(&database, &destination.join(ARTIFACTS), &manifest)
            .map_err(|_| SetupError::Invalid)?;
        // A recovery snapshot is standalone; hidden WAL state is never accepted.
        reject_sidecars(&database)?;
        let snapshot = inspect_snapshot(
            &database,
            &manifest,
            self.app_id,
            signer.public_key(),
            now()?,
        )?;
        let signature = URL_SAFE_NO_PAD.encode(
            signer
                .0
                .sign(
                    &canonical_json(&(ANCHOR_SCHEMA, &snapshot))
                        .map_err(|_| SetupError::Invalid)?,
                )
                .as_ref(),
        );
        let anchor = AppRecoveryAnchor {
            schema: ANCHOR_SCHEMA.into(),
            snapshot,
            signature,
        };
        write_new(&destination.join(ANCHOR), &anchor.bytes()?)?;
        sync_directory(destination)?;
        Ok(anchor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRecoveryDecision {
    Abandon,
    ReconcileProgress { check_id: i64 },
    ReconcileCompleted { check_id: i64 },
}
pub struct AppRecoveryRequest<'a> {
    pub job_id: &'a str,
    pub actor: &'a str,
    pub reason: &'a str,
    pub decision: AppRecoveryDecision,
}
pub struct AppRecoveryEvidence<'a> {
    pub receipt_key: &'a [u8],
    pub readback: Option<&'a PublicBundle>,
    /// Must reserve the existing non-refundable operator request budget before IO.
    pub provider: &'a dyn AppExecutionProvider,
    pub checks: &'a dyn AppCheckDelivery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRecoveryOutcome {
    Abandoned,
    ReconciledProgress { check_id: i64 },
    ReconciledCompleted { check_id: i64 },
}

impl AppExecutionQueue {
    /// Pause dispatch and stop all workers first. Operator evidence uses the SAME
    /// budgeted, App-owned exact-payload lookup as normal delivery. This API NEVER
    /// calls create/update, resets accounting, or grants another attempt.
    pub fn recover_job(
        &self,
        request: &AppRecoveryRequest<'_>,
        evidence: Option<&AppRecoveryEvidence<'_>>,
    ) -> Result<AppRecoveryOutcome> {
        if !valid_secret(request.job_id)
            || !valid_operator_text(request.actor, 128)
            || !valid_operator_text(request.reason, 1024)
        {
            return Err(SetupError::Invalid);
        }
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let row: Option<(String, String, i64, i64, bool, Option<i64>)> = tx.query_row(
            "SELECT input,state,attempts,delivery_attempts,delivery_attempted,check_id FROM app_analysis_jobs WHERE id=?1 AND app_id=?2",
            params![request.job_id, self.app_id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional().map_err(db)?;
        let (raw, state, attempts, delivery_attempts, create_attempted, known_check) =
            row.ok_or(SetupError::Denied)?;
        if state == "delivered"
            || state == "cancelled"
            || !(state == "failed"
                || attempts >= 3
                || delivery_attempts >= 3
                || (create_attempted && known_check.is_none()))
        {
            return Err(SetupError::Conflict);
        }
        let input: AnalysisInput = serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
        if input.id()? != request.job_id || input.app_id != self.app_id {
            return Err(SetupError::Denied);
        }
        let (decision, check_id, outcome) = match request.decision {
            AppRecoveryDecision::Abandon => {
                // Even revoked work may be abandoned. No credentials are needed.
                tx.execute("UPDATE app_analysis_jobs SET state='cancelled',lease=NULL,lease_until=NULL WHERE id=?1 AND app_id=?2",
                    params![request.job_id,self.app_id]).map_err(db)?;
                tx.execute(
                    "UPDATE app_report_publications SET withdrawn=1 WHERE job_id=?1 AND app_id=?2",
                    params![request.job_id, self.app_id],
                )
                .map_err(db)?;
                ("abandoned", None, AppRecoveryOutcome::Abandoned)
            }
            decision => {
                let evidence = evidence.ok_or(SetupError::Invalid)?;
                let expected = match decision {
                    AppRecoveryDecision::ReconcileProgress { check_id }
                    | AppRecoveryDecision::ReconcileCompleted { check_id } => check_id,
                    AppRecoveryDecision::Abandon => unreachable!(),
                };
                if expected <= 0
                    || !create_attempted
                    || known_check.is_some_and(|id| id != expected)
                {
                    return Err(SetupError::Conflict);
                }
                current_grant(&tx, &input)?;
                recheck(&input, evidence.provider)?;
                let (payload, completed) = match decision {
                    AppRecoveryDecision::ReconcileProgress { .. } => {
                        let raw: String = tx.query_row(
                            "SELECT create_payload FROM app_check_progress WHERE job_id=?1 AND create_payload IS NOT NULL",
                            [request.job_id], |r| r.get(0),
                        ).optional().map_err(db)?.ok_or(SetupError::Conflict)?;
                        if raw.len() > 16384 {
                            return Err(SetupError::Invalid);
                        }
                        let payload: Value =
                            serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
                        if payload["name"] != "CodeShouldBehave / Architecture"
                            || payload["head_sha"] != input.comparison.head
                            || payload["external_id"] != format!("csb-app-v1:{}", request.job_id)
                        {
                            return Err(SetupError::Denied);
                        }
                        (payload, false)
                    }
                    AppRecoveryDecision::ReconcileCompleted { .. } => {
                        let readback = evidence.readback.ok_or(SetupError::Invalid)?;
                        readback.validate().map_err(|_| SetupError::Invalid)?;
                        let raw: String = tx
                            .query_row(
                                "SELECT receipt FROM app_report_intents WHERE job_id=?1",
                                [request.job_id],
                                |r| r.get(0),
                            )
                            .optional()
                            .map_err(db)?
                            .ok_or(SetupError::Denied)?;
                        let signed: SignedAppReceipt =
                            serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
                        signed.verify(evidence.receipt_key)?;
                        if signed.receipt.job != input
                            || sha256(
                                &canonical_json(&readback.manifest)
                                    .map_err(|_| SetupError::Invalid)?,
                            ) != signed.receipt.bundle_digest
                        {
                            return Err(SetupError::Denied);
                        }
                        let published: bool = tx.query_row(
                            "SELECT EXISTS(SELECT 1 FROM app_report_publications WHERE app_id=?1 AND job_id=?2 AND report_id=?3 AND manifest_digest=?4 AND withdrawn=0 AND expires_at>?5)",
                            params![self.app_id,request.job_id,signed.receipt.report_id,signed.receipt.bundle_digest,now()?],
                            |r| r.get(0),
                        ).map_err(db)?;
                        if !published {
                            return Err(SetupError::Denied);
                        }
                        (completed_check_payload(request.job_id, &signed), true)
                    }
                    AppRecoveryDecision::Abandon => unreachable!(),
                };
                let found =
                    evidence
                        .checks
                        .find(input.repository_id, &input.comparison.head, &payload)?;
                if found != Some(expected) {
                    return Err(SetupError::Conflict);
                }
                // find is read-only. A missing/foreign/mismatched check can NEVER
                // cause a replacement POST, even after all attempts are exhausted.
                tx.execute(
                    "UPDATE app_analysis_jobs SET check_id=?2,state=CASE WHEN ?3 THEN 'delivered' ELSE state END WHERE id=?1 AND app_id=?4",
                    params![request.job_id,expected,completed,self.app_id],
                ).map_err(db)?;
                if completed {
                    (
                        "reconciled",
                        Some(expected),
                        AppRecoveryOutcome::ReconciledCompleted { check_id: expected },
                    )
                } else {
                    (
                        "reconciled_progress",
                        Some(expected),
                        AppRecoveryOutcome::ReconciledProgress { check_id: expected },
                    )
                }
            }
        };
        tx.execute(
            "INSERT INTO app_recovery_decisions(job_id,app_id,decision,check_id,actor,reason,created_at,analysis_attempts,delivery_attempts,create_attempted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![request.job_id,self.app_id,decision,check_id,request.actor,request.reason,now()?,attempts,delivery_attempts,create_attempted],
        ).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(outcome)
    }
}
fn valid_operator_text(text: &str, maximum: usize) -> bool {
    !text.trim().is_empty() && text.len() <= maximum && !text.chars().any(char::is_control)
}

fn inspect_snapshot(
    database: &Path,
    manifest: &Path,
    app: i64,
    key: &[u8],
    created_at: i64,
) -> Result<AppRecoverySnapshot> {
    let database_sha256 = file_digest(database)?;
    let manifest_sha256 = file_digest(manifest)?;
    let connection = immutable_connection(database)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(db)?;
    if integrity != "ok" {
        return Err(SetupError::Invalid);
    }
    let foreign_key_error = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(db)?
        .exists([])
        .map_err(db)?;
    if foreign_key_error {
        return Err(SetupError::Invalid);
    }
    // A pre-recovery schema is not qualified merely because it has signed receipts.
    let recovery_schema: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='app_recovery_decisions')",
        [], |r| r.get(0)).map_err(db)?;
    if !recovery_schema {
        return Err(SetupError::Invalid);
    }
    let schema_version = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db)?;
    let (receipt_count, receipt_tip) = audit_snapshot_receipts(&connection, app, key)?;
    let app_state_sha256 = app_state_digest(&connection, app)?;
    drop(connection);
    reject_sidecars(database)?;
    if file_digest(database)? != database_sha256 || file_digest(manifest)? != manifest_sha256 {
        return Err(SetupError::Conflict);
    }
    Ok(AppRecoverySnapshot {
        app_id: app,
        created_at,
        database_sha256,
        manifest_sha256,
        schema_version,
        receipt_count,
        receipt_tip,
        app_state_sha256,
    })
}

fn audit_snapshot_receipts(
    c: &Connection,
    app: i64,
    key: &[u8],
) -> Result<(usize, Option<String>)> {
    let mut q = c.prepare("SELECT i.job_id,i.receipt,i.receipt_digest,i.predecessor,j.input FROM app_report_intents i JOIN app_analysis_jobs j ON j.id=i.job_id WHERE j.app_id=?1 ORDER BY i.rowid LIMIT 10001").map_err(db)?;
    let mut rows = q.query([app]).map_err(db)?;
    let mut previous = None;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(db)? {
        count += 1;
        let id: String = row.get(0).map_err(db)?;
        let raw: String = row.get(1).map_err(db)?;
        let hash: String = row.get(2).map_err(db)?;
        let predecessor: Option<String> = row.get(3).map_err(db)?;
        let input: String = row.get(4).map_err(db)?;
        if count > 10000
            || raw.len() > 16384
            || sha256(raw.as_bytes()) != hash
            || predecessor != previous
        {
            return Err(SetupError::Invalid);
        }
        let signed: SignedAppReceipt =
            serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
        signed.verify(key)?;
        let input: AnalysisInput = serde_json::from_str(&input).map_err(|_| SetupError::Invalid)?;
        if signed.receipt.job != input
            || signed.receipt.job.app_id != app
            || signed.receipt.job.id()? != id
            || signed.receipt.previous != previous
        {
            return Err(SetupError::Denied);
        }
        previous = Some(hash);
    }
    Ok((count, previous))
}

/// Include future App-scoped tables automatically. The full DB byte hash also
/// binds schema, other Apps, and non-App state; this digest makes safety coverage
/// explicit without pretending a receipt prefix proves monotonicity.
fn app_state_digest(c: &Connection, app: i64) -> Result<String> {
    let mut tables = c
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name GLOB 'app_*' ORDER BY name",
        )
        .map_err(db)?;
    let names = tables
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db)?;
    let mut hash = Sha256::new();
    hash.update(b"csb-app-safety-state-v1\0");
    for name in names {
        if !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(SetupError::Invalid);
        }
        let mut columns = c
            .prepare(&format!("PRAGMA table_info({name})"))
            .map_err(db)?;
        let columns = columns
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        if !columns.iter().any(|n| n == "app_id") {
            // Receipt rows have no app_id and are separately chain-audited.
            continue;
        }
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        let ordering = (1..=columns.len())
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut q = c
            .prepare(&format!(
                "SELECT * FROM {name} WHERE app_id=?1 ORDER BY {ordering}"
            ))
            .map_err(db)?;
        let mut rows = q.query([app]).map_err(db)?;
        while let Some(row) = rows.next().map_err(db)? {
            hash.update(b"row\0");
            for index in 0..columns.len() {
                match row.get_ref(index).map_err(db)? {
                    ValueRef::Null => hash.update(b"n"),
                    ValueRef::Integer(n) => {
                        hash.update(b"i");
                        hash.update(n.to_be_bytes());
                    }
                    ValueRef::Real(n) => {
                        hash.update(b"r");
                        hash.update(n.to_bits().to_be_bytes());
                    }
                    value @ (ValueRef::Text(bytes) | ValueRef::Blob(bytes)) => {
                        hash.update(if matches!(value, ValueRef::Text(_)) {
                            b"t"
                        } else {
                            b"b"
                        });
                        hash.update((bytes.len() as u64).to_be_bytes());
                        hash.update(bytes);
                    }
                }
            }
        }
        hash.update(b"end\0");
    }
    Ok(finalize_hex(hash))
}

fn immutable_connection(path: &Path) -> Result<Connection> {
    no_symlinks(path)?;
    reject_sidecars(path)?;
    let absolute = path.canonicalize().map_err(io_error)?;
    let text = absolute.to_str().ok_or(SetupError::Invalid)?;
    let mut uri = String::from("file:");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            uri.push(byte as char);
        } else {
            use std::fmt::Write as _;
            write!(&mut uri, "%{byte:02X}").map_err(|_| SetupError::Invalid)?;
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(db)
}
fn reject_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match fs::symlink_metadata(PathBuf::from(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(SetupError::Conflict),
        }
    }
    Ok(())
}
fn no_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if fs::symlink_metadata(ancestor)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
        {
            return Err(SetupError::Invalid);
        }
    }
    Ok(())
}
fn file_digest(path: &Path) -> Result<String> {
    no_symlinks(path)?;
    let mut file = File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(SetupError::Invalid);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(finalize_hex(hash))
}
fn finalize_hex(hash: Sha256) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in hash.finalize() {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}
fn read_limited_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    no_symlinks(path)?;
    let file = File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(SetupError::Invalid);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(SetupError::Capacity);
    }
    Ok(bytes)
}
fn new_private_directory(path: &Path) -> Result<()> {
    no_symlinks(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(io_error)
}
fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(io_error)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = new_file(path)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}
fn copy_regular(source: &Path, destination: &Path) -> Result<()> {
    no_symlinks(source)?;
    let mut source = File::open(source).map_err(io_error)?;
    if !source.metadata().map_err(io_error)?.is_file() {
        return Err(SetupError::Invalid);
    }
    let mut destination = new_file(destination)?;
    std::io::copy(&mut source, &mut destination).map_err(io_error)?;
    destination.sync_all().map_err(io_error)
}
fn copy_manifest_artifacts(source: &Path, destination: &Path, manifest: &Path) -> Result<()> {
    let text = fs::read_to_string(manifest).map_err(io_error)?;
    let mut lines = text.lines();
    if lines.next() != Some("codeshouldbehave-backup-manifest-v1") {
        return Err(SetupError::Invalid);
    }
    for line in lines {
        let parts: Vec<_> = line.split('\t').collect();
        if parts.len() != 4 {
            return Err(SetupError::Invalid);
        }
        let relative = Path::new(parts[3]);
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err(SetupError::Invalid);
        }
        let target = destination.join(relative);
        let parent = target.parent().ok_or(SetupError::Invalid)?;
        // The destination is exclusively newly created by this operation.
        fs::create_dir_all(parent).map_err(io_error)?;
        no_symlinks(parent)?;
        copy_regular(&source.join(relative), &target)?;
        sync_directory(parent)?;
    }
    sync_directory(destination)
}
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io_error)
}
fn io_error(_: std::io::Error) -> SetupError {
    SetupError::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct OfflineFixture {
        root: PathBuf,
        queue: AppExecutionQueue,
        signer: AppReceiptSigner,
        id: String,
    }
    impl OfflineFixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("csb-exact-recovery-{}", secret().unwrap()));
            fs::create_dir(&root).unwrap();
            let storage = Storage::open(&root.join("source.db"), &root.join(ARTIFACTS)).unwrap();
            let input = AnalysisInput {
                schema: "csb-app-analysis-v1".into(),
                app_id: 1,
                installation_id: 2,
                repository_id: 3,
                scope_revision: 1,
                consent_digest: "a".repeat(64),
                engine_digest: "b".repeat(64),
                engine_metadata_digest: "c".repeat(64),
                comparison: CurrentPullRequest {
                    repository_id: 3,
                    number: 7,
                    selected_base: "a".repeat(40),
                    merge_base: "a".repeat(40),
                    head: "b".repeat(40),
                    open: true,
                    draft: false,
                },
            };
            let id = input.id().unwrap();
            storage.connection.execute(
                "INSERT INTO app_analysis_jobs(id,app_id,installation_id,repository_id,pr_number,consent_digest,input,state,attempts,delivery_attempts,delivery_attempted,created_at,available_at) VALUES(?1,1,2,3,7,?2,?3,'failed',3,3,1,1,1)",
                params![id,input.consent_digest,serde_json::to_string(&input).unwrap()],
            ).unwrap();
            storage
                .connection
                .execute(
                    "INSERT INTO app_request_budgets VALUES(1,'job:2:fixture',77)",
                    [],
                )
                .unwrap();
            storage.connection.execute("INSERT INTO app_publication_consents(app_id,installation_id,repository_id,revision,actor_id,created_at,document,digest) VALUES(1,2,3,1,4,1,'{}',?1)", ["a".repeat(64)]).unwrap();
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
            let signer = AppReceiptSigner::from_pkcs8(pkcs8.as_ref()).unwrap();
            let signed = signer
                .sign(Receipt {
                    schema: "csb-app-receipt-v1".into(),
                    job: input,
                    bundle_digest: "d".repeat(64),
                    report_id: "e".repeat(64),
                    focus_url: "https://reports.example/workspace/".into(),
                    previous: None,
                    created_at: 1,
                    check: CheckOutcome {
                        conclusion: "neutral".into(),
                        title: "Review".into(),
                    },
                })
                .unwrap();
            let raw = serde_json::to_string(&signed).unwrap();
            storage.connection.execute(
                "INSERT INTO app_report_intents(job_id,receipt,receipt_digest,created_at) VALUES(?1,?2,?3,1)",
                params![id,raw,sha256(raw.as_bytes())],
            ).unwrap();
            storage.connection.execute(
                "INSERT INTO app_report_publications(app_id,job_id,report_id,manifest_digest,expires_at) VALUES(1,?1,?2,?3,9999999999)",
                params![id,"e".repeat(64),"d".repeat(64)],
            ).unwrap();
            Self {
                root,
                queue: AppExecutionQueue::new(storage, 1).unwrap(),
                signer,
                id,
            }
        }
        fn backup(&self, name: &str) -> (PathBuf, PathBuf, AppRecoveryAnchor) {
            let directory = self.root.join(name);
            let anchor = self
                .queue
                .create_recovery_backup(&directory, &self.root.join(ARTIFACTS), &self.signer)
                .unwrap();
            let trusted = self.root.join(format!("independent-{name}.json"));
            write_new(&trusted, &anchor.bytes().unwrap()).unwrap();
            (directory, trusted, anchor)
        }
    }
    impl Drop for OfflineFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn exact_anchor_rejects_signed_prefix_rollback_stale_consent_and_refunds() {
        let f = OfflineFixture::new();
        let (old, _, old_anchor) = f.backup("old");
        {
            let s = f.queue.storage.lock().unwrap();
            s.connection
                .execute("UPDATE app_publication_consents SET revoked=1", [])
                .unwrap();
            s.connection
                .execute("UPDATE app_request_budgets SET used=used+9", [])
                .unwrap();
            s.connection
                .execute("UPDATE app_report_publications SET withdrawn=1", [])
                .unwrap();
        }
        let (latest, trusted, anchor) = f.backup("latest");
        // The receipt chain is IDENTICAL: a prefix-only audit would miss every
        // revocation, withdrawal and consumed budget in this rollback.
        assert_eq!(old_anchor.snapshot.receipt_tip, anchor.snapshot.receipt_tip);
        assert_ne!(
            old_anchor.snapshot.app_state_sha256,
            anchor.snapshot.app_state_sha256
        );
        let pin = anchor.digest().unwrap();
        assert!(
            AppOfflineRecovery::verify(&latest, &trusted, &pin, 1, f.signer.public_key()).is_ok()
        );
        assert!(
            AppOfflineRecovery::verify(&old, &trusted, &pin, 1, f.signer.public_key()).is_err()
        );
        let copied = f.root.join("restored-new");
        AppOfflineRecovery::verify_and_copy(
            &latest,
            &copied,
            &trusted,
            &pin,
            1,
            f.signer.public_key(),
        )
        .unwrap();
        let c = immutable_connection(&copied.join(DATABASE)).unwrap();
        assert_eq!(
            c.query_row("SELECT revoked FROM app_publication_consents", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT withdrawn FROM app_report_publications", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT used FROM app_request_budgets", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            86
        );
        assert_eq!(
            c.query_row(
                "SELECT attempts,delivery_attempts,delivery_attempted FROM app_analysis_jobs",
                [],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?
                ))
            )
            .unwrap(),
            (3, 3, 1)
        );
        drop(c);
        assert!(
            AppOfflineRecovery::verify_and_copy(
                &latest,
                &copied,
                &trusted,
                &pin,
                1,
                f.signer.public_key()
            )
            .is_err()
        );
        // Ordinary valid SQL tampering is still tampering of the exact snapshot.
        let c = Connection::open(copied.join(DATABASE)).unwrap();
        c.execute("UPDATE app_request_budgets SET used=used+1", [])
            .unwrap();
        drop(c);
        assert!(
            AppOfflineRecovery::verify(&copied, &trusted, &pin, 1, f.signer.public_key()).is_err()
        );
    }

    #[test]
    fn external_pin_app_key_manifest_and_sidecars_are_mandatory_boundaries() {
        let f = OfflineFixture::new();
        let (backup, trusted, anchor) = f.backup("backup");
        let pin = anchor.digest().unwrap();
        assert!(
            AppOfflineRecovery::verify(
                &backup,
                &trusted,
                &"f".repeat(64),
                1,
                f.signer.public_key()
            )
            .is_err()
        );
        assert!(
            AppOfflineRecovery::verify(&backup, &trusted, &pin, 2, f.signer.public_key()).is_err()
        );
        assert!(AppOfflineRecovery::verify(&backup, &trusted, &pin, 1, &[0; 32]).is_err());
        write_new(&backup.join("state.db-wal"), b"").unwrap();
        assert!(
            AppOfflineRecovery::verify(&backup, &trusted, &pin, 1, f.signer.public_key()).is_err()
        );
        fs::remove_file(backup.join("state.db-wal")).unwrap();
        OpenOptions::new()
            .append(true)
            .open(backup.join(MANIFEST))
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert!(
            AppOfflineRecovery::verify(&backup, &trusted, &pin, 1, f.signer.public_key()).is_err()
        );
    }

    #[test]
    fn exhausted_abandon_is_terminal_and_preserves_lifetime_history_after_restart() {
        let f = OfflineFixture::new();
        let request = AppRecoveryRequest {
            job_id: &f.id,
            actor: "offline-operator",
            reason: "Explicitly abandon exhausted identity",
            decision: AppRecoveryDecision::Abandon,
        };
        assert_eq!(
            f.queue.recover_job(&request, None).unwrap(),
            AppRecoveryOutcome::Abandoned
        );
        let s = f.queue.storage.lock().unwrap();
        let row = s
            .connection
            .query_row(
                "SELECT state,attempts,delivery_attempts,delivery_attempted FROM app_analysis_jobs",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, ("cancelled".into(), 3, 3, 1));
        for sql in [
            "UPDATE app_analysis_jobs SET state='pending'",
            "UPDATE app_analysis_jobs SET attempts=0",
            "UPDATE app_analysis_jobs SET delivery_attempts=0",
            "UPDATE app_analysis_jobs SET delivery_attempted=0",
            "DELETE FROM app_analysis_jobs",
            "UPDATE app_request_budgets SET used=0",
            "DELETE FROM app_request_budgets",
            "DELETE FROM app_publication_consents",
            "UPDATE app_report_publications SET withdrawn=0",
            "DELETE FROM app_report_publications",
            "DELETE FROM app_recovery_decisions",
        ] {
            assert!(s.connection.execute(sql, []).is_err(), "{sql}");
        }
        drop(s);
        let restarted = AppExecutionQueue::new(
            Storage::open(&f.root.join("source.db"), &f.root.join(ARTIFACTS)).unwrap(),
            1,
        )
        .unwrap();
        assert_eq!(
            restarted.recover_job(&request, None),
            Err(SetupError::Conflict)
        );
    }

    struct Provider(CurrentPullRequest);
    impl AppExecutionProvider for Provider {
        fn current_pr(
            &self,
            installation: i64,
            repository: i64,
            pr: i64,
        ) -> Result<CurrentPullRequest> {
            assert_eq!((installation, repository, pr), (2, 3, 7));
            Ok(self.0.clone())
        }
    }
    struct FindOnly {
        expected: Value,
        found: Cell<Option<i64>>,
        lookups: Cell<usize>,
    }
    impl AppCheckDelivery for FindOnly {
        fn find(&self, repository: i64, head: &str, payload: &Value) -> Result<Option<i64>> {
            assert_eq!(repository, 3);
            assert_eq!(payload["head_sha"], head);
            assert_eq!(payload, &self.expected);
            self.lookups.set(self.lookups.get() + 1);
            Ok(self.found.get())
        }
        fn create(&self, _: i64, _: &Value) -> Result<i64> {
            panic!("operator recovery must NEVER create a check")
        }
        fn update(&self, _: i64, _: i64, _: &Value) -> Result<i64> {
            panic!("operator recovery must NEVER mutate an unknown check")
        }
    }

    #[test]
    fn unknown_progress_only_reconciles_exact_existing_id_without_reopening_exhaustion() {
        let f = crate::github_app_setup::pipeline_tests::Fixture::new();
        let queue = f.queue();
        let provider = Provider(f.request.input.comparison.clone());
        let id = queue.reconcile(2, 3, 7, &provider).unwrap().unwrap();
        let payload = json!({"name":"CodeShouldBehave / Architecture","head_sha":provider.0.head,
            "external_id":format!("csb-app-v1:{id}"),"status":"queued"});
        {
            let s = queue.storage.lock().unwrap();
            s.connection.execute("UPDATE app_analysis_jobs SET state='failed',delivery_attempts=3,delivery_attempted=1", []).unwrap();
            s.connection.execute(
                "INSERT INTO app_check_progress(job_id,create_payload,desired_digest,attempts,next_at) VALUES(?1,?2,?3,3,0)",
                params![id,serde_json::to_string(&payload).unwrap(),sha256(&canonical_json(&payload).unwrap())],
            ).unwrap();
        }
        let writer = FindOnly {
            expected: payload,
            found: Cell::new(None),
            lookups: Cell::new(0),
        };
        let evidence = AppRecoveryEvidence {
            receipt_key: &[],
            readback: None,
            provider: &provider,
            checks: &writer,
        };
        let request = AppRecoveryRequest {
            job_id: &id,
            actor: "operator",
            reason: "Reconcile ambiguous POST only",
            decision: AppRecoveryDecision::ReconcileProgress { check_id: 42 },
        };
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)),
            Err(SetupError::Conflict)
        );
        writer.found.set(Some(43));
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)),
            Err(SetupError::Conflict)
        );
        writer.found.set(Some(42));
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)).unwrap(),
            AppRecoveryOutcome::ReconciledProgress { check_id: 42 }
        );
        let s = queue.storage.lock().unwrap();
        assert_eq!(s.connection.query_row(
            "SELECT state,check_id,delivery_attempted,delivery_attempts FROM app_analysis_jobs",
            [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)),
        ).unwrap(), ("failed".into(),42,1,3));
        assert_eq!(writer.lookups.get(), 3);
    }

    #[test]
    fn completed_recovery_requires_full_readback_and_never_creates_when_lookup_is_missing() {
        let f = crate::github_app_setup::pipeline_tests::Fixture::new();
        let queue = f.queue();
        let provider = Provider(f.request.input.comparison.clone());
        let id = queue.reconcile(2, 3, 7, &provider).unwrap().unwrap();
        let lease = queue.claim().unwrap().unwrap();
        let bundle = f.bundle();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let signer = AppReceiptSigner::from_pkcs8(pkcs8.as_ref()).unwrap();
        let signed = queue
            .stage(
                &lease,
                &bundle,
                &bundle.manifest.engine,
                "https://reports.example.test",
                &signer,
                &provider,
            )
            .unwrap();
        queue
            .confirm_readback(&id, signer.public_key(), &bundle, &provider)
            .unwrap();
        {
            let s = queue.storage.lock().unwrap();
            s.connection.execute(
                "INSERT INTO app_report_publications(app_id,job_id,report_id,manifest_digest,expires_at) VALUES(1,?1,?2,?3,?4)",
                params![id,signed.report_id(),signed.manifest_digest(),now().unwrap()+600],
            ).unwrap();
            s.connection.execute("UPDATE app_analysis_jobs SET state='failed',delivery_attempts=3,delivery_attempted=1", []).unwrap();
        }
        let writer = FindOnly {
            expected: completed_check_payload(&id, &signed),
            found: Cell::new(None),
            lookups: Cell::new(0),
        };
        let mut evidence = AppRecoveryEvidence {
            receipt_key: signer.public_key(),
            readback: None,
            provider: &provider,
            checks: &writer,
        };
        let request = AppRecoveryRequest {
            job_id: &id,
            actor: "operator",
            reason: "Retain existing exact completed check",
            decision: AppRecoveryDecision::ReconcileCompleted { check_id: 42 },
        };
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)),
            Err(SetupError::Invalid)
        );
        assert_eq!(writer.lookups.get(), 0);
        evidence.readback = Some(&bundle);
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)),
            Err(SetupError::Conflict)
        );
        writer.found.set(Some(42));
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)).unwrap(),
            AppRecoveryOutcome::ReconciledCompleted { check_id: 42 }
        );
        let s = queue.storage.lock().unwrap();
        assert_eq!(s.connection.query_row(
            "SELECT state,check_id,attempts,delivery_attempts,delivery_attempted FROM app_analysis_jobs",
            [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?)),
        ).unwrap(), ("delivered".into(),42,1,3,1));
    }

    #[test]
    fn referenced_artifacts_are_copied_and_verified_not_just_manifest_checked() {
        let f = OfflineFixture::new();
        let artifact = {
            let mut s = f.queue.storage.lock().unwrap();
            let artifact = s
                .publish_artifact("test-domain", "graph-v1", b"exact retained bytes")
                .unwrap();
            s.connection.execute(
                "INSERT INTO artifact_links(artifact_id,owner_kind,owner_id,purpose) VALUES(?1,'review','fixture','graph')",
                [artifact.id],
            ).unwrap();
            artifact
        };
        let (backup, trusted, anchor) = f.backup("with-artifact");
        let pin = anchor.digest().unwrap();
        let verified =
            AppOfflineRecovery::verify(&backup, &trusted, &pin, 1, f.signer.public_key()).unwrap();
        assert_eq!(verified.artifact_count, 1);
        let copied = f.root.join("with-artifact-copy");
        AppOfflineRecovery::verify_and_copy(
            &backup,
            &copied,
            &trusted,
            &pin,
            1,
            f.signer.public_key(),
        )
        .unwrap();
        let copied_artifact = copied.join(ARTIFACTS).join(artifact.relative_path);
        assert_eq!(fs::read(&copied_artifact).unwrap(), b"exact retained bytes");
        fs::write(copied_artifact, b"tampered retained bytes").unwrap();
        assert!(
            AppOfflineRecovery::verify(&copied, &trusted, &pin, 1, f.signer.public_key()).is_err()
        );
    }

    #[test]
    fn revoked_consent_blocks_even_read_only_unknown_check_reconciliation() {
        let f = crate::github_app_setup::pipeline_tests::Fixture::new();
        let queue = f.queue();
        let provider = Provider(f.request.input.comparison.clone());
        let id = queue.reconcile(2, 3, 7, &provider).unwrap().unwrap();
        {
            let s = queue.storage.lock().unwrap();
            s.connection
                .execute(
                    "UPDATE app_analysis_jobs SET state='failed',delivery_attempted=1",
                    [],
                )
                .unwrap();
            s.connection
                .execute("UPDATE app_publication_consents SET revoked=1", [])
                .unwrap();
        }
        let writer = FindOnly {
            expected: Value::Null,
            found: Cell::new(Some(42)),
            lookups: Cell::new(0),
        };
        let evidence = AppRecoveryEvidence {
            receipt_key: &[],
            readback: None,
            provider: &provider,
            checks: &writer,
        };
        let request = AppRecoveryRequest {
            job_id: &id,
            actor: "operator",
            reason: "Do not revive old consent",
            decision: AppRecoveryDecision::ReconcileProgress { check_id: 42 },
        };
        assert_eq!(
            queue.recover_job(&request, Some(&evidence)),
            Err(SetupError::Denied)
        );
        assert_eq!(writer.lookups.get(), 0);
    }
}
