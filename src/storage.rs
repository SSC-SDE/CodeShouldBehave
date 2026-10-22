use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, types::Type};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "0001_foundation",
        include_str!("../migrations/0001_foundation.sql"),
    ),
    (
        2,
        "0002_indexes",
        include_str!("../migrations/0002_indexes.sql"),
    ),
    (
        3,
        "0003_graph_navigation",
        include_str!("../migrations/0003_graph_navigation.sql"),
    ),
    (
        4,
        "0004_incremental_change_sets",
        include_str!("../migrations/0004_incremental_change_sets.sql"),
    ),
    (
        5,
        "0005_scheduler",
        include_str!("../migrations/0005_scheduler.sql"),
    ),
    (
        6,
        "0006_provider_intake",
        include_str!("../migrations/0006_provider_intake.sql"),
    ),
    (
        7,
        "0007_export_jobs",
        include_str!("../migrations/0007_export_jobs.sql"),
    ),
    (
        8,
        "0008_ai_audit",
        include_str!("../migrations/0008_ai_audit.sql"),
    ),
    (
        9,
        "0009_review_continuity",
        include_str!("../migrations/0009_review_continuity.sql"),
    ),
    (
        10,
        "0010_onboarding",
        include_str!("../migrations/0010_onboarding.sql"),
    ),
    (
        11,
        "0011_onboarding_defaults",
        include_str!("../migrations/0011_onboarding_defaults.sql"),
    ),
    (
        12,
        "0012_design",
        include_str!("../migrations/0012_design.sql"),
    ),
    (
        13,
        "0013_lld_defaults",
        include_str!("../migrations/0013_lld_defaults.sql"),
    ),
    (
        14,
        "0014_workspace_preferences",
        include_str!("../migrations/0014_workspace_preferences.sql"),
    ),
];
const MIN_SQLITE: (u32, u32, u32) = (3, 51, 3);
const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub struct Storage {
    pub(crate) connection: Connection,
    artifact_root: PathBuf,
    sqlite_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactRef {
    pub id: i64,
    pub digest: String,
    pub relative_path: PathBuf,
    pub byte_length: u64,
    pub format_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiAuditRecord {
    pub id: i64,
    pub review_id: String,
    pub bundle_id: String,
    pub principal: String,
    pub destination: String,
    pub disclosure: String,
    pub capability: String,
    pub outcome: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub rule_version: String,
    pub evidence_count: usize,
    pub evidence_bytes: usize,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFileProgress {
    pub path: String,
    pub blob_id: Option<String>,
    pub coverage: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSymbol {
    pub stable_key: String,
    pub correspondence_key: Option<String>,
    pub path: String,
    pub kind: String,
    pub name: String,
    pub start_line: Option<i64>,
    pub start_column: Option<i64>,
    pub end_line: Option<i64>,
    pub end_column: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredEdge {
    pub stable_key: String,
    pub from_symbol: Option<String>,
    pub to_symbol: Option<String>,
    pub kind: String,
    pub confidence: f64,
    pub resolution: String,
    pub source_path: String,
    pub source_span: Option<(i64, i64, i64, i64)>,
    pub provenance: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    pub id: i64,
    pub stable_key: String,
    pub from_symbol: Option<String>,
    pub to_symbol: Option<String>,
    pub kind: String,
    pub confidence: f64,
    pub resolution: String,
    pub source_path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphPage {
    pub edges: Vec<GraphEdge>,
    pub next_cursor: Option<String>,
    pub omitted: usize,
    pub coverage: Vec<(String, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphPathPage {
    pub paths: Vec<(String, String)>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphSymbolPage {
    pub symbols: Vec<StoredSymbol>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reconciliation {
    pub temporary_files_removed: usize,
    pub missing_references: usize,
    pub corrupt_references: usize,
    pub orphaned_files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactRetentionPolicy {
    pub minimum_unreferenced_age_ms: u64,
    pub maximum_deletions: usize,
}

impl Default for ArtifactRetentionPolicy {
    fn default() -> Self {
        Self {
            minimum_unreferenced_age_ms: 30 * 24 * 60 * 60 * 1_000,
            maximum_deletions: 100,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtifactGarbageCollection {
    pub candidates: usize,
    pub deleted: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedReviewRetentionPolicy {
    pub minimum_closed_age_ms: u64,
    pub maximum_reviews: usize,
}

impl Default for ClosedReviewRetentionPolicy {
    fn default() -> Self {
        Self {
            minimum_closed_age_ms: 30 * 24 * 60 * 60 * 1_000,
            maximum_reviews: 100,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedReviewRetentionCandidate {
    pub review_id: String,
    pub closed_at_ms: u64,
    pub artifact_links: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedReviewRetentionPlan {
    pub identity: String,
    pub candidates: Vec<ClosedReviewRetentionCandidate>,
    pub artifact_links: usize,
}

impl Storage {
    pub fn open(database: &Path, artifact_root: &Path) -> Result<Self, String> {
        reject_symlink(database, "Database")?;
        reject_symlink(artifact_root, "Artifact root")?;
        if let Some(parent) = database
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create database directory: {error}"))?;
        }
        fs::create_dir_all(artifact_root)
            .map_err(|error| format!("Cannot create artifact root: {error}"))?;
        let mut connection = Connection::open(database)
            .map_err(|error| format!("Cannot open SQLite database: {error}"))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| format!("Cannot configure SQLite busy timeout: {error}"))?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;\nPRAGMA journal_mode=WAL;\nPRAGMA synchronous=FULL;",
            )
            .map_err(|error| format!("Cannot configure SQLite durability: {error}"))?;
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .map_err(|error| format!("Cannot verify SQLite foreign keys: {error}"))?;
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|error| format!("Cannot verify SQLite WAL mode: {error}"))?;
        if foreign_keys != 1 || !journal_mode.eq_ignore_ascii_case("wal") {
            return Err("SQLite must have foreign keys enabled and use local-disk WAL mode".into());
        }
        let sqlite_version: String = connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .map_err(|error| format!("Cannot read SQLite runtime version: {error}"))?;
        if parse_version(&sqlite_version)? < MIN_SQLITE {
            return Err(format!(
                "SQLite {sqlite_version} is unsupported; require 3.51.3 or a documented fixed backport"
            ));
        }
        apply_migrations(&mut connection)?;
        Ok(Self {
            connection,
            artifact_root: artifact_root.to_path_buf(),
            sqlite_version,
        })
    }

    pub fn sqlite_version(&self) -> &str {
        &self.sqlite_version
    }

    pub fn schema_version(&self) -> Result<i64, String> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| format!("Cannot read schema version: {error}"))
    }

    pub fn create_consistent_backup(&self, database: &Path, manifest: &Path) -> Result<(), String> {
        reject_new_backup_path(database, "Backup database")?;
        reject_new_backup_path(manifest, "Backup manifest")?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(database)
            .map_err(|error| format!("Cannot reserve backup database: {error}"))?;
        if let Err(error) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(manifest)
        {
            let _ = fs::remove_file(database);
            return Err(format!("Cannot reserve backup manifest: {error}"));
        }
        let result = (|| {
            self.connection
                .backup("main", database, None)
                .map_err(|error| format!("Cannot create SQLite online backup: {error}"))?;
            fs::File::open(database)
                .and_then(|file| file.sync_all())
                .map_err(|error| format!("Cannot sync backup database: {error}"))?;

            // Query the completed backup, not the live connection, so the manifest and
            // metadata always describe the same SQLite snapshot.
            let backup =
                Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|error| format!("Cannot inspect backup database: {error}"))?;
            let mut statement = backup
                .prepare(
                    "SELECT digest,relative_path,byte_length,format_version
                     FROM artifact_refs AS artifact
                     WHERE state='ready' AND (
                       EXISTS (SELECT 1 FROM artifact_links WHERE artifact_id=artifact.id)
                       OR EXISTS (SELECT 1 FROM export_jobs WHERE artifact_id=artifact.id)
                     ) ORDER BY relative_path",
                )
                .map_err(|error| format!("Cannot prepare backup manifest: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|error| format!("Cannot query backup manifest: {error}"))?;
            let mut bytes = b"codeshouldbehave-backup-manifest-v1\n".to_vec();
            for row in rows {
                let (digest, relative, length, format) =
                    row.map_err(|error| format!("Cannot decode backup manifest: {error}"))?;
                if length < 0
                    || [digest.as_str(), relative.as_str(), format.as_str()]
                        .iter()
                        .any(|value| value.contains(['\t', '\n', '\r']))
                {
                    return Err("Unsafe backup manifest value".into());
                }
                bytes.extend_from_slice(
                    format!("{digest}\t{length}\t{format}\t{relative}\n").as_bytes(),
                );
            }
            let mut file = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(manifest)
                .map_err(|error| format!("Cannot create backup manifest: {error}"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| format!("Cannot persist backup manifest: {error}"))?;
            if let Some(parent) = database.parent() {
                fs::File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| format!("Cannot sync backup directory: {error}"))?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(manifest);
            let _ = fs::remove_file(database);
        }
        result
    }

    pub fn verify_backup(
        database: &Path,
        artifact_root: &Path,
        manifest: &Path,
    ) -> Result<usize, String> {
        reject_symlink(database, "Backup database")?;
        reject_symlink(manifest, "Backup manifest")?;
        reject_symlink(artifact_root, "Backup artifact root")?;
        let connection =
            Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| format!("Cannot open backup database: {error}"))?;
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|error| format!("Cannot verify backup database: {error}"))?;
        if integrity != "ok" {
            return Err("Backup database integrity check failed".into());
        }
        let expected: i64 = connection
            .query_row(
                "SELECT count(*) FROM artifact_refs AS artifact
                 WHERE state='ready' AND (
                   EXISTS (SELECT 1 FROM artifact_links WHERE artifact_id=artifact.id)
                   OR EXISTS (SELECT 1 FROM export_jobs WHERE artifact_id=artifact.id)
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot count referenced backup artifacts: {error}"))?;
        let expected =
            usize::try_from(expected).map_err(|_| "Invalid referenced backup artifact count")?;
        let text = fs::read_to_string(manifest)
            .map_err(|error| format!("Cannot read backup manifest: {error}"))?;
        let mut lines = text.lines();
        if lines.next() != Some("codeshouldbehave-backup-manifest-v1") {
            return Err("Unsupported backup manifest".into());
        }
        let mut count = 0;
        let mut seen = BTreeSet::new();
        for line in lines {
            let parts = line.split('\t').collect::<Vec<_>>();
            if parts.len() != 4 {
                return Err("Malformed backup manifest".into());
            }
            if !seen.insert((parts[0].to_owned(), parts[3].to_owned())) {
                return Err("Backup manifest contains duplicate artifacts".into());
            }
            let length = parts[1]
                .parse::<u64>()
                .map_err(|_| "Malformed backup artifact length")?;
            let sql_length =
                i64::try_from(length).map_err(|_| "Backup artifact length exceeds SQLite range")?;
            let path = safe_artifact_path(artifact_root, parts[3])?;
            let mut file = fs::File::open(&path).map_err(|_| "Backup artifact is missing")?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|_| "Cannot read backup artifact")?;
            if bytes.len() as u64 != length || hex_digest(&bytes) != parts[0] {
                return Err("Backup artifact failed length or digest verification".into());
            }
            let exists: i64 = connection
                .query_row(
                    "SELECT count(*) FROM artifact_refs AS artifact
                     WHERE digest=?1 AND relative_path=?2 AND byte_length=?3
                       AND format_version=?4 AND state='ready' AND (
                         EXISTS (SELECT 1 FROM artifact_links WHERE artifact_id=artifact.id)
                         OR EXISTS (SELECT 1 FROM export_jobs WHERE artifact_id=artifact.id)
                       )",
                    params![parts[0], parts[3], sql_length, parts[2]],
                    |row| row.get(0),
                )
                .map_err(|error| format!("Cannot bind backup manifest: {error}"))?;
            if exists != 1 {
                return Err("Backup manifest does not match metadata".into());
            }
            count += 1;
        }
        if count != expected {
            return Err("Backup manifest omits referenced artifacts".into());
        }
        Ok(count)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_ai_audit(
        &mut self,
        review_id: &str,
        bundle_id: &str,
        principal: &str,
        destination: &str,
        disclosure: &str,
        capability: &str,
        outcome: &str,
        provider: Option<&str>,
        model: Option<&str>,
        rule_version: &str,
        evidence_count: usize,
        evidence_bytes: usize,
        created_at_ms: i64,
    ) -> Result<i64, String> {
        for (kind, value) in [
            ("bundle identity", bundle_id),
            ("principal", principal),
            ("destination", destination),
            ("rule version", rule_version),
        ] {
            validate_label(kind, value)?;
        }
        if !matches!(
            disclosure,
            "code_content" | "symbols_only" | "derived_summary"
        ) || !matches!(capability, "disabled" | "fake_only" | "unavailable")
            || !matches!(
                outcome,
                "prepared" | "disabled" | "accepted" | "rejected" | "cancelled"
            )
            || evidence_count > 100
            || evidence_bytes > 256 * 1024
            || created_at_ms < 0
        {
            return Err("Invalid AI audit metadata".into());
        }
        if let Some(value) = provider {
            validate_label("provider", value)?;
        }
        if let Some(value) = model {
            validate_label("model", value)?;
        }
        self.connection.execute(
            "INSERT INTO ai_audit_events(review_id,bundle_id,principal,destination,disclosure,capability,outcome,provider,model,rule_version,evidence_count,evidence_bytes,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![review_id,bundle_id,principal,destination,disclosure,capability,outcome,provider,model,rule_version,evidence_count as i64,evidence_bytes as i64,created_at_ms],
        ).map_err(|error| format!("Cannot append AI audit metadata: {error}"))?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn ai_audit_for_review(
        &self,
        review_id: &str,
        limit: usize,
    ) -> Result<Vec<AiAuditRecord>, String> {
        if limit == 0 || limit > 200 {
            return Err("AI audit limit must be between 1 and 200".into());
        }
        let mut statement = self.connection.prepare(
            "SELECT id,review_id,bundle_id,principal,destination,disclosure,capability,outcome,provider,model,rule_version,evidence_count,evidence_bytes,created_at_ms FROM ai_audit_events WHERE review_id=?1 ORDER BY id LIMIT ?2"
        ).map_err(|error| format!("Cannot prepare AI audit query: {error}"))?;
        let rows = statement
            .query_map(params![review_id, limit as i64], |row| {
                Ok(AiAuditRecord {
                    id: row.get(0)?,
                    review_id: row.get(1)?,
                    bundle_id: row.get(2)?,
                    principal: row.get(3)?,
                    destination: row.get(4)?,
                    disclosure: row.get(5)?,
                    capability: row.get(6)?,
                    outcome: row.get(7)?,
                    provider: row.get(8)?,
                    model: row.get(9)?,
                    rule_version: row.get(10)?,
                    evidence_count: usize::try_from(row.get::<_, i64>(11)?).unwrap_or(usize::MAX),
                    evidence_bytes: usize::try_from(row.get::<_, i64>(12)?).unwrap_or(usize::MAX),
                    created_at_ms: row.get(13)?,
                })
            })
            .map_err(|error| format!("Cannot query AI audit metadata: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode AI audit metadata: {error}"))
    }

    pub fn register_repository(
        &mut self,
        security_domain: &str,
        provider: &str,
        owner: &str,
        name: &str,
    ) -> Result<i64, String> {
        for (kind, value) in [
            ("security domain", security_domain),
            ("repository provider", provider),
            ("repository owner", owner),
            ("repository name", name),
        ] {
            validate_label(kind, value)?;
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO repositories(security_domain,provider,owner,name,created_at_ms)
             VALUES (?1,?2,?3,?4,?5)",
            params![security_domain, provider, owner, name, now_millis()],
        ).map_err(|error| format!("Cannot register repository: {error}"))?;
        self.connection
            .query_row(
                "SELECT id FROM repositories
             WHERE security_domain=?1 AND provider=?2 AND owner=?3 AND name=?4",
                params![security_domain, provider, owner, name],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot read repository identity: {error}"))
    }

    pub fn register_scope(
        &mut self,
        repository_id: i64,
        version: &str,
        canonical: &[u8],
    ) -> Result<i64, String> {
        validate_label("scope version", version)?;
        if canonical.is_empty() || canonical.len() > 1024 * 1024 {
            return Err("Canonical scope must contain 1 byte to 1 MiB".into());
        }
        self.connection
            .execute(
                "INSERT OR IGNORE INTO scopes(repository_id,version,canonical,created_at_ms)
             VALUES (?1,?2,?3,?4)",
                params![repository_id, version, canonical, now_millis()],
            )
            .map_err(|error| format!("Cannot register scope: {error}"))?;
        self.connection
            .query_row(
                "SELECT id FROM scopes
                 WHERE repository_id=?1 AND version=?2 AND canonical=?3",
                params![repository_id, version, canonical],
                |row| row.get(0),
            )
            .map_err(|_| {
                "Scope version or canonical identity conflicts with an existing scope".into()
            })
    }

    pub fn begin_snapshot(
        &mut self,
        repository_id: i64,
        commit_id: &str,
        scope_id: i64,
        analyzer_version: &str,
    ) -> Result<(i64, bool), String> {
        if !matches!(commit_id.len(), 40 | 64)
            || !commit_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || analyzer_version.is_empty()
            || analyzer_version.len() > 128
        {
            return Err("Invalid snapshot identity".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start snapshot transaction: {error}"))?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO snapshots
                 (repository_id,commit_id,scope_id,analyzer_version,status,created_at_ms)
                 VALUES (?1,?2,?3,?4,'pending',?5)",
                params![
                    repository_id,
                    commit_id,
                    scope_id,
                    analyzer_version,
                    now_millis()
                ],
            )
            .map_err(|error| format!("Cannot create snapshot: {error}"))?
            == 1;
        let id = transaction
            .query_row(
                "SELECT id FROM snapshots
                 WHERE repository_id=?1 AND commit_id=?2 AND scope_id=?3 AND analyzer_version=?4",
                params![repository_id, commit_id, scope_id, analyzer_version],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot read snapshot identity: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit snapshot identity: {error}"))?;
        Ok((id, inserted))
    }

    pub fn record_graph_paths(&mut self, snapshot_id: i64, files: &[String]) -> Result<(), String> {
        if files.len() > 256 {
            return Err("Graph path chunks are limited to 256 files".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start graph path transaction: {error}"))?;
        for file in files {
            validate_repository_path(file)?;
            let parts = file.split('/').collect::<Vec<_>>();
            let mut parent: Option<String> = None;
            for index in 0..parts.len() {
                let path = parts[..=index].join("/");
                let kind = if index + 1 == parts.len() {
                    "file"
                } else if index == 0 {
                    "component"
                } else {
                    "module"
                };
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO graph_paths(snapshot_id,path,parent_path,kind)
                         VALUES (?1,?2,?3,?4)",
                        params![snapshot_id, path, parent, kind],
                    )
                    .map_err(|error| format!("Cannot record graph hierarchy: {error}"))?;
                let recorded: (Option<String>, String) = transaction
                    .query_row(
                        "SELECT parent_path,kind FROM graph_paths
                         WHERE snapshot_id=?1 AND path=?2",
                        params![snapshot_id, path],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(|error| format!("Cannot verify graph hierarchy: {error}"))?;
                if recorded != (parent.clone(), kind.to_owned()) {
                    return Err("Conflicting file and directory graph paths".into());
                }
                parent = Some(path);
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit graph hierarchy: {error}"))
    }

    pub fn record_graph_chunk(
        &mut self,
        snapshot_id: i64,
        symbols: &[StoredSymbol],
        edges: &[StoredEdge],
    ) -> Result<(), String> {
        if symbols.len() > 256 || edges.len() > 512 {
            return Err("Graph publication chunk exceeds its bounded limit".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start graph transaction: {error}"))?;
        for symbol in symbols {
            validate_repository_path(&symbol.path)?;
            validate_graph_label("symbol stable key", &symbol.stable_key, 4096)?;
            validate_graph_label("symbol kind", &symbol.kind, 128)?;
            validate_graph_label("symbol name", &symbol.name, 1024)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO symbols
                     (snapshot_id,path,stable_key,correspondence_key,kind,name,start_line,start_column,end_line,end_column)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        snapshot_id,
                        symbol.path,
                        symbol.stable_key,
                        symbol.correspondence_key,
                        symbol.kind,
                        symbol.name,
                        symbol.start_line,
                        symbol.start_column,
                        symbol.end_line,
                        symbol.end_column
                    ],
                )
                .map_err(|error| format!("Cannot record graph symbol: {error}"))?;
        }
        let symbol_ids = {
            let mut statement = transaction
                .prepare("SELECT stable_key,id FROM symbols WHERE snapshot_id=?1")
                .map_err(|error| format!("Cannot prepare symbol identity query: {error}"))?;
            let rows = statement
                .query_map([snapshot_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|error| format!("Cannot query symbol identities: {error}"))?;
            rows.collect::<Result<BTreeMap<String, i64>, _>>()
                .map_err(|error| format!("Cannot decode symbol identities: {error}"))?
        };
        for edge in edges {
            validate_repository_path(&edge.source_path)?;
            validate_graph_label("edge stable key", &edge.stable_key, 4096)?;
            validate_graph_label("edge kind", &edge.kind, 128)?;
            validate_graph_label("edge resolution", &edge.resolution, 128)?;
            if !(0.0..=1.0).contains(&edge.confidence) || !edge.confidence.is_finite() {
                return Err("Invalid edge confidence".into());
            }
            let from_id = edge
                .from_symbol
                .as_ref()
                .map(|key| {
                    symbol_ids
                        .get(key)
                        .copied()
                        .ok_or("Unknown edge source symbol")
                })
                .transpose()?;
            let to_id = edge
                .to_symbol
                .as_ref()
                .map(|key| {
                    symbol_ids
                        .get(key)
                        .copied()
                        .ok_or("Unknown edge target symbol")
                })
                .transpose()?;
            let (start_line, start_column, end_line, end_column) =
                edge.source_span.unwrap_or((0, 0, 0, 0));
            transaction
                .execute(
                    "INSERT OR IGNORE INTO edges
                     (snapshot_id,stable_key,from_symbol_id,to_symbol_id,kind,confidence,resolution,
                      source_path,source_start_line,source_start_column,source_end_line,source_end_column,provenance)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                    params![snapshot_id,edge.stable_key,from_id,to_id,edge.kind,edge.confidence,
                        edge.resolution,edge.source_path,start_line,start_column,end_line,end_column,
                        edge.provenance],
                )
                .map_err(|error| format!("Cannot record graph edge: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit graph chunk: {error}"))
    }

    pub fn finish_snapshot(&mut self, snapshot_id: i64) -> Result<String, String> {
        let pending: i64 = self
            .connection
            .query_row(
                "SELECT count(*) FROM files WHERE snapshot_id=?1 AND coverage='pending'",
                [snapshot_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect snapshot coverage: {error}"))?;
        let complete: i64 = self
            .connection
            .query_row(
                "SELECT enumeration_complete FROM snapshots WHERE id=?1",
                [snapshot_id],
                |row| row.get(0),
            )
            .map_err(|_| "Snapshot does not exist".to_owned())?;
        if pending != 0 || complete != 1 {
            return Err(
                "Snapshot cannot complete while enumeration or file coverage is pending".into(),
            );
        }
        let partial: i64 = self
            .connection
            .query_row(
                "SELECT count(*) FROM files
                 WHERE snapshot_id=?1 AND coverage IN ('unsupported','failed','deferred')",
                [snapshot_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect terminal coverage: {error}"))?;
        let status = if partial == 0 { "ready" } else { "partial" };
        self.connection
            .execute(
                "UPDATE snapshots SET status=?1 WHERE id=?2",
                params![status, snapshot_id],
            )
            .map_err(|error| format!("Cannot finish snapshot: {error}"))?;
        Ok(status.into())
    }

    pub fn graph_children(
        &self,
        snapshot_id: i64,
        parent: Option<&str>,
        scope: &crate::PathScope,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphPathPage, String> {
        if limit == 0 || limit > 200 {
            return Err("Graph navigation limit must be between 1 and 200".into());
        }
        if let Some(parent) = parent {
            validate_repository_path(parent)?;
        }
        let binding = cursor_binding(snapshot_id, "children", parent.unwrap_or("root"));
        let after = decode_cursor(cursor, "p1", &binding, "")?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT path,kind FROM graph_paths
                 WHERE snapshot_id=?1 AND parent_path IS ?2 AND path>?3
                 ORDER BY path LIMIT 1001",
            )
            .map_err(|error| format!("Cannot prepare graph navigation: {error}"))?;
        let rows = statement
            .query_map(params![snapshot_id, parent, after], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("Cannot query graph navigation: {error}"))?;
        let visible = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode graph navigation: {error}"))?
            .into_iter()
            .filter(|(path, _)| scope.intersects_prefix(path))
            .collect::<Vec<_>>();
        let has_more = visible.len() > limit;
        let paths = visible.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = has_more
            .then(|| {
                paths
                    .last()
                    .map(|(path, _)| encode_cursor("p1", &binding, path))
            })
            .flatten();
        Ok(GraphPathPage { paths, next_cursor })
    }

    pub fn graph_symbols(
        &self,
        snapshot_id: i64,
        path: &str,
        scope: &crate::PathScope,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphSymbolPage, String> {
        if limit == 0 || limit > 200 {
            return Err("Graph navigation limit must be between 1 and 200".into());
        }
        validate_repository_path(path)?;
        if !scope.allows(path) {
            return Err("Graph file is unavailable".into());
        }
        let binding = cursor_binding(snapshot_id, "symbols", path);
        let after = decode_cursor(cursor, "s1", &binding, "")?;
        let mut statement = self.connection.prepare(
            "SELECT stable_key,correspondence_key,path,kind,name,start_line,start_column,end_line,end_column
             FROM symbols WHERE snapshot_id=?1 AND path=?2 AND stable_key>?3
             ORDER BY stable_key LIMIT ?4",
        ).map_err(|error| format!("Cannot prepare graph symbol navigation: {error}"))?;
        let rows = statement
            .query_map(
                params![snapshot_id, path, after, (limit + 1) as i64],
                |row| {
                    Ok(StoredSymbol {
                        stable_key: row.get(0)?,
                        correspondence_key: row.get(1)?,
                        path: row.get(2)?,
                        kind: row.get(3)?,
                        name: row.get(4)?,
                        start_line: row.get(5)?,
                        start_column: row.get(6)?,
                        end_line: row.get(7)?,
                        end_column: row.get(8)?,
                    })
                },
            )
            .map_err(|error| format!("Cannot query graph symbol navigation: {error}"))?;
        let mut symbols = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode graph symbol navigation: {error}"))?;
        let has_more = symbols.len() > limit;
        symbols.truncate(limit);
        let next_cursor = has_more
            .then(|| {
                symbols
                    .last()
                    .map(|symbol| encode_cursor("s1", &binding, &symbol.stable_key))
            })
            .flatten();
        Ok(GraphSymbolPage {
            symbols,
            next_cursor,
        })
    }

    pub fn graph_neighborhood(
        &self,
        snapshot_id: i64,
        symbol_key: &str,
        direction: GraphDirection,
        scope: &crate::PathScope,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphPage, String> {
        if limit == 0 || limit > 200 {
            return Err("Graph neighborhood limit must be between 1 and 200".into());
        }
        let (symbol_id, symbol_path): (i64, String) = self
            .connection
            .query_row(
                "SELECT id,path FROM symbols WHERE snapshot_id=?1 AND stable_key=?2",
                params![snapshot_id, symbol_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Graph symbol is unavailable".to_owned())?;
        if !scope.allows(&symbol_path) {
            return Err("Graph symbol is unavailable".into());
        }
        let direction_name = match direction {
            GraphDirection::Incoming => "incoming",
            GraphDirection::Outgoing => "outgoing",
        };
        let binding = cursor_binding(snapshot_id, direction_name, symbol_key);
        let after = decode_cursor(cursor, "e1", &binding, "0")?
            .parse::<i64>()
            .map_err(|_| "Invalid graph cursor position")?;
        let column = match direction {
            GraphDirection::Incoming => "e.to_symbol_id",
            GraphDirection::Outgoing => "e.from_symbol_id",
        };
        let sql = format!(
            "SELECT e.id,e.stable_key,fs.stable_key,ts.stable_key,e.kind,e.confidence,
                    e.resolution,e.source_path,fs.path,ts.path
             FROM edges e
             LEFT JOIN symbols fs ON fs.id=e.from_symbol_id
             LEFT JOIN symbols ts ON ts.id=e.to_symbol_id
             WHERE e.snapshot_id=?1 AND {column}=?2 AND e.id>?3
             ORDER BY e.id LIMIT 10001"
        );
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| format!("Cannot prepare graph neighborhood: {error}"))?;
        let rows = statement
            .query_map(params![snapshot_id, symbol_id, after], |row| {
                Ok((
                    GraphEdge {
                        id: row.get(0)?,
                        stable_key: row.get(1)?,
                        from_symbol: row.get(2)?,
                        to_symbol: row.get(3)?,
                        kind: row.get(4)?,
                        confidence: row.get(5)?,
                        resolution: row.get(6)?,
                        source_path: row.get(7)?,
                    },
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            })
            .map_err(|error| format!("Cannot query graph neighborhood: {error}"))?;
        let visible = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode graph neighborhood: {error}"))?
            .into_iter()
            .filter(|(edge, from_path, to_path)| {
                scope.allows(&edge.source_path)
                    && from_path.as_deref().is_none_or(|path| scope.allows(path))
                    && to_path.as_deref().is_none_or(|path| scope.allows(path))
            })
            .map(|(edge, _, _)| edge)
            .collect::<Vec<_>>();
        let omitted = visible.len().saturating_sub(limit);
        let edges = visible.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = (omitted > 0)
            .then(|| {
                edges
                    .last()
                    .map(|edge| encode_cursor("e1", &binding, &edge.id.to_string()))
            })
            .flatten();
        let mut coverage = BTreeMap::new();
        let mut coverage_statement = self
            .connection
            .prepare("SELECT path,coverage FROM files WHERE snapshot_id=?1 ORDER BY path")
            .map_err(|error| format!("Cannot prepare coverage query: {error}"))?;
        let coverage_rows = coverage_statement
            .query_map([snapshot_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("Cannot query graph coverage: {error}"))?;
        for row in coverage_rows {
            let (path, state) =
                row.map_err(|error| format!("Cannot decode graph coverage: {error}"))?;
            if scope.allows(&path) {
                *coverage.entry(state).or_insert(0) += 1;
            }
        }
        Ok(GraphPage {
            edges,
            next_cursor,
            omitted,
            coverage: coverage.into_iter().collect(),
        })
    }

    pub fn record_file_chunk(
        &mut self,
        snapshot_id: i64,
        files: &[StoredFileProgress],
        enumeration_complete: bool,
    ) -> Result<(), String> {
        if files.len() > 256 {
            return Err("Snapshot progress chunks are limited to 256 files".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start snapshot progress transaction: {error}"))?;
        for file in files {
            validate_repository_path(&file.path)?;
            if !matches!(
                file.coverage.as_str(),
                "pending" | "indexed" | "unsupported" | "excluded" | "failed" | "deferred"
            ) {
                return Err("Invalid snapshot file progress record".into());
            }
            transaction
                .execute(
                    "INSERT INTO files(snapshot_id,path,blob_id,coverage,detail)
                     VALUES (?1,?2,?3,?4,?5)
                     ON CONFLICT(snapshot_id,path) DO UPDATE SET
                         blob_id=excluded.blob_id,
                         coverage=excluded.coverage,
                         detail=excluded.detail",
                    params![
                        snapshot_id,
                        file.path,
                        file.blob_id,
                        file.coverage,
                        file.detail
                    ],
                )
                .map_err(|error| format!("Cannot record snapshot file progress: {error}"))?;
        }
        if enumeration_complete {
            transaction
                .execute(
                    "UPDATE snapshots SET enumeration_complete=1 WHERE id=?1",
                    [snapshot_id],
                )
                .map_err(|error| format!("Cannot complete snapshot enumeration: {error}"))?;
            if transaction.changes() != 1 {
                return Err("Snapshot does not exist".into());
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit snapshot progress: {error}"))
    }

    pub fn load_file_progress(&self, snapshot_id: i64) -> Result<Vec<StoredFileProgress>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT path,blob_id,coverage,detail FROM files
                 WHERE snapshot_id=?1 ORDER BY path",
            )
            .map_err(|error| format!("Cannot prepare snapshot progress query: {error}"))?;
        statement
            .query_map([snapshot_id], |row| {
                Ok(StoredFileProgress {
                    path: row.get(0)?,
                    blob_id: row.get(1)?,
                    coverage: row.get(2)?,
                    detail: row.get(3)?,
                })
            })
            .map_err(|error| format!("Cannot query snapshot progress: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode snapshot progress: {error}"))
    }

    pub fn store_change_set(
        &mut self,
        repository_id: i64,
        baseline_snapshot_id: i64,
        proposal_snapshot_id: i64,
        change_set: &crate::ChangeSet,
    ) -> Result<bool, String> {
        if change_set.changes.len() > 2_000
            || change_set.invalidations.len() > 4_000
            || change_set.reused_paths.len() > 100_000
            || change_set.affected_unchanged_callers.len() > 100_000
        {
            return Err("Incremental change set exceeds its bounded storage limits".into());
        }
        if !strictly_sorted(&change_set.parse_paths)
            || !strictly_sorted(&change_set.reused_paths)
            || !strictly_sorted(&change_set.affected_unchanged_callers)
        {
            return Err("Change-set path lists must be sorted and unique".into());
        }
        let expected_resolution = if change_set
            .invalidations
            .iter()
            .all(|cause| matches!(cause, crate::InvalidationCause::Body(_)))
        {
            crate::ResolutionStrategy::Local
        } else {
            crate::ResolutionStrategy::Full {
                reasons: change_set
                    .invalidations
                    .iter()
                    .filter(|cause| !matches!(cause, crate::InvalidationCause::Body(_)))
                    .cloned()
                    .collect(),
            }
        };
        if change_set.resolution != expected_resolution {
            return Err("Change-set resolution does not match its invalidations".into());
        }
        validate_graph_label("change set identity", &change_set.id, 128)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start change-set transaction: {error}"))?;
        let snapshot_commits: (String, String) = transaction
            .query_row(
                "SELECT b.commit_id,p.commit_id FROM snapshots b JOIN snapshots p
                 WHERE b.id=?1 AND p.id=?2
                   AND b.repository_id=?3 AND p.repository_id=?3",
                params![baseline_snapshot_id, proposal_snapshot_id, repository_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Change-set snapshots do not belong to the repository".to_owned())?;
        if snapshot_commits != (change_set.baseline.clone(), change_set.proposal.clone()) {
            return Err("Change-set commits do not match their snapshots".into());
        }
        let strategy = match change_set.resolution {
            crate::ResolutionStrategy::Local => "local",
            crate::ResolutionStrategy::Full { .. } => "full",
        };
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO change_sets
                 (id,repository_id,baseline_snapshot_id,proposal_snapshot_id,baseline_commit,
                  proposal_commit,resolution_strategy,created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    change_set.id,
                    repository_id,
                    baseline_snapshot_id,
                    proposal_snapshot_id,
                    change_set.baseline,
                    change_set.proposal,
                    strategy,
                    now_millis()
                ],
            )
            .map_err(|error| format!("Cannot store change-set identity: {error}"))?
            == 1;
        if inserted {
            for (position, change) in change_set.changes.iter().enumerate() {
                let (kind, path, old_path) = match change {
                    crate::PathChange::Added { path } => ("added", path, None),
                    crate::PathChange::Deleted { path } => ("deleted", path, None),
                    crate::PathChange::Modified { path } => ("modified", path, None),
                    crate::PathChange::Renamed { from, to } => ("renamed", to, Some(from)),
                };
                validate_repository_path(path)?;
                if let Some(old_path) = old_path {
                    validate_repository_path(old_path)?;
                }
                let parse_required = change_set.parse_paths.binary_search(path).is_ok();
                transaction.execute(
                    "INSERT INTO change_paths(change_set_id,position,kind,path,old_path,parse_required)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                    params![change_set.id, position as i64, kind, path, old_path, i64::from(parse_required)],
                ).map_err(|error| format!("Cannot store changed path: {error}"))?;
            }
            for (position, cause) in change_set.invalidations.iter().enumerate() {
                let (kind, path, target) = encode_invalidation(cause);
                transaction.execute(
                    "INSERT INTO change_invalidations(change_set_id,position,kind,path,target_path)
                     VALUES (?1,?2,?3,?4,?5)",
                    params![change_set.id, position as i64, kind, path, target],
                ).map_err(|error| format!("Cannot store invalidation: {error}"))?;
            }
            for path in &change_set.reused_paths {
                validate_repository_path(path)?;
                transaction
                    .execute(
                        "INSERT INTO change_reused_paths(change_set_id,path) VALUES (?1,?2)",
                        params![change_set.id, path],
                    )
                    .map_err(|error| format!("Cannot store reused path: {error}"))?;
            }
            for path in &change_set.affected_unchanged_callers {
                validate_repository_path(path)?;
                transaction
                    .execute(
                        "INSERT INTO change_affected_callers(change_set_id,path) VALUES (?1,?2)",
                        params![change_set.id, path],
                    )
                    .map_err(|error| format!("Cannot store affected caller: {error}"))?;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit change set: {error}"))?;
        if self.load_change_set(&change_set.id)? != *change_set {
            return Err("Stored change set conflicts with its immutable identity".into());
        }
        Ok(inserted)
    }

    pub fn load_change_set(&self, id: &str) -> Result<crate::ChangeSet, String> {
        validate_graph_label("change set identity", id, 128)?;
        let (baseline, proposal, strategy): (String, String, String) = self.connection.query_row(
            "SELECT baseline_commit,proposal_commit,resolution_strategy FROM change_sets WHERE id=?1",
            [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(|_| "Change set is unavailable".to_owned())?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT kind,path,old_path,parse_required FROM change_paths
             WHERE change_set_id=?1 ORDER BY position",
            )
            .map_err(|error| format!("Cannot prepare changed paths: {error}"))?;
        let rows = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| format!("Cannot query changed paths: {error}"))?;
        let mut changes = Vec::new();
        let mut parse_paths = Vec::new();
        for row in rows {
            let (kind, path, old_path, parse_required) =
                row.map_err(|error| format!("Cannot decode changed path: {error}"))?;
            if parse_required == 1 {
                parse_paths.push(path.clone());
            }
            changes.push(match kind.as_str() {
                "added" => crate::PathChange::Added { path },
                "deleted" => crate::PathChange::Deleted { path },
                "modified" => crate::PathChange::Modified { path },
                "renamed" => crate::PathChange::Renamed {
                    from: old_path.ok_or("Renamed path has no origin")?,
                    to: path,
                },
                _ => return Err("Unknown changed-path kind".into()),
            });
        }
        let mut invalidation_statement = self
            .connection
            .prepare(
                "SELECT kind,path,target_path FROM change_invalidations
             WHERE change_set_id=?1 ORDER BY position",
            )
            .map_err(|error| format!("Cannot prepare invalidations: {error}"))?;
        let invalidation_rows = invalidation_statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| format!("Cannot query invalidations: {error}"))?;
        let invalidations = invalidation_rows
            .map(|row| {
                let (kind, path, target) =
                    row.map_err(|error| format!("Cannot decode invalidation: {error}"))?;
                decode_invalidation(&kind, path, target)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let reused_paths = load_path_set(&self.connection, "change_reused_paths", id)?;
        let affected_unchanged_callers =
            load_path_set(&self.connection, "change_affected_callers", id)?;
        let resolution = match strategy.as_str() {
            "local" => crate::ResolutionStrategy::Local,
            "full" => crate::ResolutionStrategy::Full {
                reasons: invalidations
                    .iter()
                    .filter(|cause| !matches!(cause, crate::InvalidationCause::Body(_)))
                    .cloned()
                    .collect(),
            },
            _ => return Err("Unknown resolution strategy".into()),
        };
        Ok(crate::ChangeSet {
            id: id.into(),
            baseline,
            proposal,
            changes,
            parse_paths,
            reused_paths,
            affected_unchanged_callers,
            invalidations,
            resolution,
        })
    }

    pub fn publish_artifact(
        &mut self,
        security_domain: &str,
        format_version: &str,
        bytes: &[u8],
    ) -> Result<ArtifactRef, String> {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err("Artifact exceeds the 64 MiB publication limit".into());
        }
        validate_label("Security domain", security_domain)?;
        validate_label("Artifact format version", format_version)?;
        let digest = hex_digest(bytes);
        let domain = hex_digest(security_domain.as_bytes());
        let relative = PathBuf::from("objects")
            .join(domain)
            .join(format!("{digest}-{format_version}.bin"));
        let final_path = self.artifact_root.join(&relative);
        let parent = final_path
            .parent()
            .ok_or("Artifact destination has no parent")?;
        reject_symlink(parent, "Artifact directory")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create artifact directory: {error}"))?;
        let temporary = parent.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| format!("Cannot create temporary artifact: {error}"))?;
            file.write_all(bytes)
                .map_err(|error| format!("Cannot write temporary artifact: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("Cannot sync temporary artifact: {error}"))?;
            match fs::hard_link(&temporary, &final_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    verify_file(&final_path, bytes.len() as u64, &digest)?;
                }
                Err(error) => return Err(format!("Cannot finalize artifact atomically: {error}")),
            }
            fs::remove_file(&temporary)
                .map_err(|error| format!("Cannot remove temporary artifact: {error}"))?;
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("Cannot sync artifact directory: {error}"))?;
            Ok::<(), String>(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result?;

        let now = now_millis();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start artifact transaction: {error}"))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO artifact_refs
                 (security_domain,digest,relative_path,byte_length,format_version,state,created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,'ready',?6)",
                params![
                    security_domain,
                    digest,
                    path_text(&relative)?,
                    bytes.len() as i64,
                    format_version,
                    now
                ],
            )
            .map_err(|error| format!("Cannot record artifact: {error}"))?;
        let reference = transaction
            .query_row(
                "SELECT id,digest,relative_path,byte_length,format_version
                 FROM artifact_refs WHERE security_domain=?1 AND digest=?2 AND format_version=?3",
                params![security_domain, digest, format_version],
                artifact_from_row,
            )
            .map_err(|error| format!("Cannot read artifact reference: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit artifact reference: {error}"))?;
        Ok(reference)
    }

    pub fn collect_unreferenced_artifacts(
        &mut self,
        policy: &ArtifactRetentionPolicy,
        now_ms: u64,
    ) -> Result<ArtifactGarbageCollection, String> {
        if policy.minimum_unreferenced_age_ms < 60_000
            || policy.maximum_deletions == 0
            || policy.maximum_deletions > 1_000
        {
            return Err("Invalid artifact retention policy".into());
        }
        let cutoff = now_ms
            .checked_sub(policy.minimum_unreferenced_age_ms)
            .ok_or("Retention cutoff precedes the Unix epoch")?;
        let now = i64::try_from(now_ms).map_err(|_| "Retention time exceeds SQLite range")?;
        let cutoff = i64::try_from(cutoff).map_err(|_| "Retention cutoff exceeds SQLite range")?;
        let limit = i64::try_from(policy.maximum_deletions)
            .map_err(|_| "Retention deletion limit exceeds SQLite range")?;
        let artifact_root = self.artifact_root.clone();
        reject_symlink(&artifact_root, "Artifact root")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start artifact retention transaction: {error}"))?;
        let active_leases: i64 = transaction
            .query_row(
                "SELECT count(*) FROM jobs
                 WHERE state='running' AND lease_expires_at_ms>?1",
                [now],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect active leases: {error}"))?;
        if active_leases > 0 {
            return Err("Artifact collection is blocked by active worker leases".into());
        }
        let candidates = {
            let mut statement = transaction
                .prepare(
                    "SELECT id,digest,relative_path,byte_length,state
                     FROM artifact_refs AS artifact
                     WHERE created_at_ms<=?1
                       AND NOT EXISTS (
                         SELECT 1 FROM artifact_links WHERE artifact_id=artifact.id
                       )
                       AND NOT EXISTS (
                         SELECT 1 FROM export_jobs WHERE artifact_id=artifact.id
                       )
                     ORDER BY created_at_ms,id LIMIT ?2",
                )
                .map_err(|error| format!("Cannot prepare artifact retention query: {error}"))?;
            statement
                .query_map(params![cutoff, limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(|error| format!("Cannot query artifact retention candidates: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Cannot decode artifact retention candidate: {error}"))?
        };
        let mut result = ArtifactGarbageCollection {
            candidates: candidates.len(),
            ..ArtifactGarbageCollection::default()
        };
        for (id, digest, relative, byte_length, state) in candidates {
            let expected_length =
                u64::try_from(byte_length).map_err(|_| "Invalid retained artifact byte length")?;
            validate_repository_path(&relative)
                .map_err(|_| "Unsafe retained artifact path".to_owned())?;
            let path = safe_join(&artifact_root, Path::new(&relative))?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    safe_artifact_path(&artifact_root, &relative)?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err("Retained artifact is not a regular file".into());
                    }
                    if state == "ready" {
                        verify_file(&path, expected_length, &digest)?;
                    }
                    fs::remove_file(&path)
                        .map_err(|error| format!("Cannot remove retained artifact: {error}"))?;
                    if let Some(parent) = path.parent() {
                        fs::File::open(parent)
                            .and_then(|directory| directory.sync_all())
                            .map_err(|error| {
                                format!("Cannot sync retained artifact directory: {error}")
                            })?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && state != "ready" => {}
                Err(error) => {
                    return Err(format!("Cannot inspect retained artifact: {error}"));
                }
            }
            let deleted = transaction
                .execute(
                    "DELETE FROM artifact_refs
                     WHERE id=?1
                       AND NOT EXISTS (
                         SELECT 1 FROM artifact_links WHERE artifact_id=artifact_refs.id
                       )
                       AND NOT EXISTS (
                         SELECT 1 FROM export_jobs WHERE artifact_id=artifact_refs.id
                       )",
                    [id],
                )
                .map_err(|error| format!("Cannot delete artifact metadata: {error}"))?;
            if deleted != 1 {
                return Err("Artifact became referenced during collection".into());
            }
            result.deleted += 1;
            result.reclaimed_bytes = result
                .reclaimed_bytes
                .checked_add(expected_length)
                .ok_or("Artifact reclaimed-byte counter overflow")?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit artifact retention: {error}"))?;
        Ok(result)
    }

    pub fn plan_closed_review_retention(
        &self,
        policy: &ClosedReviewRetentionPolicy,
        now_ms: u64,
    ) -> Result<ClosedReviewRetentionPlan, String> {
        closed_review_retention_plan(&self.connection, policy, now_ms)
    }

    pub fn apply_closed_review_retention(
        &mut self,
        policy: &ClosedReviewRetentionPolicy,
        now_ms: u64,
        principal: &str,
        expected_plan: &ClosedReviewRetentionPlan,
    ) -> Result<usize, String> {
        validate_label("retention principal", principal)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                format!("Cannot start closed-review retention transaction: {error}")
            })?;
        let current = closed_review_retention_plan(&transaction, policy, now_ms)?;
        if &current != expected_plan {
            return Err("Closed-review retention plan is stale".into());
        }
        let audit_detail = format!(
            "plan={};artifact_links={}",
            current.identity, current.artifact_links
        );
        let occurred_at =
            i64::try_from(now_ms).map_err(|_| "Retention time exceeds SQLite range")?;
        let mut released = 0;
        for candidate in &current.candidates {
            let deleted = transaction
                .execute(
                    "DELETE FROM artifact_links WHERE owner_kind='review' AND owner_id=?1",
                    [&candidate.review_id],
                )
                .map_err(|error| format!("Cannot release closed-review artifacts: {error}"))?;
            if deleted != candidate.artifact_links {
                return Err("Closed-review artifact references changed during retention".into());
            }
            let security_domain: String = transaction
                .query_row(
                    "SELECT repo.security_domain FROM reviews review
                     JOIN repositories repo ON repo.id=review.repository_id
                     WHERE review.id=?1",
                    [&candidate.review_id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("Cannot read retention security domain: {error}"))?;
            transaction
                .execute(
                    "INSERT INTO audit_events(security_domain,principal,action,object_kind,
                     object_id,occurred_at_ms,detail) VALUES (?1,?2,'retention_release',
                     'review',?3,?4,?5)",
                    params![
                        security_domain,
                        principal,
                        candidate.review_id,
                        occurred_at,
                        audit_detail
                    ],
                )
                .map_err(|error| format!("Cannot audit closed-review retention: {error}"))?;
            released += deleted;
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit closed-review retention: {error}"))?;
        Ok(released)
    }

    pub fn reconcile_artifacts(&mut self) -> Result<Reconciliation, String> {
        let mut reconciliation = Reconciliation::default();
        let mut files = Vec::new();
        collect_files(&self.artifact_root, &mut files, 100_000)?;
        for path in &files {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".tmp-"))
            {
                fs::remove_file(path)
                    .map_err(|error| format!("Cannot remove abandoned artifact: {error}"))?;
                reconciliation.temporary_files_removed += 1;
            }
        }

        let references = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT id,digest,relative_path,byte_length,format_version FROM artifact_refs",
                )
                .map_err(|error| format!("Cannot query artifact references: {error}"))?;
            statement
                .query_map([], artifact_from_row)
                .map_err(|error| format!("Cannot enumerate artifact references: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Cannot decode artifact reference: {error}"))?
        };
        let referenced = references
            .iter()
            .map(|reference| reference.relative_path.clone())
            .collect::<BTreeSet<_>>();
        for reference in references {
            let path = safe_join(&self.artifact_root, &reference.relative_path)?;
            let state = match verify_file(&path, reference.byte_length, &reference.digest) {
                Ok(()) => "ready",
                Err(_) if !path.exists() => {
                    reconciliation.missing_references += 1;
                    "missing"
                }
                Err(_) => {
                    reconciliation.corrupt_references += 1;
                    "corrupt"
                }
            };
            self.connection
                .execute(
                    "UPDATE artifact_refs SET state=?1 WHERE id=?2",
                    params![state, reference.id],
                )
                .map_err(|error| format!("Cannot update artifact state: {error}"))?;
        }
        for path in files {
            if path.exists()
                && path
                    .strip_prefix(&self.artifact_root)
                    .ok()
                    .is_some_and(|relative| {
                        !relative
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(".tmp-"))
                            && !referenced.contains(relative)
                    })
            {
                reconciliation.orphaned_files += 1;
            }
        }
        Ok(reconciliation)
    }
}

fn apply_migrations(connection: &mut Connection) -> Result<(), String> {
    let current: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("Cannot read schema compatibility version: {error}"))?;
    let supported = MIGRATIONS.last().map_or(0, |migration| migration.0);
    if current > supported {
        return Err(format!(
            "Database schema {current} is newer than supported schema {supported}"
        ));
    }
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                applied_at_ms INTEGER NOT NULL
            );",
        )
        .map_err(|error| format!("Cannot initialize migrations: {error}"))?;
    for (version, name, sql) in MIGRATIONS {
        let applied: Option<String> = connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version=?1",
                [version],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Cannot inspect migration {version}: {error}"))?;
        if let Some(applied) = applied {
            if applied != *name {
                return Err(format!("Migration {version} name mismatch"));
            }
            if *version > current {
                return Err(format!(
                    "Migration {version} is recorded beyond schema version {current}"
                ));
            }
            continue;
        }
        if *version <= current {
            return Err(format!(
                "Schema version {current} is missing migration record {version}"
            ));
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start migration {version}: {error}"))?;
        transaction
            .execute_batch(sql)
            .map_err(|error| format!("Migration {version} failed: {error}"))?;
        transaction
            .execute(
                "INSERT INTO schema_migrations(version,name,applied_at_ms) VALUES (?1,?2,?3)",
                params![version, name, now_millis()],
            )
            .map_err(|error| format!("Cannot record migration {version}: {error}"))?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(|error| format!("Cannot set schema version {version}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit migration {version}: {error}"))?;
    }
    Ok(())
}

fn closed_review_retention_plan(
    connection: &Connection,
    policy: &ClosedReviewRetentionPolicy,
    now_ms: u64,
) -> Result<ClosedReviewRetentionPlan, String> {
    if policy.minimum_closed_age_ms < 60_000
        || policy.maximum_reviews == 0
        || policy.maximum_reviews > 1_000
    {
        return Err("Invalid closed-review retention policy".into());
    }
    let cutoff = now_ms
        .checked_sub(policy.minimum_closed_age_ms)
        .ok_or("Closed-review retention cutoff precedes the Unix epoch")?;
    let cutoff =
        i64::try_from(cutoff).map_err(|_| "Closed-review retention cutoff exceeds SQLite range")?;
    i64::try_from(now_ms).map_err(|_| "Retention time exceeds SQLite range")?;
    let limit = i64::try_from(policy.maximum_reviews)
        .map_err(|_| "Closed-review retention limit exceeds SQLite range")?;
    let mut statement = connection
        .prepare(
            "WITH latest AS (
               SELECT repository_id,provider_pr_id,max(generation) AS generation
               FROM pr_revisions GROUP BY repository_id,provider_pr_id
             ), closed_prs AS (
               SELECT revision.repository_id,revision.provider_pr_id,
                      revision.observed_at_ms AS closed_at_ms
               FROM pr_revisions revision JOIN latest
                 ON latest.repository_id=revision.repository_id
                AND latest.provider_pr_id=revision.provider_pr_id
                AND latest.generation=revision.generation
               WHERE revision.state='closed' AND revision.observed_at_ms<=?1
             ), candidates AS (
               SELECT history.review_id,max(closed_prs.closed_at_ms) AS closed_at_ms
               FROM pr_revisions history JOIN closed_prs
                 ON closed_prs.repository_id=history.repository_id
                AND closed_prs.provider_pr_id=history.provider_pr_id
               WHERE history.review_id IS NOT NULL GROUP BY history.review_id
             )
             SELECT candidate.review_id,candidate.closed_at_ms,count(link.artifact_id)
             FROM candidates candidate JOIN artifact_links link
               ON link.owner_kind='review' AND link.owner_id=candidate.review_id
             WHERE NOT EXISTS (
                 SELECT 1 FROM current_results WHERE review_id=candidate.review_id
               )
               AND NOT EXISTS (
                 SELECT 1 FROM pr_heads WHERE current_review_id=candidate.review_id
               )
               AND NOT EXISTS (
                 SELECT 1 FROM jobs WHERE review_id=candidate.review_id
                   AND state IN ('queued','retry','running')
               )
               AND NOT EXISTS (
                 SELECT 1 FROM pr_revisions shared
                 WHERE shared.review_id=candidate.review_id
                   AND shared.generation=(
                     SELECT max(current.generation) FROM pr_revisions current
                     WHERE current.repository_id=shared.repository_id
                       AND current.provider_pr_id=shared.provider_pr_id
                   )
                   AND shared.state!='closed'
               )
             GROUP BY candidate.review_id,candidate.closed_at_ms
             ORDER BY candidate.closed_at_ms,candidate.review_id LIMIT ?2",
        )
        .map_err(|error| format!("Cannot prepare closed-review retention plan: {error}"))?;
    let candidates = statement
        .query_map(params![cutoff, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| format!("Cannot query closed-review retention plan: {error}"))?
        .map(|row| {
            let (review_id, closed_at_ms, artifact_links) =
                row.map_err(|error| format!("Cannot decode closed-review candidate: {error}"))?;
            Ok(ClosedReviewRetentionCandidate {
                review_id,
                closed_at_ms: u64::try_from(closed_at_ms)
                    .map_err(|_| "Invalid closed-review timestamp")?,
                artifact_links: usize::try_from(artifact_links)
                    .map_err(|_| "Invalid closed-review artifact count")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let artifact_links = candidates.iter().try_fold(0usize, |total, candidate| {
        total
            .checked_add(candidate.artifact_links)
            .ok_or("Closed-review artifact count overflow")
    })?;
    let mut identity = b"closed-review-retention-v1".to_vec();
    identity.extend_from_slice(&policy.minimum_closed_age_ms.to_le_bytes());
    identity.extend_from_slice(&(policy.maximum_reviews as u64).to_le_bytes());
    identity.extend_from_slice(&now_ms.to_le_bytes());
    for candidate in &candidates {
        identity.extend_from_slice(&(candidate.review_id.len() as u64).to_le_bytes());
        identity.extend_from_slice(candidate.review_id.as_bytes());
        identity.extend_from_slice(&candidate.closed_at_ms.to_le_bytes());
        identity.extend_from_slice(&(candidate.artifact_links as u64).to_le_bytes());
    }
    Ok(ClosedReviewRetentionPlan {
        identity: hex_digest(&identity),
        candidates,
        artifact_links,
    })
}

fn artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRef> {
    let relative: String = row.get(2)?;
    let byte_length = u64::try_from(row.get::<_, i64>(3)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Integer, Box::new(error))
    })?;
    Ok(ArtifactRef {
        id: row.get(0)?,
        digest: row.get(1)?,
        relative_path: PathBuf::from(relative),
        byte_length,
        format_version: row.get(4)?,
    })
}

fn reject_symlink(path: &Path, kind: &str) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        Err(format!("{kind} must not be a symlink"))
    } else {
        Ok(())
    }
}

fn reject_new_backup_path(path: &Path, kind: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => return Err(format!("{kind} already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot inspect {kind}: {error}")),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| format!("{kind} requires an existing parent directory"))?;
    let metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("Cannot inspect {kind} directory: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("{kind} directory must be a real directory"));
    }
    Ok(())
}

fn safe_artifact_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_repository_path(relative).map_err(|_| "Unsafe backup artifact path".to_owned())?;
    let relative = Path::new(relative);
    let path = safe_join(root, relative)?;
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Cannot inspect backup artifact root: {error}"))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err("Backup artifact root must be a real directory".into());
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err("Unsafe backup artifact path".into());
        };
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| "Backup artifact is missing".to_owned())?;
        if metadata.file_type().is_symlink() {
            return Err("Backup artifact path must not contain symlinks".into());
        }
    }
    Ok(path)
}

fn validate_label(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Err(format!("Invalid {kind}"))
    } else {
        Ok(())
    }
}

fn validate_repository_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains(['\0', '\\'])
        || path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
        })
    {
        Err("Invalid repository-relative path".into())
    } else {
        Ok(())
    }
}

fn strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn encode_invalidation(
    cause: &crate::InvalidationCause,
) -> (&'static str, Option<&str>, Option<&str>) {
    match cause {
        crate::InvalidationCause::Body(path) => ("body", Some(path), None),
        crate::InvalidationCause::Signature(path) => ("signature", Some(path), None),
        crate::InvalidationCause::Imports(path) => ("imports", Some(path), None),
        crate::InvalidationCause::ModuleMove { from, to } => ("module_move", Some(from), Some(to)),
        crate::InvalidationCause::Manifest(path) => ("manifest", Some(path), None),
        crate::InvalidationCause::Contracts => ("contracts", None, None),
        crate::InvalidationCause::ResolverConfiguration => ("resolver_configuration", None, None),
    }
}

fn decode_invalidation(
    kind: &str,
    path: Option<String>,
    target: Option<String>,
) -> Result<crate::InvalidationCause, String> {
    let required_path = || -> Result<String, String> {
        path.clone()
            .ok_or_else(|| "Invalidation path is missing".into())
    };
    Ok(match kind {
        "body" => crate::InvalidationCause::Body(required_path()?),
        "signature" => crate::InvalidationCause::Signature(required_path()?),
        "imports" => crate::InvalidationCause::Imports(required_path()?),
        "module_move" => crate::InvalidationCause::ModuleMove {
            from: required_path()?,
            to: target.ok_or("Module move target is missing")?,
        },
        "manifest" => crate::InvalidationCause::Manifest(required_path()?),
        "contracts" => crate::InvalidationCause::Contracts,
        "resolver_configuration" => crate::InvalidationCause::ResolverConfiguration,
        _ => return Err("Unknown invalidation kind".into()),
    })
}

fn load_path_set(connection: &Connection, table: &str, id: &str) -> Result<Vec<String>, String> {
    if !matches!(table, "change_reused_paths" | "change_affected_callers") {
        return Err("Invalid change-set path table".into());
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT path FROM {table} WHERE change_set_id=?1 ORDER BY path"
        ))
        .map_err(|error| format!("Cannot prepare change-set paths: {error}"))?;
    statement
        .query_map([id], |row| row.get(0))
        .map_err(|error| format!("Cannot query change-set paths: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot decode change-set paths: {error}"))
}

fn validate_graph_label(kind: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        Err(format!("Invalid {kind}"))
    } else {
        Ok(())
    }
}

fn cursor_binding(snapshot_id: i64, query: &str, subject: &str) -> String {
    let identity = format!("{snapshot_id}\0{query}\0{subject}");
    hex_digest(identity.as_bytes())[..24].to_owned()
}

fn encode_cursor(prefix: &str, binding: &str, value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("{prefix}:{binding}:{encoded}")
}

fn decode_cursor(
    cursor: Option<&str>,
    prefix: &str,
    binding: &str,
    default: &str,
) -> Result<String, String> {
    let Some(cursor) = cursor else {
        return Ok(default.to_owned());
    };
    if cursor.len() > 8192 {
        return Err("Invalid graph cursor".into());
    }
    let mut parts = cursor.splitn(3, ':');
    if parts.next() != Some(prefix) || parts.next() != Some(binding) {
        return Err("Graph cursor does not belong to this query".into());
    }
    let encoded = parts.next().ok_or("Invalid graph cursor")?;
    if encoded.len() % 2 != 0 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid graph cursor".into());
    }
    let bytes = encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| "Invalid graph cursor")?;
            u8::from_str_radix(text, 16).map_err(|_| "Invalid graph cursor")
        })
        .collect::<Result<Vec<_>, _>>()?;
    String::from_utf8(bytes).map_err(|_| "Invalid graph cursor".into())
}

fn parse_version(version: &str) -> Result<(u32, u32, u32), String> {
    let parts = version
        .split('.')
        .map(|part| part.parse::<u32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("Invalid SQLite version: {version}"))?;
    match parts.as_slice() {
        [major, minor, patch, ..] => Ok((*major, *minor, *patch)),
        _ => Err(format!("Invalid SQLite version: {version}")),
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "Artifact path is not UTF-8".into())
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Unsafe artifact reference path".into());
    }
    Ok(root.join(relative))
}

fn verify_file(path: &Path, expected_length: u64, expected_digest: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("Cannot inspect artifact: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != expected_length
    {
        return Err("Artifact type or length mismatch".into());
    }
    let mut file =
        fs::File::open(path).map_err(|error| format!("Cannot open artifact: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Cannot read artifact: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in actual {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    if encoded == expected_digest {
        Ok(())
    } else {
        Err("Artifact digest mismatch".into())
    }
}

fn collect_files(root: &Path, output: &mut Vec<PathBuf>, limit: usize) -> Result<(), String> {
    if output.len() >= limit {
        return Err("Artifact reconciliation file limit exceeded".into());
    }
    for entry in
        fs::read_dir(root).map_err(|error| format!("Cannot read artifact root: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Cannot read artifact entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Cannot inspect artifact entry: {error}"))?;
        if file_type.is_dir() {
            collect_files(&entry.path(), output, limit)?;
        } else {
            output.push(entry.path());
            if output.len() >= limit {
                return Err("Artifact reconciliation file limit exceeded".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn paths(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "flowdiff-storage-test-{}-{}-{}",
            std::process::id(),
            name,
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        (root.join("flowdiff.db"), root.join("artifacts"), root)
    }

    #[test]
    fn fresh_database_has_supported_sqlite_and_all_migrations() {
        let (database, artifacts, root) = paths("fresh");
        let storage = Storage::open(&database, &artifacts).unwrap();
        println!("bundled SQLite runtime: {}", storage.sqlite_version());
        assert!(parse_version(storage.sqlite_version()).unwrap() >= MIN_SQLITE);
        assert_eq!(storage.schema_version().unwrap(), 14);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_manifest_restores_only_referenced_artifacts_and_detects_corruption() {
        let (database, artifacts, root) = paths("backup-restore");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let referenced = storage
            .publish_artifact("tenant-a", "graph-v1", b"referenced graph")
            .unwrap();
        let unreferenced = storage
            .publish_artifact("tenant-a", "graph-v1", b"unreferenced graph")
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO artifact_links(artifact_id,owner_kind,owner_id,purpose)
                 VALUES (?1,'review','review-one','graph')",
                [referenced.id],
            )
            .unwrap();

        let backup_dir = root.join("backup");
        fs::create_dir_all(&backup_dir).unwrap();
        let backup_database = backup_dir.join("state.db");
        let manifest = backup_dir.join("artifacts.manifest");
        storage
            .create_consistent_backup(&backup_database, &manifest)
            .unwrap();
        assert!(
            storage
                .create_consistent_backup(&backup_database, &manifest)
                .unwrap_err()
                .contains("already exists")
        );

        let manifest_text = fs::read_to_string(&manifest).unwrap();
        assert!(manifest_text.contains(&referenced.digest));
        assert!(!manifest_text.contains(&unreferenced.digest));

        let restored_artifacts = root.join("restored-artifacts");
        let restored_path = restored_artifacts.join(&referenced.relative_path);
        fs::create_dir_all(restored_path.parent().unwrap()).unwrap();
        fs::copy(artifacts.join(&referenced.relative_path), &restored_path).unwrap();
        assert_eq!(
            Storage::verify_backup(&backup_database, &restored_artifacts, &manifest).unwrap(),
            1
        );

        fs::write(&manifest, "codeshouldbehave-backup-manifest-v1\n").unwrap();
        assert!(
            Storage::verify_backup(&backup_database, &restored_artifacts, &manifest)
                .unwrap_err()
                .contains("omits referenced artifacts")
        );
        fs::write(&manifest, &manifest_text).unwrap();
        fs::remove_file(&restored_path).unwrap();
        assert!(
            Storage::verify_backup(&backup_database, &restored_artifacts, &manifest)
                .unwrap_err()
                .contains("missing")
        );
        fs::copy(artifacts.join(&referenced.relative_path), &restored_path).unwrap();
        fs::write(&restored_path, b"corrupt artifact").unwrap();
        assert!(
            Storage::verify_backup(&backup_database, &restored_artifacts, &manifest)
                .unwrap_err()
                .contains("length or digest")
        );
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn isolated_restore_preserves_historical_review_scope_and_commits() {
        let (database, artifacts, root) = paths("historical-restore");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let repository = storage
            .register_repository("tenant-a", "github", "acme", "api")
            .unwrap();
        let context_repository = storage
            .register_repository("tenant-a", "github", "acme", "contracts")
            .unwrap();
        let scope = storage
            .register_scope(repository, "scope-v1", b"include=src")
            .unwrap();
        let baseline_commit = "1111111111111111111111111111111111111111";
        let proposal_commit = "2222222222222222222222222222222222222222";
        let context_commit = "3333333333333333333333333333333333333333";
        storage.connection.execute(
            "INSERT INTO context_revisions(scope_id,position,repository_id,commit_id,path_scope)
             VALUES (?1,0,?2,?3,?4)",
            params![scope, context_repository, context_commit, b"include=schemas"],
        ).unwrap();
        let (baseline_snapshot, _) = storage
            .begin_snapshot(repository, baseline_commit, scope, "analyzer-v1")
            .unwrap();
        let (proposal_snapshot, _) = storage
            .begin_snapshot(repository, proposal_commit, scope, "analyzer-v1")
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO reviews(id,repository_id,selected_base_commit,analysis_base_commit,
             proposal_commit,comparison_mode,scope_id,baseline_snapshot_id,
             proposal_snapshot_id,created_at_ms)
             VALUES ('historical-review',?1,?2,?2,?3,'exact',?4,?5,?6,0)",
                params![
                    repository,
                    baseline_commit,
                    proposal_commit,
                    scope,
                    baseline_snapshot,
                    proposal_snapshot
                ],
            )
            .unwrap();
        let artifact = storage
            .publish_artifact("tenant-a", "graph-v1", b"historical graph")
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO artifact_links VALUES (?1,'review','historical-review','graph')",
                [artifact.id],
            )
            .unwrap();

        let backup_dir = root.join("historical-backup");
        fs::create_dir_all(&backup_dir).unwrap();
        let backup_database = backup_dir.join("state.db");
        let manifest = backup_dir.join("artifacts.manifest");
        storage
            .create_consistent_backup(&backup_database, &manifest)
            .unwrap();
        let restored_artifacts = root.join("historical-restored-artifacts");
        let restored_path = restored_artifacts.join(&artifact.relative_path);
        fs::create_dir_all(restored_path.parent().unwrap()).unwrap();
        fs::copy(artifacts.join(&artifact.relative_path), restored_path).unwrap();
        assert_eq!(
            Storage::verify_backup(&backup_database, &restored_artifacts, &manifest).unwrap(),
            1
        );

        let mut restored = Storage::open(&backup_database, &restored_artifacts).unwrap();
        assert_eq!(
            restored.reconcile_artifacts().unwrap(),
            Reconciliation::default()
        );
        let identity: (String, String, String, String, Vec<u8>) = restored
            .connection
            .query_row(
                "SELECT review.selected_base_commit,review.proposal_commit,scope.version,
                    context.commit_id,context.path_scope
             FROM reviews review JOIN scopes scope ON scope.id=review.scope_id
             JOIN context_revisions context ON context.scope_id=scope.id
             WHERE review.id='historical-review'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            identity,
            (
                baseline_commit.into(),
                proposal_commit.into(),
                "scope-v1".into(),
                context_commit.into(),
                b"include=schemas".to_vec(),
            )
        );
        drop(restored);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_collection_is_bounded_reference_safe_and_lease_aware() {
        let (database, artifacts, root) = paths("artifact-retention");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let repository = storage
            .register_repository("tenant-a", "github", "acme", "api")
            .unwrap();
        let scope = storage
            .register_scope(repository, "scope-v1", b"scope")
            .unwrap();
        let commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        storage
            .connection
            .execute(
                "INSERT INTO reviews(id,repository_id,selected_base_commit,analysis_base_commit,
             proposal_commit,comparison_mode,scope_id,created_at_ms)
             VALUES ('review-retention',?1,?2,?2,?2,'exact',?3,0)",
                params![repository, commit, scope],
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO jobs(review_id,kind,priority,state,available_at_ms,lease_owner,
             lease_expires_at_ms,lease_generation,attempt_count,enqueued_at_ms)
             VALUES ('review-retention','graph',1,'running',0,'worker-a',200000,1,1,0)",
                [],
            )
            .unwrap();

        let old_unreferenced = storage
            .publish_artifact("tenant-a", "graph-v1", b"old unreferenced")
            .unwrap();
        let old_referenced = storage
            .publish_artifact("tenant-a", "graph-v1", b"old referenced")
            .unwrap();
        let fresh_unreferenced = storage
            .publish_artifact("tenant-a", "graph-v1", b"fresh unreferenced")
            .unwrap();
        storage
            .connection
            .execute(
                "UPDATE artifact_refs SET created_at_ms=0 WHERE id IN (?1,?2)",
                params![old_unreferenced.id, old_referenced.id],
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO artifact_links(artifact_id,owner_kind,owner_id,purpose)
             VALUES (?1,'review','review-retention','graph')",
                [old_referenced.id],
            )
            .unwrap();
        let policy = ArtifactRetentionPolicy {
            minimum_unreferenced_age_ms: 60_000,
            maximum_deletions: 1,
        };
        assert!(
            storage
                .collect_unreferenced_artifacts(&policy, 100_000)
                .unwrap_err()
                .contains("active worker leases")
        );
        storage
            .connection
            .execute(
                "UPDATE jobs SET state='completed',lease_owner=NULL,lease_expires_at_ms=NULL",
                [],
            )
            .unwrap();
        let collection = storage
            .collect_unreferenced_artifacts(&policy, 100_000)
            .unwrap();
        assert_eq!(
            collection,
            ArtifactGarbageCollection {
                candidates: 1,
                deleted: 1,
                reclaimed_bytes: b"old unreferenced".len() as u64,
            }
        );
        assert!(!artifacts.join(old_unreferenced.relative_path).exists());
        assert!(artifacts.join(old_referenced.relative_path).exists());
        assert!(artifacts.join(fresh_unreferenced.relative_path).exists());
        let remaining: i64 = storage
            .connection
            .query_row("SELECT count(*) FROM artifact_refs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn closed_review_retention_requires_stable_plan_and_preserves_open_reuse() {
        let (database, artifacts, root) = paths("closed-review-retention");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let repository = storage
            .register_repository("tenant-a", "github", "acme", "api")
            .unwrap();
        let scope = storage
            .register_scope(repository, "scope-v1", b"scope")
            .unwrap();
        for (review, commit) in [
            (
                "review-eligible",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            ("review-shared", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ] {
            storage.connection.execute(
                "INSERT INTO reviews(id,repository_id,selected_base_commit,analysis_base_commit,
                 proposal_commit,comparison_mode,scope_id,created_at_ms)
                 VALUES (?1,?2,?3,?3,?3,'exact',?4,0)",
                params![review, repository, commit, scope],
            ).unwrap();
        }
        storage
            .connection
            .execute_batch(
                "INSERT INTO pr_revisions VALUES
             (1,'closed-only',1,'review-eligible','base','head','main','open',0),
             (1,'closed-only',2,NULL,'base','head','main','closed',0),
             (1,'closed-shared',1,'review-shared','base','head','main','open',0),
             (1,'closed-shared',2,NULL,'base','head','main','closed',0),
             (1,'still-open',1,'review-shared','base','head','main','open',0);",
            )
            .unwrap();
        let eligible_one = storage
            .publish_artifact("tenant-a", "graph-v1", b"eligible one")
            .unwrap();
        let eligible_two = storage
            .publish_artifact("tenant-a", "graph-v1", b"eligible two")
            .unwrap();
        let shared = storage
            .publish_artifact("tenant-a", "graph-v1", b"shared open review")
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO artifact_links VALUES (?1,'review','review-eligible','graph')",
                [eligible_one.id],
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO artifact_links VALUES (?1,'review','review-shared','graph')",
                [shared.id],
            )
            .unwrap();
        storage
            .connection
            .execute("UPDATE artifact_refs SET created_at_ms=0", [])
            .unwrap();
        let policy = ClosedReviewRetentionPolicy {
            minimum_closed_age_ms: 60_000,
            maximum_reviews: 10,
        };
        let stale = storage
            .plan_closed_review_retention(&policy, 100_000)
            .unwrap();
        assert_eq!(stale.candidates.len(), 1);
        assert_eq!(stale.candidates[0].review_id, "review-eligible");
        storage
            .connection
            .execute(
                "INSERT INTO artifact_links VALUES (?1,'review','review-eligible','flow')",
                [eligible_two.id],
            )
            .unwrap();
        assert!(
            storage
                .apply_closed_review_retention(&policy, 100_000, "operator-a", &stale)
                .unwrap_err()
                .contains("stale")
        );
        let current = storage
            .plan_closed_review_retention(&policy, 100_000)
            .unwrap();
        assert_eq!(current.artifact_links, 2);
        assert_eq!(
            storage
                .apply_closed_review_retention(&policy, 100_000, "operator-a", &current)
                .unwrap(),
            2
        );
        let eligible_links: i64 = storage
            .connection
            .query_row(
                "SELECT count(*) FROM artifact_links WHERE owner_id='review-eligible'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let shared_links: i64 = storage
            .connection
            .query_row(
                "SELECT count(*) FROM artifact_links WHERE owner_id='review-shared'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let audits: i64 = storage
            .connection
            .query_row(
                "SELECT count(*) FROM audit_events WHERE action='retention_release'
             AND object_id='review-eligible'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((eligible_links, shared_links, audits), (0, 1, 1));

        let collection = storage
            .collect_unreferenced_artifacts(
                &ArtifactRetentionPolicy {
                    minimum_unreferenced_age_ms: 60_000,
                    maximum_deletions: 10,
                },
                100_000,
            )
            .unwrap();
        assert_eq!(collection.deleted, 2);
        assert!(artifacts.join(shared.relative_path).exists());
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upgrades_a_version_one_fixture() {
        let (database, artifacts, root) = paths("upgrade");
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    applied_at_ms INTEGER NOT NULL
                );",
            )
            .unwrap();
        connection.execute_batch(MIGRATIONS[0].2).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations VALUES (1,'0001_foundation',0)",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);
        let storage = Storage::open(&database, &artifacts).unwrap();
        assert_eq!(storage.schema_version().unwrap(), 14);
        let index_exists: i64 = storage
            .connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='jobs_claim_order'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_exists, 1);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_audit_is_append_only_metadata_without_payloads() {
        let (database, artifacts, root) = paths("ai-audit");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        storage.connection.execute("INSERT INTO repositories(security_domain,provider,owner,name,created_at_ms) VALUES ('tenant','github','acme','api',0)", []).unwrap();
        storage.connection.execute("INSERT INTO scopes(repository_id,version,canonical,created_at_ms) VALUES (1,'scope',x'01',0)", []).unwrap();
        storage.connection.execute("INSERT INTO reviews(id,repository_id,selected_base_commit,analysis_base_commit,proposal_commit,comparison_mode,scope_id,created_at_ms) VALUES ('review-ai',1,?1,?1,?1,'exact',1,0)", ["a".repeat(40)]).unwrap();
        let first = storage
            .append_ai_audit(
                "review-ai",
                "bundle-1",
                "reviewer",
                "fake-local",
                "code_content",
                "disabled",
                "prepared",
                None,
                None,
                "rules-1",
                2,
                80,
                1,
            )
            .unwrap();
        let second = storage
            .append_ai_audit(
                "review-ai",
                "bundle-1",
                "reviewer",
                "fake-local",
                "code_content",
                "disabled",
                "disabled",
                None,
                None,
                "rules-1",
                2,
                80,
                2,
            )
            .unwrap();
        assert!(second > first);
        let records = storage.ai_audit_for_review("review-ai", 10).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].provider, None);
        let columns = storage
            .connection
            .prepare("PRAGMA table_info(ai_audit_events)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !columns
                .iter()
                .any(|name| matches!(name.as_str(), "prompt" | "source" | "result" | "credential"))
        );
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    fn graph_fixture(storage: &mut Storage) -> i64 {
        storage
            .connection
            .execute(
                "INSERT INTO repositories(security_domain,provider,owner,name,created_at_ms)
             VALUES ('tenant-a','github','acme','api',0)",
                [],
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO scopes(repository_id,version,canonical,created_at_ms)
             VALUES (1,'scope-v1',x'01',0)",
                [],
            )
            .unwrap();
        let (snapshot, inserted) = storage
            .begin_snapshot(
                1,
                "0123456789012345678901234567890123456789",
                1,
                "analyzer-v1",
            )
            .unwrap();
        assert!(inserted);
        storage
            .record_file_chunk(
                snapshot,
                &[
                    StoredFileProgress {
                        path: "src/a.rs".into(),
                        blob_id: Some("a".into()),
                        coverage: "indexed".into(),
                        detail: None,
                    },
                    StoredFileProgress {
                        path: "src/b.rs".into(),
                        blob_id: Some("b".into()),
                        coverage: "indexed".into(),
                        detail: None,
                    },
                    StoredFileProgress {
                        path: "private/secret.rs".into(),
                        blob_id: Some("s".into()),
                        coverage: "unsupported".into(),
                        detail: Some("unsupported language".into()),
                    },
                ],
                true,
            )
            .unwrap();
        storage
            .record_graph_paths(
                snapshot,
                &[
                    "src/a.rs".into(),
                    "src/b.rs".into(),
                    "private/secret.rs".into(),
                ],
            )
            .unwrap();
        let symbol = |stable_key: &str, path: &str, name: &str| StoredSymbol {
            stable_key: stable_key.into(),
            correspondence_key: Some(name.into()),
            path: path.into(),
            kind: "function".into(),
            name: name.into(),
            start_line: Some(1),
            start_column: Some(1),
            end_line: Some(2),
            end_column: Some(1),
        };
        storage
            .record_graph_chunk(
                snapshot,
                &[
                    symbol("a::caller", "src/a.rs", "caller"),
                    symbol("b::first", "src/b.rs", "first"),
                    symbol("b::second", "src/b.rs", "second"),
                    symbol("secret::target", "private/secret.rs", "target"),
                ],
                &[
                    StoredEdge {
                        stable_key: "edge-1".into(),
                        from_symbol: Some("a::caller".into()),
                        to_symbol: Some("b::first".into()),
                        kind: "call".into(),
                        confidence: 1.0,
                        resolution: "resolved".into(),
                        source_path: "src/a.rs".into(),
                        source_span: Some((1, 1, 1, 6)),
                        provenance: b"parser".to_vec(),
                    },
                    StoredEdge {
                        stable_key: "edge-2".into(),
                        from_symbol: Some("a::caller".into()),
                        to_symbol: Some("secret::target".into()),
                        kind: "call".into(),
                        confidence: 0.5,
                        resolution: "ambiguous".into(),
                        source_path: "src/a.rs".into(),
                        source_span: None,
                        provenance: b"parser".to_vec(),
                    },
                    StoredEdge {
                        stable_key: "edge-3".into(),
                        from_symbol: Some("a::caller".into()),
                        to_symbol: Some("b::second".into()),
                        kind: "call".into(),
                        confidence: 1.0,
                        resolution: "resolved".into(),
                        source_path: "src/a.rs".into(),
                        source_span: None,
                        provenance: b"parser".to_vec(),
                    },
                ],
            )
            .unwrap();
        snapshot
    }

    #[test]
    fn graph_navigation_is_scoped_paginated_and_restart_safe() {
        let (database, artifacts, root) = paths("graph-navigation");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let snapshot = graph_fixture(&mut storage);
        assert_eq!(storage.finish_snapshot(snapshot).unwrap(), "partial");
        let scope = crate::PathScope::new(vec![".".into()], vec!["private".into()]).unwrap();

        let roots = storage
            .graph_children(snapshot, None, &scope, 10, None)
            .unwrap();
        assert_eq!(roots.paths, vec![("src".into(), "component".into())]);
        assert!(
            storage
                .record_graph_paths(snapshot, &["src".into()])
                .is_err()
        );
        let files = storage
            .graph_children(snapshot, Some("src"), &scope, 10, None)
            .unwrap();
        assert_eq!(files.paths.len(), 2);
        let first_symbol = storage
            .graph_symbols(snapshot, "src/b.rs", &scope, 1, None)
            .unwrap();
        assert_eq!(first_symbol.symbols[0].stable_key, "b::first");
        let second_symbol = storage
            .graph_symbols(
                snapshot,
                "src/b.rs",
                &scope,
                1,
                first_symbol.next_cursor.as_deref(),
            )
            .unwrap();
        assert_eq!(second_symbol.symbols[0].stable_key, "b::second");

        let first = storage
            .graph_neighborhood(
                snapshot,
                "a::caller",
                GraphDirection::Outgoing,
                &scope,
                1,
                None,
            )
            .unwrap();
        assert_eq!(first.edges[0].stable_key, "edge-1");
        assert_eq!(first.coverage, vec![("indexed".into(), 2)]);
        let cursor = first.next_cursor.clone().unwrap();
        let second = storage
            .graph_neighborhood(
                snapshot,
                "a::caller",
                GraphDirection::Outgoing,
                &scope,
                1,
                Some(&cursor),
            )
            .unwrap();
        assert_eq!(second.edges[0].stable_key, "edge-3");
        assert!(second.edges.iter().all(|edge| edge.stable_key != "edge-2"));
        assert!(
            storage
                .graph_neighborhood(
                    snapshot,
                    "a::caller",
                    GraphDirection::Incoming,
                    &scope,
                    1,
                    Some(&cursor)
                )
                .is_err()
        );
        drop(storage);

        let storage = Storage::open(&database, &artifacts).unwrap();
        let reopened = storage
            .graph_neighborhood(
                snapshot,
                "a::caller",
                GraphDirection::Outgoing,
                &scope,
                1,
                Some(&cursor),
            )
            .unwrap();
        assert_eq!(reopened, second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_snapshot_requests_reuse_one_completed_snapshot() {
        let (database, artifacts, root) = paths("snapshot-idempotency");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let snapshot = graph_fixture(&mut storage);
        storage.finish_snapshot(snapshot).unwrap();
        let (same, inserted) = storage
            .begin_snapshot(
                1,
                "0123456789012345678901234567890123456789",
                1,
                "analyzer-v1",
            )
            .unwrap();
        assert_eq!(same, snapshot);
        assert!(!inserted);
        let status: String = storage
            .connection
            .query_row("SELECT status FROM snapshots WHERE id=?1", [same], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "partial");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_identical_snapshot_claims_create_one_scan_identity() {
        let (database, artifacts, root) = paths("snapshot-concurrency");
        let storage = Storage::open(&database, &artifacts).unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO repositories(security_domain,provider,owner,name,created_at_ms)
             VALUES ('tenant-a','github','acme','api',0)",
                [],
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO scopes(repository_id,version,canonical,created_at_ms)
             VALUES (1,'scope-v1',x'01',0)",
                [],
            )
            .unwrap();
        drop(storage);

        let barrier = Arc::new(Barrier::new(4));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let database = database.clone();
            let artifacts = artifacts.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let mut storage = Storage::open(&database, &artifacts).unwrap();
                barrier.wait();
                storage
                    .begin_snapshot(
                        1,
                        "abcdefabcdefabcdefabcdefabcdefabcdefabcd",
                        1,
                        "analyzer-v1",
                    )
                    .unwrap()
            }));
        }
        let claims = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            claims
                .iter()
                .map(|claim| claim.0)
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );
        assert_eq!(claims.iter().filter(|claim| claim.1).count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn immutable_change_sets_round_trip_and_cannot_cross_snapshots() {
        let (database, artifacts, root) = paths("change-sets");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let repository = storage
            .register_repository("tenant-a", "github", "acme", "api")
            .unwrap();
        let scope = storage
            .register_scope(repository, "scope-v1", b"scope")
            .unwrap();
        let baseline_commit = "1111111111111111111111111111111111111111";
        let proposal_commit = "2222222222222222222222222222222222222222";
        let (baseline, _) = storage
            .begin_snapshot(repository, baseline_commit, scope, "analyzer-v1")
            .unwrap();
        let (proposal, _) = storage
            .begin_snapshot(repository, proposal_commit, scope, "analyzer-v1")
            .unwrap();
        let change_set = crate::ChangeSet {
            id: "change-set-one".into(),
            baseline: baseline_commit.into(),
            proposal: proposal_commit.into(),
            changes: vec![crate::PathChange::Modified {
                path: "src/lib.rs".into(),
            }],
            parse_paths: vec!["src/lib.rs".into()],
            reused_paths: vec!["src/caller.rs".into()],
            affected_unchanged_callers: vec!["src/caller.rs".into()],
            invalidations: vec![crate::InvalidationCause::Signature("src/lib.rs".into())],
            resolution: crate::ResolutionStrategy::Full {
                reasons: vec![crate::InvalidationCause::Signature("src/lib.rs".into())],
            },
        };
        assert!(
            storage
                .store_change_set(repository, baseline, proposal, &change_set)
                .unwrap()
        );
        assert!(
            !storage
                .store_change_set(repository, baseline, proposal, &change_set)
                .unwrap()
        );
        drop(storage);

        let mut storage = Storage::open(&database, &artifacts).unwrap();
        assert_eq!(storage.load_change_set(&change_set.id).unwrap(), change_set);
        let mut collision = change_set.clone();
        collision.proposal = "3333333333333333333333333333333333333333".into();
        assert!(
            storage
                .store_change_set(repository, baseline, proposal, &collision)
                .is_err()
        );
        assert!(
            storage
                .store_change_set(repository + 1, baseline, proposal, &change_set)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_duplicate_publication_has_one_reference() {
        let (database, artifacts, root) = paths("concurrent");
        drop(Storage::open(&database, &artifacts).unwrap());
        let barrier = Arc::new(Barrier::new(4));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let database = database.clone();
            let artifacts = artifacts.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let mut storage = Storage::open(&database, &artifacts).unwrap();
                barrier.wait();
                storage
                    .publish_artifact("tenant-a", "graph-v1", b"same artifact")
                    .unwrap()
                    .id
            }));
        }
        let ids = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconciliation_removes_temps_and_marks_corruption() {
        let (database, artifacts, root) = paths("reconcile");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let reference = storage
            .publish_artifact("tenant-a", "graph-v1", b"valid artifact")
            .unwrap();
        let abandoned = artifacts.join(".tmp-abandoned");
        fs::write(&abandoned, b"partial").unwrap();
        fs::write(artifacts.join(&reference.relative_path), b"corrupt").unwrap();
        let result = storage.reconcile_artifacts().unwrap();
        assert_eq!(result.temporary_files_removed, 1);
        assert_eq!(result.corrupt_references, 1);
        assert!(!abandoned.exists());
        let state: String = storage
            .connection
            .query_row(
                "SELECT state FROM artifact_refs WHERE id=?1",
                [reference.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "corrupt");
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_newer_schema_without_modifying_it() {
        let (database, artifacts, root) = paths("future-schema");
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);

        let error = Storage::open(&database, &artifacts).err().unwrap();
        assert!(error.contains("newer than supported"));
        let connection = Connection::open(&database).unwrap();
        let migration_table_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_table_count, 0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconciliation_marks_missing_and_counts_orphans() {
        let (database, artifacts, root) = paths("missing-orphan");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        let reference = storage
            .publish_artifact("tenant-a", "graph-v1", b"referenced")
            .unwrap();
        fs::remove_file(artifacts.join(&reference.relative_path)).unwrap();
        fs::write(artifacts.join("orphan.bin"), b"unreferenced").unwrap();

        let result = storage.reconcile_artifacts().unwrap();
        assert_eq!(result.missing_references, 1);
        assert_eq!(result.orphaned_files, 1);
        let state: String = storage
            .connection
            .query_row(
                "SELECT state FROM artifact_refs WHERE id=?1",
                [reference.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "missing");
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn immutable_keys_and_foreign_keys_reject_inconsistent_records() {
        let (database, artifacts, root) = paths("constraints");
        let storage = Storage::open(&database, &artifacts).unwrap();
        let foreign_key_error = storage.connection.execute(
            "INSERT INTO scopes(repository_id,version,canonical,created_at_ms) VALUES (999,'v1',x'00',0)",
            [],
        );
        assert!(foreign_key_error.is_err());

        storage
            .connection
            .execute(
                "INSERT INTO repositories(security_domain,provider,owner,name,created_at_ms) VALUES ('tenant-a','github','acme','api',0)",
                [],
            )
            .unwrap();
        let duplicate = storage.connection.execute(
            "INSERT INTO repositories(security_domain,provider,owner,name,created_at_ms) VALUES ('tenant-a','github','acme','api',1)",
            [],
        );
        assert!(duplicate.is_err());
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_progress_survives_restart_and_resumes_terminal_chunks() {
        let (database, artifacts, root) = paths("file-progress");
        let mut storage = Storage::open(&database, &artifacts).unwrap();
        storage.connection.execute_batch(
            "INSERT INTO repositories(id,security_domain,provider,owner,name,created_at_ms)
             VALUES (1,'tenant-a','github','acme','api',0);
             INSERT INTO scopes(id,repository_id,version,canonical,created_at_ms)
             VALUES (1,1,'scope-v1',x'00',0);
             INSERT INTO snapshots(id,repository_id,commit_id,scope_id,analyzer_version,status,created_at_ms)
             VALUES (1,1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,'reader-v1','pending',0);",
        ).unwrap();
        storage
            .record_file_chunk(
                1,
                &[
                    StoredFileProgress {
                        path: "src/indexed.rs".into(),
                        blob_id: Some("b".repeat(40)),
                        coverage: "indexed".into(),
                        detail: None,
                    },
                    StoredFileProgress {
                        path: "private/key.rs".into(),
                        blob_id: Some("c".repeat(40)),
                        coverage: "excluded".into(),
                        detail: Some("outside immutable scope".into()),
                    },
                ],
                false,
            )
            .unwrap();
        drop(storage);

        let mut resumed = Storage::open(&database, &artifacts).unwrap();
        let progress = resumed.load_file_progress(1).unwrap();
        assert_eq!(progress.len(), 2);
        assert_eq!(progress[0].coverage, "excluded");
        assert_eq!(progress[1].coverage, "indexed");
        resumed.record_file_chunk(1, &[], true).unwrap();
        let complete: i64 = resumed
            .connection
            .query_row(
                "SELECT enumeration_complete FROM snapshots WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(complete, 1);
        drop(resumed);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_storage_endpoints() {
        use std::os::unix::fs::symlink;

        let (database, artifacts, root) = paths("symlink");
        fs::create_dir_all(&root).unwrap();
        let real_artifacts = root.join("real-artifacts");
        fs::create_dir_all(&real_artifacts).unwrap();
        symlink(&real_artifacts, &artifacts).unwrap();
        let error = Storage::open(&database, &artifacts).err().unwrap();
        assert!(error.contains("Artifact root must not be a symlink"));
        fs::remove_dir_all(root).unwrap();
    }
}
