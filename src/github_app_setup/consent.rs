//! Final public-source consent is an immutable admin decision, never activation.
use super::*;
use crate::public_report::{PublicPolicy, canonical_json, sha256};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Clone, Serialize)]
pub struct PublicationOperatorPolicy {
    pub report_origin: String,
    pub engine_digest: String,
    pub engine_metadata_digest: String,
}
impl PublicationOperatorPolicy {
    pub fn new(
        report_origin: String,
        engine_digest: String,
        engine_metadata_digest: String,
    ) -> Result<Self> {
        SetupConfig::new(1, "consent".into(), report_origin.clone())?;
        if !valid_secret(&engine_digest) || !valid_secret(&engine_metadata_digest) {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            report_origin,
            engine_digest,
            engine_metadata_digest,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationConsentInput {
    pub installation_id: i64,
    pub repository_id: i64,
    pub expected_revision: i64,
    pub expires_at: i64,
    pub document_digest: String,
    pub allow_automatic_analysis: bool,
    pub allow_public_source: bool,
    pub allow_checks: bool,
}
impl<P: GitHubSetupProvider> AppSetupHost<P> {
    pub fn with_publication_policy(mut self, policy: PublicationOperatorPolicy) -> Result<Self> {
        PublicationOperatorPolicy::new(
            policy.report_origin.clone(),
            policy.engine_digest.clone(),
            policy.engine_metadata_digest.clone(),
        )?;
        if policy
            .report_origin
            .eq_ignore_ascii_case(&self.config.origin)
        {
            return Err(SetupError::Invalid);
        }
        self.publication_policy = Some(policy);
        Ok(self)
    }
    fn publication_document(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
        expires: i64,
    ) -> Result<Value> {
        let operator = self
            .publication_policy
            .as_ref()
            .ok_or(SetupError::Unavailable)?;
        let choice = self.authorized_choice(token, installation, repository)?;
        let actor = self.session(token, now()?)?.credential.user_id;
        let saved = self
            .saved_scope(token, installation, repository)?
            .ok_or(SetupError::Conflict)?;
        let inventory = self.scope_inventory(token, installation, repository)?;
        if digest(&serde_json::to_string(&inventory).map_err(|_| SetupError::Invalid)?)
            != saved.document.inventory_fingerprint
        {
            return Err(SetupError::Conflict);
        }
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        scope::unblocked(
            &storage.connection,
            self.config.app_id,
            installation,
            repository,
        )?;
        let context:String=storage.connection.query_row("SELECT context FROM app_architecture_acceptances WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND scope_revision=?4",params![self.config.app_id,installation,repository,saved.revision],|r|r.get(0)).optional().map_err(db)?.ok_or(SetupError::Conflict)?;
        let expected:i64=storage.connection.query_row("SELECT coalesce(max(revision),0) FROM app_publication_consents WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.config.app_id,installation,repository],|r|r.get(0)).map_err(db)?;
        let policy:PublicPolicy=serde_json::from_value(json!({"schema_version":"csb-public-policy-v1","repository":{"id":repository,"owner":choice.owner,"name":choice.name},"public":true,"publication_consent":true,"include":saved.document.include,"exclude":saved.document.exclude,"baseline_mode":"merge_base","max_details":100})).map_err(|_|SetupError::Invalid)?;
        policy.validate().map_err(|_| SetupError::Invalid)?;
        Ok(
            json!({"schema":"csb-app-publication-consent-v1","app_id":self.config.app_id,"installation_id":installation,"repository_id":repository,"actor_id":actor,"expected_revision":expected,"scope_revision":saved.revision,"scope_commit":saved.document.commit,"context_digest":sha256(context.as_bytes()),"operator":operator,"policy":policy,"expires_at":expires,"retention_days":7,"source_visibility":"public authorized excerpts","analysis":"static only; incomplete coverage requires review","limits":{"unique_blobs":64,"blob_bytes":262144,"report_bytes":8388608,"job_rest_reservations":384,"installation_daily_rest_reservations":2000},"activation":"off"}),
        )
    }
    pub fn publication_consent_status(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<Value> {
        self.authorized_choice(token, installation, repository)?;
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        scope::fence(&sessions, token)?;
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let latest = scope::latest(&s.connection, self.config.app_id, installation, repository)?
            .map(|r| r.revision);
        let row:Option<(i64,String,bool)>=s.connection.query_row("SELECT revision,document,revoked FROM app_publication_consents WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 ORDER BY revision DESC LIMIT 1",params![self.config.app_id,installation,repository],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db)?;
        let Some((revision, doc, revoked)) = row else {
            return Ok(json!({"publication_consent":false,"activation":"off"}));
        };
        let doc: Value = serde_json::from_str(&doc).map_err(|_| SetupError::Invalid)?;
        let current = latest == doc["scope_revision"].as_i64()
            && self
                .publication_policy
                .as_ref()
                .is_some_and(|p| serde_json::to_value(p).ok().as_ref() == Some(&doc["operator"]));
        Ok(
            json!({"consent_revision":revision,"publication_consent":!revoked&&current,"revoked":revoked,"scope_current":current,"activation":"off"}),
        )
    }
    pub fn preview_publication_consent(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<Value> {
        let document = self.publication_document(token, installation, repository, now()? + 600)?;
        let hash = sha256(&canonical_json(&document).map_err(|_| SetupError::Invalid)?);
        Ok(json!({"document":document,"document_digest":hash,"activation":"off"}))
    }
    pub fn confirm_publication_consent(
        &self,
        token: &str,
        input: PublicationConsentInput,
    ) -> Result<Value> {
        let at = now()?;
        if !input.allow_automatic_analysis
            || !input.allow_public_source
            || !input.allow_checks
            || input.expires_at <= at
            || input.expires_at > at + 600
            || input.expected_revision < 0
        {
            return Err(SetupError::Invalid);
        }
        let doc = self.publication_document(
            token,
            input.installation_id,
            input.repository_id,
            input.expires_at,
        )?;
        let bytes = canonical_json(&doc).map_err(|_| SetupError::Invalid)?;
        if sha256(&bytes) != input.document_digest
            || doc["expected_revision"] != input.expected_revision
        {
            return Err(SetupError::Conflict);
        }
        // Fresh provider authorization precedes a session + scope/revision fence.
        self.authorized_choice(token, input.installation_id, input.repository_id)?;
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        scope::fence(&sessions, token)?;
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        scope::unblocked(
            &tx,
            self.config.app_id,
            input.installation_id,
            input.repository_id,
        )?;
        let latest = scope::latest(
            &tx,
            self.config.app_id,
            input.installation_id,
            input.repository_id,
        )?
        .ok_or(SetupError::Conflict)?;
        let revision:i64=tx.query_row("SELECT coalesce(max(revision),0) FROM app_publication_consents WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.config.app_id,input.installation_id,input.repository_id],|r|r.get(0)).map_err(db)?;
        if latest.revision != doc["scope_revision"].as_i64().ok_or(SetupError::Invalid)?
            || revision != input.expected_revision
            || now()? >= input.expires_at
        {
            return Err(SetupError::Conflict);
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM app_publication_consents", [], |r| {
                r.get(0)
            })
            .map_err(db)?;
        if count >= 10000 {
            return Err(SetupError::Capacity);
        }
        tx.execute("INSERT INTO app_publication_consents(app_id,installation_id,repository_id,revision,actor_id,created_at,document,digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![self.config.app_id,input.installation_id,input.repository_id,revision+1,doc["actor_id"].as_i64(),at,String::from_utf8(bytes).map_err(|_|SetupError::Invalid)?,input.document_digest]).map_err(db)?;
        disable(
            &tx,
            self.config.app_id,
            input.installation_id,
            input.repository_id,
        )?;
        tx.commit().map_err(db)?;
        Ok(
            json!({"consent_revision":revision+1,"document_digest":input.document_digest,"publication_consent":true,"activation":"off","operator_approval_required":true}),
        )
    }
    pub fn revoke_publication_consent(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<Value> {
        self.authorized_choice(token, installation, repository)?;
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        scope::fence(&sessions, token)?;
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute("UPDATE app_publication_consents SET revoked=1 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.config.app_id,installation,repository]).map_err(db)?;
        disable(&tx, self.config.app_id, installation, repository)?;
        tx.commit().map_err(db)?;
        Ok(json!({"publication_consent":false,"activation":"off","withdrawn":true}))
    }
}
fn disable(c: &rusqlite::Connection, app: i64, installation: i64, repository: i64) -> Result<()> {
    c.execute("UPDATE app_execution_grants SET enabled=0 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![app,installation,repository]).map_err(db)?;
    c.execute("UPDATE app_report_publications SET withdrawn=1 WHERE job_id IN (SELECT id FROM app_analysis_jobs WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3)",params![app,installation,repository]).map_err(db)?;
    c.execute("UPDATE app_analysis_jobs SET state='cancelled',lease=NULL,lease_until=NULL WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND state IN ('pending','leased','staged','published')",params![app,installation,repository]).map_err(db)?;
    Ok(())
}
