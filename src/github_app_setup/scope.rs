//! Scope confirmation is a versioned inactive intent, not an analysis grant.
use super::*;
use crate::PathScope;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
const MAX_ENTRIES: usize = 2000;
const PREVIEW_TTL: i64 = 900;
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InventoryEntry {
    pub path: String,
    pub kind: String,
    pub oid: String,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScopeInventory {
    pub repository_id: i64,
    pub default_branch: String,
    pub commit: String,
    pub tree: String,
    pub entries: Vec<InventoryEntry>,
}
impl ScopeInventory {
    pub(super) fn validate(&mut self, repository: i64) -> Result<()> {
        if self.repository_id != repository
            || !oid(&self.commit)
            || !oid(&self.tree)
            || self.default_branch.is_empty()
            || self.default_branch.len() > 256
            || self.default_branch.chars().any(char::is_control)
        {
            return Err(SetupError::Unavailable);
        }
        if self.entries.len() > MAX_ENTRIES {
            return Err(SetupError::Capacity);
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            if !path(&entry.path)
                || !oid(&entry.oid)
                || !matches!(
                    entry.kind.as_str(),
                    "file" | "directory" | "symlink" | "submodule"
                )
                || !seen.insert(&entry.path)
            {
                return Err(SetupError::Unavailable);
            }
        }
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(())
    }
}
pub(super) fn oid(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn path(s: &str) -> bool {
    s.len() <= 256
        && !s.is_empty()
        && !s.chars().any(char::is_control)
        && s != "."
        && s.split('/').all(|part| part != ".")
        && PathScope::new([s.to_string()], []).is_ok()
        && !s.ends_with('/')
}
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScopeInput {
    pub installation_id: i64,
    pub repository_id: i64,
    pub commit: String,
    pub expected_revision: i64,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScopeDocument {
    pub schema: String,
    pub installation_id: i64,
    pub repository_id: i64,
    pub commit: String,
    pub tree: String,
    pub default_branch: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub files: Vec<String>,
    pub rust_candidates: usize,
    pub unsupported_files: usize,
    pub automatically_excluded: usize,
    pub inventory_fingerprint: String,
    pub sensitive_path_policy: String,
    pub source_scanned: bool,
    pub architecture_confirmed: bool,
    pub publication_consent: bool,
}
#[derive(Serialize, Debug)]
pub struct ScopePreview {
    pub preview_id: String,
    pub expires_at_s: i64,
    pub expected_revision: i64,
    pub document: ScopeDocument,
    pub activation: &'static str,
}
#[derive(Serialize, Debug)]
pub struct SavedScope {
    pub revision: i64,
    pub actor_id: i64,
    pub confirmed_at_s: i64,
    pub document: ScopeDocument,
    pub activation: &'static str,
}
impl<P: GitHubSetupProvider> AppSetupHost<P> {
    pub(super) fn authorized_choice(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<RepositoryChoice> {
        self.repositories(token)?
            .into_iter()
            .find(|r| r.installation_id == installation && r.repository_id == repository)
            .ok_or(SetupError::Denied)
    }
    pub fn scope_inventory(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<ScopeInventory> {
        let choice = self.authorized_choice(token, installation, repository)?;
        let session = self.session(token, now()?)?;
        let mut inventory = self
            .provider
            .scope_inventory(&session.credential, &choice)?;
        inventory.validate(repository)?;
        // Permissions can change while metadata is being fetched.
        self.authorized_choice(token, installation, repository)?;
        Ok(inventory)
    }
    pub fn saved_scope(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<Option<SavedScope>> {
        self.authorized_choice(token, installation, repository)?;
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        fence(&sessions, token)?;
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        latest(
            &storage.connection,
            self.config.app_id,
            installation,
            repository,
        )
    }
    pub fn preview_scope(&self, token: &str, input: ScopeInput) -> Result<ScopePreview> {
        if !oid(&input.commit)
            || input.expected_revision < 0
            || input.include.len() > 32
            || input.exclude.len() > 32
            || input
                .include
                .iter()
                .chain(&input.exclude)
                .any(|p| p != "." && !path(p))
        {
            return Err(SetupError::Invalid);
        }
        let scope =
            PathScope::new(input.include, input.exclude).map_err(|_| SetupError::Invalid)?;
        let inventory = self.scope_inventory(token, input.installation_id, input.repository_id)?;
        if inventory.commit != input.commit {
            return Err(SetupError::Conflict);
        }
        for root in scope.includes().chain(scope.excludes()) {
            if root != "."
                && !inventory
                    .entries
                    .iter()
                    .any(|e| e.path == root || e.path.starts_with(&format!("{root}/")))
            {
                return Err(SetupError::Invalid);
            }
        }
        let selected = inventory
            .entries
            .iter()
            .filter(|e| scope.allows(&e.path) && e.kind != "directory")
            .collect::<Vec<_>>();
        let files = selected
            .iter()
            .filter(|e| e.kind == "file" && !crate::sensitive_path(&e.path))
            .map(|e| e.path.clone())
            .collect::<Vec<_>>();
        if files.is_empty() {
            return Err(SetupError::Invalid);
        }
        let rust_candidates = files.iter().filter(|p| p.ends_with(".rs")).count();
        let document = ScopeDocument {
            schema: "csb-app-scope-intent-v1".into(),
            installation_id: input.installation_id,
            repository_id: input.repository_id,
            commit: inventory.commit.clone(),
            tree: inventory.tree.clone(),
            default_branch: inventory.default_branch.clone(),
            include: scope.includes().map(str::to_owned).collect(),
            exclude: scope.excludes().map(str::to_owned).collect(),
            unsupported_files: files
                .iter()
                .filter(|p| !p.ends_with(".rs") && p.rsplit('/').next() != Some("Cargo.toml"))
                .count(),
            automatically_excluded: selected.len() - files.len(),
            files,
            rust_candidates,
            inventory_fingerprint: digest(
                &serde_json::to_string(&inventory).map_err(|_| SetupError::Unavailable)?,
            ),
            sensitive_path_policy: "sensitive-path-policy-v1".into(),
            source_scanned: false,
            architecture_confirmed: false,
            publication_consent: false,
        };
        let document_json =
            serde_json::to_string(&document).map_err(|_| SetupError::Unavailable)?;
        if document_json.len() > 524288 {
            return Err(SetupError::Capacity);
        }
        let preview_id = secret()?;
        let at = now()?;
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        let actor = fence(&sessions, token)?.credential.user_id;
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let saved = latest(
            &tx,
            self.config.app_id,
            input.installation_id,
            input.repository_id,
        )?;
        if saved.map_or(0, |s| s.revision) != input.expected_revision {
            return Err(SetupError::Conflict);
        }
        let selected:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_setup_repositories WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3)",params![self.config.app_id,input.installation_id,input.repository_id],|r|r.get(0)).map_err(db)?;
        if !selected {
            return Err(SetupError::Invalid);
        }
        tx.execute("DELETE FROM app_setup_scope_previews WHERE expires_at_s<=?1 OR (app_id=?2 AND installation_id=?3 AND repository_id=?4 AND actor_id=?5)",params![at,self.config.app_id,input.installation_id,input.repository_id,actor]).map_err(db)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM app_setup_scope_previews", [], |r| {
                r.get(0)
            })
            .map_err(db)?;
        if count >= 256 {
            return Err(SetupError::Capacity);
        }
        let expires_at_s = at + PREVIEW_TTL;
        tx.execute(
            "INSERT INTO app_setup_scope_previews VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                preview_id,
                self.config.app_id,
                input.installation_id,
                input.repository_id,
                actor,
                input.expected_revision,
                expires_at_s,
                document_json
            ],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(ScopePreview {
            preview_id,
            expires_at_s,
            expected_revision: input.expected_revision,
            document,
            activation: "off",
        })
    }
    pub fn confirm_scope(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
        preview: &str,
        revision: i64,
        acknowledge: bool,
    ) -> Result<SavedScope> {
        if !valid_secret(preview) || revision < 0 || !acknowledge {
            return Err(SetupError::Invalid);
        }
        let actor = self.session(token, now()?)?.credential.user_id;
        // Read only this user's preview; never expose another user's cached paths.
        let document = {
            let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            preview_document(
                &storage.connection,
                self.config.app_id,
                installation,
                repository,
                actor,
                preview,
                revision,
            )?
        };
        let inventory = self.scope_inventory(token, installation, repository)?;
        if document.inventory_fingerprint
            != digest(&serde_json::to_string(&inventory).map_err(|_| SetupError::Unavailable)?)
        {
            return Err(SetupError::Conflict);
        }
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        fence(&sessions, token)?;
        let at = now()?;
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        preview_document(
            &tx,
            self.config.app_id,
            installation,
            repository,
            actor,
            preview,
            revision,
        )?;
        if latest(&tx, self.config.app_id, installation, repository)?.map_or(0, |s| s.revision)
            != revision
        {
            return Err(SetupError::Conflict);
        }
        let stored_count: i64 = tx
            .query_row("SELECT count(*) FROM app_setup_scopes", [], |r| r.get(0))
            .map_err(db)?;
        if stored_count >= 1024 {
            return Err(SetupError::Capacity);
        }
        if revision >= 100 {
            return Err(SetupError::Capacity);
        }
        tx.execute("INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![self.config.app_id,installation,repository,revision+1,actor,at,serde_json::to_string(&document).map_err(|_|SetupError::Unavailable)?]).map_err(db)?;
        tx.execute(
            "DELETE FROM app_setup_scope_previews WHERE id=?1",
            [preview],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(SavedScope {
            revision: revision + 1,
            actor_id: actor,
            confirmed_at_s: at,
            document,
            activation: "off",
        })
    }
}
pub(super) fn fence<'a>(
    sessions: &'a BTreeMap<String, Session>,
    token: &str,
) -> Result<&'a Session> {
    let at = now()?;
    sessions
        .get(&digest(token))
        .filter(|s| s.expires_at_s > at)
        .ok_or(SetupError::Authentication)
}
pub(super) fn latest(
    dbh: &rusqlite::Connection,
    app: i64,
    installation: i64,
    repository: i64,
) -> Result<Option<SavedScope>> {
    unblocked(dbh, app, installation, repository)?;
    let row:Option<(i64,i64,i64,String)>=dbh.query_row("SELECT revision,actor_id,confirmed_at_s,document FROM app_setup_scopes WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 ORDER BY revision DESC LIMIT 1",params![app,installation,repository],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db)?;
    row.map(|(revision, actor_id, confirmed_at_s, json)| {
        Ok(SavedScope {
            revision,
            actor_id,
            confirmed_at_s,
            document: serde_json::from_str(&json).map_err(|_| SetupError::Unavailable)?,
            activation: "off",
        })
    })
    .transpose()
}
#[allow(clippy::too_many_arguments)]
fn preview_document(
    dbh: &rusqlite::Connection,
    app: i64,
    installation: i64,
    repository: i64,
    actor: i64,
    preview: &str,
    revision: i64,
) -> Result<ScopeDocument> {
    let row:Option<String>=dbh.query_row("SELECT document FROM app_setup_scope_previews WHERE id=?1 AND app_id=?2 AND installation_id=?3 AND repository_id=?4 AND actor_id=?5 AND expected_revision=?6 AND expires_at_s>?7",params![preview,app,installation,repository,actor,revision,now()?],|r|r.get(0)).optional().map_err(db)?;
    serde_json::from_str(&row.ok_or(SetupError::Conflict)?).map_err(|_| SetupError::Unavailable)
}

pub(super) fn unblocked(
    dbh: &rusqlite::Connection,
    app: i64,
    installation: i64,
    repository: i64,
) -> Result<()> {
    let blocked:bool=dbh.query_row("SELECT EXISTS(SELECT 1 FROM app_installation_blocks WHERE app_id=?1 AND installation_id=?2) OR EXISTS(SELECT 1 FROM app_repository_blocks WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3)",params![app,installation,repository],|r|r.get(0)).map_err(db)?;
    if blocked {
        Err(SetupError::Denied)
    } else {
        Ok(())
    }
}
