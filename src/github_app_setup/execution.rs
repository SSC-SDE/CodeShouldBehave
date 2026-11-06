//! Inactive-by-default execution primitives. No host drain loop or activation route.
use super::installation::CurrentPullRequest;
use super::*;
use crate::public_report::{PublicBundle, PublicPolicy, canonical_json, sha256};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Trusted broker adapter only. Must independently establish installation/repository
/// ownership, current public visibility and exact PR revisions on every call.
pub trait AppExecutionProvider {
    fn current_pr(&self, installation: i64, repository: i64, pr: i64)
    -> Result<CurrentPullRequest>;
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AnalysisInput {
    pub schema: String,
    pub app_id: i64,
    pub installation_id: i64,
    pub repository_id: i64,
    pub scope_revision: i64,
    pub consent_digest: String,
    pub engine_digest: String,
    pub engine_metadata_digest: String,
    pub comparison: CurrentPullRequest,
}
impl AnalysisInput {
    pub fn scope_version(&self) -> String {
        format!(
            "app-scope-v1:{}:{}:{}:{}:{}",
            self.app_id,
            self.installation_id,
            self.repository_id,
            self.scope_revision,
            self.consent_digest
        )
    }
    pub fn id(&self) -> Result<String> {
        Ok(sha256(
            &canonical_json(self).map_err(|_| SetupError::Invalid)?,
        ))
    }
}
pub struct AnalysisLease {
    pub input: AnalysisInput,
    id: String,
    lease: String,
}
pub struct AppExecutionQueue {
    storage: Mutex<Storage>,
    app_id: i64,
}
struct Grant {
    revision: i64,
    consent: String,
    engine: String,
    metadata: String,
    policy: PublicPolicy,
}
impl AppExecutionQueue {
    pub fn new(storage: Storage, app_id: i64) -> Result<Self> {
        if app_id <= 0 {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            storage: Mutex::new(storage),
            app_id,
        })
    }
    /// Resolves current state inside the serialized reconciliation transaction. A
    /// stale/out-of-order webhook can only trigger this fresh read, never supply SHAs.
    pub fn reconcile(
        &self,
        installation: i64,
        repository: i64,
        pr: i64,
        provider: &dyn AppExecutionProvider,
    ) -> Result<Option<String>> {
        self.reconcile_inner(installation, repository, pr, provider, None)
    }
    /// Consume a leased PR notification and enqueue its freshly resolved comparison
    /// in one transaction. Inbox acknowledgement cannot race ahead of job persistence.
    pub fn reconcile_notification(
        &self,
        notification: &WebhookNotification,
        provider: &dyn AppExecutionProvider,
    ) -> Result<Option<String>> {
        if notification.event != "pull_request"
            || !matches!(
                notification.action.as_str(),
                "opened"
                    | "reopened"
                    | "synchronize"
                    | "ready_for_review"
                    | "edited"
                    | "closed"
                    | "converted_to_draft"
            )
        {
            return Err(SetupError::Invalid);
        }
        self.reconcile_inner(
            notification.installation_id,
            notification.repository_id.ok_or(SetupError::Invalid)?,
            notification.pr_number.ok_or(SetupError::Invalid)?,
            provider,
            Some(notification),
        )
    }
    fn reconcile_inner(
        &self,
        installation: i64,
        repository: i64,
        pr: i64,
        provider: &dyn AppExecutionProvider,
        notification: Option<&WebhookNotification>,
    ) -> Result<Option<String>> {
        if installation <= 0 || repository <= 0 || pr <= 0 {
            return Err(SetupError::Invalid);
        }
        let at = now()?;
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        if let Some(n) = notification {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_webhook_inbox WHERE app_id=?1 AND delivery_id=?2 AND installation_id=?3 AND repository_id=?4 AND pr_number=?5 AND event=?6 AND action=?7 AND state='leased' AND lease_id=?8 AND lease_expires_at_s>?9)",params![self.app_id,n.delivery_id,installation,repository,pr,n.event,n.action,n.lease_id,at],|r|r.get(0)).map_err(db)?;
            if !valid {
                return Err(SetupError::Conflict);
            }
        }
        let grant = grant(&tx, self.app_id, installation, repository)?;
        let comparison = provider.current_pr(installation, repository, pr)?;
        validate_comparison(&comparison, repository, pr)?;
        let input = AnalysisInput {
            schema: "csb-app-analysis-v1".into(),
            app_id: self.app_id,
            installation_id: installation,
            repository_id: repository,
            scope_revision: grant.revision,
            consent_digest: grant.consent,
            engine_digest: grant.engine,
            engine_metadata_digest: grant.metadata,
            comparison,
        };
        let id = input.id()?;
        tx.execute("UPDATE app_analysis_jobs SET state='cancelled',lease=NULL,lease_until=NULL WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND pr_number=?4 AND state IN ('pending','leased','staged','published') AND (id!=?5 OR ?6)",params![self.app_id,installation,repository,pr,id,!input.comparison.open || input.comparison.draft]).map_err(db)?;
        if !input.comparison.open || input.comparison.draft {
            finish_notification(&tx, self.app_id, notification)?;
            tx.commit().map_err(db)?;
            return Ok(None);
        }
        let exists = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM app_analysis_jobs WHERE id=?1)",
                [&id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(db)?;
        if !exists {
            let count:i64=tx.query_row("SELECT count(*) FROM app_analysis_jobs WHERE state IN ('pending','leased','staged','published')",[],|r|r.get(0)).map_err(db)?;
            let tenant:i64=tx.query_row("SELECT count(*) FROM app_analysis_jobs WHERE app_id=?1 AND installation_id=?2 AND state IN ('pending','leased','staged','published')",params![self.app_id,installation],|r|r.get(0)).map_err(db)?;
            let total: i64 = tx
                .query_row("SELECT count(*) FROM app_analysis_jobs", [], |r| r.get(0))
                .map_err(db)?;
            if count >= 100 || tenant >= 25 || total >= 10000 {
                return Err(SetupError::Capacity);
            }
            tx.execute("INSERT INTO app_analysis_jobs(id,app_id,installation_id,repository_id,pr_number,consent_digest,input,state,created_at,available_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?8)",params![id,self.app_id,installation,repository,pr,input.consent_digest,serde_json::to_string(&input).map_err(|_| SetupError::Invalid)?,at]).map_err(db)?;
        }
        if exists {
            let state: String = tx
                .query_row(
                    "SELECT state FROM app_analysis_jobs WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(db)?;
            if state == "failed" {
                return Err(SetupError::Conflict);
            }
            if state == "cancelled" {
                let changed=tx.execute("UPDATE app_analysis_jobs SET state=CASE WHEN EXISTS(SELECT 1 FROM app_report_intents WHERE job_id=?1) THEN 'staged' ELSE 'pending' END,available_at=?2 WHERE id=?1 AND (attempts<3 OR EXISTS(SELECT 1 FROM app_report_intents WHERE job_id=?1))",params![id,at]).map_err(db)?;
                if changed != 1 {
                    return Err(SetupError::Conflict);
                }
            }
        }
        finish_notification(&tx, self.app_id, notification)?;
        tx.commit().map_err(db)?;
        Ok(Some(id))
    }
    pub fn claim(&self) -> Result<Option<AnalysisLease>> {
        self.claim_at(now()?)
    }
    fn claim_at(&self, at: i64) -> Result<Option<AnalysisLease>> {
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute("UPDATE app_analysis_jobs SET state=CASE WHEN attempts>=3 THEN 'failed' ELSE 'pending' END, available_at=?1+60,lease=NULL,lease_until=NULL WHERE state='leased' AND lease_until<=?1",[at]).map_err(db)?;
        let running: i64 = tx
            .query_row(
                "SELECT count(*) FROM app_analysis_jobs WHERE state='leased'",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        if running >= 2 {
            tx.commit().map_err(db)?;
            return Ok(None);
        }
        let candidates = {
            let mut q=tx.prepare("SELECT id,input FROM app_analysis_jobs j WHERE app_id=?1 AND state='pending' AND available_at<=?2 AND NOT EXISTS(SELECT 1 FROM app_analysis_jobs a WHERE a.app_id=j.app_id AND a.installation_id=j.installation_id AND a.repository_id=j.repository_id AND a.state='leased') ORDER BY created_at,id LIMIT 100").map_err(db)?;
            q.query_map(params![self.app_id, at], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?
        };
        for (id, raw) in candidates {
            let input: AnalysisInput =
                serde_json::from_str(&raw).map_err(|_| SetupError::Unavailable)?;
            if let Err(error) = current_grant(&tx, &input) {
                if error != SetupError::Denied {
                    return Err(error);
                }
                tx.execute(
                    "UPDATE app_analysis_jobs SET state='cancelled' WHERE id=?1",
                    [&id],
                )
                .map_err(db)?;
                continue;
            }
            let lease = secret()?;
            tx.execute("UPDATE app_analysis_jobs SET state='leased',attempts=attempts+1,lease=?2,lease_until=?3 WHERE id=?1",params![id,lease,at+900]).map_err(db)?;
            tx.commit().map_err(db)?;
            return Ok(Some(AnalysisLease { input, id, lease }));
        }
        tx.commit().map_err(db)?;
        Ok(None)
    }
    /// Explicit worker failure, with at most two delayed retries. An expired or
    /// superseded worker cannot change the new owner's state.
    pub fn fail(&self, lease: &AnalysisLease, retryable: bool) -> Result<()> {
        if lease.input.app_id != self.app_id {
            return Err(SetupError::Denied);
        }
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let at = now()?;
        lease_fence(&tx, lease, at)?;
        current_grant(&tx, &lease.input)?;
        tx.execute("UPDATE app_analysis_jobs SET state=CASE WHEN ?2 AND attempts<3 THEN 'pending' ELSE 'failed' END, available_at=?3+60*attempts,lease=NULL,lease_until=NULL WHERE id=?1",params![lease.id,retryable,at]).map_err(db)?;
        tx.commit().map_err(db)
    }
    /// Persist a validated signed publication intent, never a check URL yet.
    /// The caller must stage immutable bytes before this transaction; workers cannot sign.
    pub fn stage(
        &self,
        lease: &AnalysisLease,
        bundle: &PublicBundle,
        engine: &Value,
        origin: &str,
        signer: &AppReceiptSigner,
        provider: &dyn AppExecutionProvider,
    ) -> Result<SignedAppReceipt> {
        if lease.input.app_id != self.app_id {
            return Err(SetupError::Denied);
        }
        bundle.validate().map_err(|_| SetupError::Invalid)?;
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        lease_fence(&tx, lease, now()?)?;
        let grant = current_grant(&tx, &lease.input)?;
        recheck(&lease.input, provider)?;
        let m = &bundle.manifest;
        let artifact: Value = serde_json::from_slice(
            bundle
                .files
                .get(&m.routes.artifact)
                .ok_or(SetupError::Invalid)?,
        )
        .map_err(|_| SetupError::Invalid)?;
        if artifact["scope_manifest"]["include"] != json!(grant.policy.include)
            || artifact["scope_manifest"]["exclude"] != json!(grant.policy.exclude)
        {
            return Err(SetupError::Denied);
        }
        if m.repository != grant.policy.repository
            || m.pr_number != lease.input.comparison.number as u64
            || m.selected_baseline != lease.input.comparison.selected_base
            || m.baseline != lease.input.comparison.merge_base
            || m.proposal != lease.input.comparison.head
            || m.policy_digest != grant.policy.digest().map_err(|_| SetupError::Invalid)?
            || m.engine != *engine
            || sha256(&canonical_json(engine).map_err(|_| SetupError::Invalid)?)
                != lease.input.engine_metadata_digest
            || m.scope_version
                != format!(
                    "public-scope-{}",
                    sha256(lease.input.scope_version().as_bytes())
                )
        {
            return Err(SetupError::Conflict);
        }
        // Separate cookie-free HTTPS origin, without credentials, query or path.
        super::SetupConfig::new(self.app_id, "receipt".into(), origin.to_owned())?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT receipt_digest FROM app_report_intents ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        let receipt = Receipt {
            schema: "csb-app-receipt-v1".into(),
            job: lease.input.clone(),
            bundle_digest: sha256(&canonical_json(m).map_err(|_| SetupError::Invalid)?),
            report_id: m.public_report_id.clone(),
            focus_url: format!(
                "{origin}/workspace/#review={}&bundle={}&layout=architecture&side=diff&mode=focus&design=hld",
                m.public_report_id, m.public_report_id
            ),
            previous: previous.clone(),
            created_at: now()?,
            check: check_outcome(bundle)?,
        };
        let signed = signer.sign(receipt)?;
        let raw = serde_json::to_string(&signed).map_err(|_| SetupError::Invalid)?;
        let hash = sha256(raw.as_bytes());
        tx.execute("INSERT INTO app_report_intents(job_id,receipt,receipt_digest,predecessor,created_at) VALUES(?1,?2,?3,?4,?5)",params![lease.id,raw,hash,previous,now()?]).map_err(db)?;
        tx.execute(
            "UPDATE app_analysis_jobs SET state='staged',lease=NULL,lease_until=NULL WHERE id=?1",
            [&lease.id],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(signed)
    }
    /// Recovery reads only the immutable exact intent. This is not publication proof.
    pub fn intent(&self, id: &str, key: &[u8]) -> Result<SignedAppReceipt> {
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let raw:String=s.connection.query_row("SELECT receipt FROM app_report_intents i JOIN app_analysis_jobs j ON j.id=i.job_id WHERE i.job_id=?1 AND j.app_id=?2 AND j.state IN ('staged','published','delivered')",params![id,self.app_id],|r|r.get(0)).optional().map_err(db)?.ok_or(SetupError::Denied)?;
        let signed: SignedAppReceipt =
            serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
        signed.verify(key)?;
        if signed.receipt.job.id()? != id || signed.receipt.job.app_id != self.app_id {
            return Err(SetupError::Denied);
        }
        current_grant(&s.connection, &signed.receipt.job)?;
        Ok(signed)
    }
    /// A publisher must independently fetch *all* public bytes and reconstruct this
    /// bundle from the cookie-free origin. A manifest-only probe is insufficient.
    pub fn confirm_readback(
        &self,
        id: &str,
        key: &[u8],
        readback: &PublicBundle,
        provider: &dyn AppExecutionProvider,
    ) -> Result<String> {
        readback.validate().map_err(|_| SetupError::Invalid)?;
        let signed = self.intent(id, key)?;
        if sha256(&canonical_json(&readback.manifest).map_err(|_| SetupError::Invalid)?)
            != signed.receipt.bundle_digest
        {
            return Err(SetupError::Conflict);
        }
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        current_grant(&tx, &signed.receipt.job)?;
        recheck(&signed.receipt.job, provider)?;
        let changed=tx.execute("UPDATE app_analysis_jobs SET state=CASE WHEN state='staged' THEN 'published' ELSE state END WHERE id=?1 AND app_id=?2 AND state IN ('staged','published','delivered')",params![id,self.app_id]).map_err(db)?;
        if changed != 1 {
            return Err(SetupError::Conflict);
        }
        tx.commit().map_err(db)?;
        Ok(signed.receipt.focus_url)
    }
}
/// Check delivery is an outbox operation. Persist the create attempt BEFORE the
/// network write; an unknown result can only be reconciled, never blindly recreated.
pub trait AppCheckDelivery {
    fn find(&self, repository: i64, head: &str, payload: &Value) -> Result<Option<i64>>;
    fn create(&self, repository: i64, payload: &Value) -> Result<i64>;
}
impl AppExecutionQueue {
    pub fn deliver(
        &self,
        id: &str,
        key: &[u8],
        provider: &dyn AppExecutionProvider,
        writer: &dyn AppCheckDelivery,
    ) -> Result<i64> {
        let signed = self.intent(id, key)?;
        let input = &signed.receipt.job;
        let payload = json!({"name":"CodeShouldBehave / Architecture","head_sha":input.comparison.head,
            "external_id":format!("csb-app-v1:{id}"),"details_url":signed.receipt.focus_url,
            "status":"completed","conclusion":signed.receipt.check.conclusion,
            "output":{"title":signed.receipt.check.title,"summary":format!("[Open CodeShouldBehave report]({})\n\nStatic evidence within selected scope; runtime effects require review. This check does not approve merging.\n\nBaseline: `{}` → Proposal: `{}`",signed.receipt.focus_url,input.comparison.selected_base,input.comparison.head)}});
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        current_grant(&tx, input)?;
        recheck(input, provider)?;
        let row:Option<(bool,Option<i64>)>=tx.query_row("SELECT delivery_attempted,check_id FROM app_analysis_jobs WHERE id=?1 AND app_id=?2 AND state IN ('published','delivered')",params![id,self.app_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db)?;
        let (attempted, done) = row.ok_or(SetupError::Conflict)?;
        if let Some(done) = done {
            return Ok(done);
        }
        let found = writer.find(input.repository_id, &input.comparison.head, &payload)?;
        let check = if let Some(found) = found {
            tx.commit().map_err(db)?;
            found
        } else {
            if attempted {
                return Err(SetupError::Conflict);
            }
            tx.execute(
                "UPDATE app_analysis_jobs SET delivery_attempted=1 WHERE id=?1",
                [id],
            )
            .map_err(db)?;
            tx.commit().map_err(db)?;
            // Another process cannot create again: the durable attempted bit is already set.
            let gate = s
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            current_grant(&gate, input)?;
            recheck(input, provider)?;
            let active:bool=gate.query_row("SELECT EXISTS(SELECT 1 FROM app_analysis_jobs WHERE id=?1 AND state='published')",[id],|r|r.get(0)).map_err(db)?;
            if !active {
                return Err(SetupError::Conflict);
            }
            let check = writer.create(input.repository_id, &payload)?;
            gate.commit().map_err(db)?;
            check
        };
        if check <= 0 {
            return Err(SetupError::Invalid);
        }
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        current_grant(&tx, input)?;
        recheck(input, provider)?;
        let n=tx.execute("UPDATE app_analysis_jobs SET state='delivered',check_id=?2 WHERE id=?1 AND state IN ('published','delivered')",params![id,check]).map_err(db)?;
        if n != 1 {
            return Err(SetupError::Conflict);
        }
        tx.commit().map_err(db)?;
        Ok(check)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckOutcome {
    conclusion: String,
    title: String,
}
fn check_outcome(bundle: &PublicBundle) -> Result<CheckOutcome> {
    let artifact: Value = serde_json::from_slice(
        bundle
            .files
            .get(&bundle.manifest.routes.artifact)
            .ok_or(SetupError::Invalid)?,
    )
    .map_err(|_| SetupError::Invalid)?;
    let hld = artifact["hld_status"].as_str().ok_or(SetupError::Invalid)?;
    let lld = artifact["lld_status"].as_str().ok_or(SetupError::Invalid)?;
    let c = &bundle.manifest.coverage;
    let complete = c.complete
        && c.gaps.is_empty()
        && c.deferred_details == 0
        && [hld, lld]
            .iter()
            .all(|s| matches!(*s, "change_detected" | "no_change_detected_within_scope"));
    let changed = [hld, lld].contains(&"change_detected");
    Ok(CheckOutcome {
        conclusion: if complete && !changed {
            "success"
        } else {
            "neutral"
        }
        .into(),
        title: if !complete {
            "Ready · Coverage incomplete"
        } else if changed {
            "Ready · Changes to review"
        } else {
            "No supported behavior change detected"
        }
        .into(),
    })
}
fn finish_notification(
    tx: &rusqlite::Connection,
    app: i64,
    n: Option<&WebhookNotification>,
) -> Result<()> {
    if let Some(n) = n {
        let changed=tx.execute("UPDATE app_webhook_inbox SET state='processed',lease_id=NULL,lease_expires_at_s=NULL WHERE app_id=?1 AND delivery_id=?2 AND state='leased' AND lease_id=?3 AND lease_expires_at_s>?4",params![app,n.delivery_id,n.lease_id,now()?]).map_err(db)?;
        if changed != 1 {
            return Err(SetupError::Conflict);
        }
    }
    Ok(())
}
fn grant(c: &rusqlite::Connection, app: i64, installation: i64, repository: i64) -> Result<Grant> {
    super::scope::unblocked(c, app, installation, repository)?;
    let row:Option<(i64,String,String,String,String)>=c.query_row("SELECT scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy FROM app_execution_grants WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND enabled=1",params![app,installation,repository],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(db)?;
    let (revision, consent, engine, metadata, raw) = row.ok_or(SetupError::Denied)?;
    let latest:i64=c.query_row("SELECT coalesce(max(revision),0) FROM app_setup_scopes WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![app,installation,repository],|r|r.get(0)).map_err(db)?;
    if revision != latest
        || !valid_secret(&consent)
        || !valid_secret(&engine)
        || !valid_secret(&metadata)
    {
        return Err(SetupError::Denied);
    }
    let policy: PublicPolicy = serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
    policy.validate().map_err(|_| SetupError::Denied)?;
    let raw_scope:String=c.query_row("SELECT document FROM app_setup_scopes WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND revision=?4",params![app,installation,repository,revision],|r|r.get(0)).map_err(db)?;
    let scope: ScopeDocument = serde_json::from_str(&raw_scope).map_err(|_| SetupError::Invalid)?;
    if scope.installation_id != installation
        || scope.repository_id != repository
        || scope.include != policy.include
        || scope.exclude != policy.exclude
    {
        return Err(SetupError::Denied);
    }
    if policy.repository.id != repository as u64 || policy.baseline_mode != "merge_base" {
        return Err(SetupError::Denied);
    }
    // Consent digest identifies the exact approved publication policy and scope version.
    if digest(&format!(
        "{revision}:{}",
        policy.digest().map_err(|_| SetupError::Invalid)?
    )) != consent
    {
        return Err(SetupError::Denied);
    }
    Ok(Grant {
        revision,
        consent,
        engine,
        metadata,
        policy,
    })
}
fn current_grant(c: &rusqlite::Connection, input: &AnalysisInput) -> Result<Grant> {
    let g = grant(c, input.app_id, input.installation_id, input.repository_id)?;
    if g.revision != input.scope_revision
        || g.consent != input.consent_digest
        || g.engine != input.engine_digest
        || g.metadata != input.engine_metadata_digest
    {
        return Err(SetupError::Denied);
    }
    Ok(g)
}
fn validate_comparison(p: &CurrentPullRequest, repo: i64, pr: i64) -> Result<()> {
    if p.repository_id != repo
        || p.number != pr
        || [&p.selected_base, &p.merge_base, &p.head].iter().any(|s| {
            s.len() != 40
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(SetupError::Invalid);
    }
    Ok(())
}
fn recheck(input: &AnalysisInput, provider: &dyn AppExecutionProvider) -> Result<()> {
    let fresh = provider.current_pr(
        input.installation_id,
        input.repository_id,
        input.comparison.number,
    )?;
    if fresh != input.comparison || !fresh.open || fresh.draft {
        return Err(SetupError::Conflict);
    }
    Ok(())
}
fn lease_fence(c: &rusqlite::Connection, lease: &AnalysisLease, at: i64) -> Result<()> {
    if lease.id != lease.input.id()? {
        return Err(SetupError::Conflict);
    }
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM app_analysis_jobs WHERE id=?1 AND state='leased' AND lease=?2 AND lease_until>?3)",params![lease.id,lease.lease,at],|r|r.get(0)).map_err(db)?;
    if !valid {
        Err(SetupError::Conflict)
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    job: AnalysisInput,
    bundle_digest: String,
    report_id: String,
    focus_url: String,
    previous: Option<String>,
    created_at: i64,
    check: CheckOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedAppReceipt {
    receipt: Receipt,
    signature: String,
}
pub struct AppReceiptSigner(Ed25519KeyPair);
impl AppReceiptSigner {
    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self> {
        Ed25519KeyPair::from_pkcs8(bytes)
            .map(Self)
            .map_err(|_| SetupError::Invalid)
    }
    pub fn public_key(&self) -> &[u8] {
        self.0.public_key().as_ref()
    }
    fn sign(&self, receipt: Receipt) -> Result<SignedAppReceipt> {
        let signature = URL_SAFE_NO_PAD.encode(
            self.0
                .sign(&canonical_json(&receipt).map_err(|_| SetupError::Invalid)?)
                .as_ref(),
        );
        Ok(SignedAppReceipt { receipt, signature })
    }
}
impl SignedAppReceipt {
    pub fn verify(&self, key: &[u8]) -> Result<()> {
        if self.receipt.schema != "csb-app-receipt-v1" {
            return Err(SetupError::Invalid);
        }
        let signature = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|_| SetupError::Invalid)?;
        UnparsedPublicKey::new(&ED25519, key)
            .verify(
                &canonical_json(&self.receipt).map_err(|_| SetupError::Invalid)?,
                &signature,
            )
            .map_err(|_| SetupError::Denied)
    }
    pub fn focus_url(&self) -> &str {
        &self.receipt.focus_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_report::{ExpectedComparison, export_public_report};
    use std::{
        cell::{Cell, RefCell},
        path::PathBuf,
    };
    struct Fixture {
        root: PathBuf,
        queue: AppExecutionQueue,
        provider: Provider,
        policy: PublicPolicy,
    }
    struct Provider(RefCell<CurrentPullRequest>);
    impl AppExecutionProvider for Provider {
        fn current_pr(
            &self,
            installation: i64,
            repository: i64,
            pr: i64,
        ) -> Result<CurrentPullRequest> {
            assert_eq!((installation, repository, pr), (2, 3, 7));
            Ok(self.0.borrow().clone())
        }
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("csb-app-execution-{}", secret().unwrap()));
            let storage = Storage::open(&root.join("state.db"), &root.join("artifacts")).unwrap();
            let policy:PublicPolicy=serde_json::from_value(json!({"schema_version":"csb-public-policy-v1","repository":{"id":3,"owner":"fixture","name":"app"},"public":true,"publication_consent":true,"include":["src"],"exclude":[],"baseline_mode":"merge_base","max_details":100})).unwrap();
            let document = json!({"schema":"csb-app-scope-v1","installation_id":2,"repository_id":3,"commit":"a".repeat(40),"tree":"b".repeat(40),"default_branch":"main","include":["src"],"exclude":[],"files":["src/lib.rs"],"rust_candidates":1,"unsupported_files":0,"automatically_excluded":0,"inventory_fingerprint":"a".repeat(64),"sensitive_path_policy":"v1","source_scanned":false,"architecture_confirmed":false,"publication_consent":false});
            storage.connection.execute("INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) VALUES(1,2,3,'fixture','app',4,1)",[]).unwrap();
            storage.connection.execute("INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) VALUES(1,2,3,1,4,1,?1)",[document.to_string()]).unwrap();
            storage.connection.execute("INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) VALUES(1,2,3,1,4,1,'{}')",[]).unwrap();
            // Test-only activation fixture. There is deliberately no production grant writer.
            storage.connection.execute("INSERT INTO app_execution_grants(app_id,installation_id,repository_id,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy) VALUES(1,2,3,1,?1,?2,?3,?4)",params![digest(&format!("1:{}",policy.digest().unwrap())),"a".repeat(64),sha256(&canonical_json(&engine()).unwrap()),serde_json::to_string(&policy).unwrap()]).unwrap();
            let queue = AppExecutionQueue::new(storage, 1).unwrap();
            let provider = Provider(RefCell::new(CurrentPullRequest {
                repository_id: 3,
                number: 7,
                selected_base: "a".repeat(40),
                merge_base: "a".repeat(40),
                head: "b".repeat(40),
                open: true,
                draft: false,
            }));
            Self {
                root,
                queue,
                provider,
                policy,
            }
        }
        fn sql(&self, sql: &str) {
            self.queue
                .storage
                .lock()
                .unwrap()
                .connection
                .execute_batch(sql)
                .unwrap();
        }
        fn enable(&self) {
            self.sql("UPDATE app_execution_grants SET enabled=1");
        }
        fn schedule(&self) -> Result<Option<String>> {
            self.queue.reconcile(2, 3, 7, &self.provider)
        }
        fn bundle(&self, input: &AnalysisInput) -> PublicBundle {
            let p = &input.comparison;
            let artifact = json!({"artifact_version":"csb-review-artifact-v1","review_id":"c".repeat(64),"selected_baseline":p.selected_base,"baseline":p.merge_base,"proposal":p.head,"scope_version":input.scope_version(),"scope_manifest":{"primary_repository":{"security_domain":"test","provider":"github","owner":"fixture","name":"app"},"include":["src"],"exclude":[],"contexts":[],"export_allowed":true},"files":[],"architecture":{"nodes":[],"edges":[]},"coverage":{"indexed":0,"unsupported":0,"excluded":0,"failed":0,"deferred":0,"enumeration_complete":true},"warnings":[]});
            export_public_report(
                &artifact,
                &self.policy,
                &ExpectedComparison {
                    repository: self.policy.repository.clone(),
                    pr_number: 7,
                    selected_baseline: p.selected_base.clone(),
                    baseline: p.merge_base.clone(),
                    proposal: p.head.clone(),
                },
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn engine() -> Value {
        json!({"package":"flowdiff","version":env!("CARGO_PKG_VERSION"),"hld_generator":crate::design::HLD_GENERATOR_VERSION,"lld_generator":crate::lld::LLD_GENERATOR_VERSION,"lld_parser":crate::lld::LLD_IR_VERSION,"public_projection":crate::public_report::PUBLIC_SCHEMA})
    }
    fn signer() -> AppReceiptSigner {
        AppReceiptSigner::from_pkcs8(
            Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap()
    }
    #[test]
    fn notification_acknowledgement_is_atomic_and_lease_bound() {
        let f = Fixture::new();
        f.enable();
        let n = WebhookNotification {
            delivery_id: "a".repeat(36),
            installation_id: 2,
            repository_id: Some(3),
            pr_number: Some(7),
            check_id: None,
            event: "pull_request".into(),
            action: "synchronize".into(),
            lease_id: "lease".into(),
        };
        {
            let s = f.queue.storage.lock().unwrap();
            s.connection.execute("INSERT INTO app_webhook_inbox(app_id,delivery_id,event,body_digest,installation_id,repository_id,pr_number,action,received_at_s,state,attempt,lease_id,lease_expires_at_s) VALUES(1,?1,'pull_request',?2,2,3,7,'synchronize',1,'leased',1,'lease',?3)",params![n.delivery_id,"a".repeat(64),now().unwrap()+60]).unwrap();
        }
        assert!(
            f.queue
                .reconcile_notification(&n, &f.provider)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            f.queue.reconcile_notification(&n, &f.provider),
            Err(SetupError::Conflict)
        );
        let s = f.queue.storage.lock().unwrap();
        assert_eq!(
            s.connection
                .query_row("SELECT state FROM app_webhook_inbox", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "processed"
        );
        assert_eq!(
            s.connection
                .query_row("SELECT count(*) FROM app_analysis_jobs", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn capacity_failure_does_not_acknowledge_notification_or_admit_work() {
        let f = Fixture::new();
        f.enable();
        f.sql("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100) INSERT INTO app_analysis_jobs(id,app_id,installation_id,repository_id,pr_number,consent_digest,input,state,created_at,available_at) SELECT printf('%064x',x),9,9,9,x,printf('%064x',0),'{}','pending',1,1 FROM n");
        assert_eq!(f.schedule(), Err(SetupError::Capacity));
        assert!(f.queue.claim().unwrap().is_none());
    }
    #[test]
    fn failed_worker_and_cross_app_leases_are_fenced() {
        let f = Fixture::new();
        f.enable();
        f.schedule().unwrap();
        let lease = f.queue.claim().unwrap().unwrap();
        let other = AppExecutionQueue::new(
            Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
            5,
        )
        .unwrap();
        assert_eq!(other.fail(&lease, false), Err(SetupError::Denied));
        let bundle = f.bundle(&lease.input);
        assert!(matches!(
            other.stage(
                &lease,
                &bundle,
                &engine(),
                "https://reports.example.test",
                &signer(),
                &f.provider
            ),
            Err(SetupError::Denied)
        ));
        f.queue.fail(&lease, true).unwrap();
        assert!(f.queue.claim().unwrap().is_none());
        assert_eq!(f.queue.fail(&lease, false), Err(SetupError::Conflict));
    }
    #[test]
    fn disabled_grants_and_revocation_never_schedule() {
        let f = Fixture::new();
        assert_eq!(f.schedule(), Err(SetupError::Denied));
        assert!(f.queue.claim().unwrap().is_none());
        f.enable();
        f.schedule().unwrap();
        f.sql("INSERT INTO app_repository_blocks VALUES(1,2,3,'removed',1)");
        assert_eq!(f.schedule(), Err(SetupError::Denied));
        assert!(f.queue.claim().unwrap().is_none());
        f.sql("DELETE FROM app_repository_blocks; UPDATE app_execution_grants SET policy=json_set(policy,'$.include',json('[\".\"]'))");
        assert!(f.schedule().is_err());
    }
    #[test]
    fn dedup_supersession_drafts_and_stale_leases() {
        let f = Fixture::new();
        f.enable();
        let first = f.schedule().unwrap();
        assert_eq!(f.schedule().unwrap(), first);
        let lease = f.queue.claim().unwrap().unwrap();
        assert!(f.queue.claim().unwrap().is_none());
        f.provider.0.borrow_mut().head = "d".repeat(40);
        assert_ne!(f.schedule().unwrap(), first);
        assert_eq!(
            lease_fence(
                &f.queue.storage.lock().unwrap().connection,
                &lease,
                now().unwrap()
            ),
            Err(SetupError::Conflict)
        );
        f.provider.0.borrow_mut().draft = true;
        assert_eq!(f.schedule().unwrap(), None);
        assert!(f.queue.claim().unwrap().is_none());
    }
    #[test]
    fn ready_after_draft_resumes_exact_job_and_webhook_removal_cancels_lease() {
        let f = Fixture::new();
        f.enable();
        let id = f.schedule().unwrap();
        let lease = f.queue.claim().unwrap().unwrap();
        f.provider.0.borrow_mut().draft = true;
        f.schedule().unwrap();
        f.provider.0.borrow_mut().draft = false;
        assert_eq!(f.schedule().unwrap(), id);
        let resumed = f.queue.claim().unwrap().unwrap();
        assert_ne!(lease.lease, resumed.lease);
        let inbox = AppWebhookInbox::new(
            Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
            1,
            vec![9; 32],
        )
        .unwrap();
        let body =
            serde_json::to_vec(&json!({"action":"deleted","installation":{"id":2,"app_id":1}}))
                .unwrap();
        let sig = format!(
            "sha256={}",
            crate::provider::hmac_sha256(&[9; 32], &body)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        inbox
            .accept(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "installation",
                &sig,
                &body,
            )
            .unwrap();
        assert_eq!(
            f.queue
                .storage
                .lock()
                .unwrap()
                .connection
                .query_row("SELECT state FROM app_analysis_jobs", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "cancelled"
        );
        assert!(f.queue.fail(&resumed, true).is_err());
        assert_eq!(f.schedule(), Err(SetupError::Denied));
    }
    #[test]
    fn restart_and_retry_limit_are_durable() {
        let f = Fixture::new();
        f.enable();
        f.schedule().unwrap();
        let lease = f.queue.claim().unwrap().unwrap();
        let reopened = AppExecutionQueue::new(
            Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
            1,
        )
        .unwrap();
        assert!(reopened.claim().unwrap().is_none());
        f.sql("UPDATE app_analysis_jobs SET lease_until=1");
        assert!(reopened.claim_at(100).unwrap().is_none());
        let second = reopened.claim_at(161).unwrap().unwrap();
        assert_ne!(lease.lease, second.lease);
        f.sql("UPDATE app_analysis_jobs SET lease_until=1,attempts=3");
        assert!(reopened.claim_at(200).unwrap().is_none());
        assert!(reopened.claim_at(500).unwrap().is_none());
    }
    struct Writer {
        posts: Cell<usize>,
        visible: Cell<bool>,
        unknown: bool,
        payload: RefCell<Option<Value>>,
    }
    impl AppCheckDelivery for Writer {
        fn find(&self, _: i64, _: &str, payload: &Value) -> Result<Option<i64>> {
            if self.visible.get() {
                assert_eq!(self.payload.borrow().as_ref(), Some(payload));
                Ok(Some(91))
            } else {
                Ok(None)
            }
        }
        fn create(&self, _: i64, payload: &Value) -> Result<i64> {
            self.posts.set(self.posts.get() + 1);
            self.payload.replace(Some(payload.clone()));
            if self.unknown {
                Err(SetupError::Unavailable)
            } else {
                self.visible.set(true);
                Ok(91)
            }
        }
    }
    #[test]
    fn signed_intent_readback_unknown_write_and_restart_recovery() {
        let f = Fixture::new();
        f.enable();
        let id = f.schedule().unwrap().unwrap();
        let lease = f.queue.claim().unwrap().unwrap();
        let bundle = f.bundle(&lease.input);
        let key = signer();
        let signed = f
            .queue
            .stage(
                &lease,
                &bundle,
                &engine(),
                "https://reports.example.test",
                &key,
                &f.provider,
            )
            .unwrap();
        assert!(signed.focus_url().contains("mode=focus&design=hld"));
        assert!(signed.verify(key.public_key()).is_ok());
        let other = signer();
        assert!(signed.verify(other.public_key()).is_err());
        let mut changed = signed.clone();
        changed.receipt.job.repository_id = 88;
        assert!(changed.verify(key.public_key()).is_err());
        assert!(
            f.queue
                .stage(
                    &lease,
                    &bundle,
                    &engine(),
                    "https://reports.example.test",
                    &key,
                    &f.provider
                )
                .is_err()
        );
        let writer = Writer {
            posts: Cell::new(0),
            visible: Cell::new(false),
            unknown: true,
            payload: RefCell::new(None),
        };
        assert!(
            f.queue
                .deliver(&id, key.public_key(), &f.provider, &writer)
                .is_err()
        );
        assert_eq!(writer.posts.get(), 0);
        let mut bad = bundle.clone();
        bad.files.values_mut().next().unwrap().push(b' ');
        assert!(
            f.queue
                .confirm_readback(&id, key.public_key(), &bad, &f.provider)
                .is_err()
        );
        f.queue
            .confirm_readback(&id, key.public_key(), &bundle, &f.provider)
            .unwrap();
        assert_eq!(
            f.queue.deliver(&id, key.public_key(), &f.provider, &writer),
            Err(SetupError::Unavailable)
        );
        let reopened = AppExecutionQueue::new(
            Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
            1,
        )
        .unwrap();
        assert_eq!(
            reopened.deliver(&id, key.public_key(), &f.provider, &writer),
            Err(SetupError::Conflict)
        );
        assert_eq!(writer.posts.get(), 1);
        writer.visible.set(true);
        assert_eq!(
            reopened.deliver(&id, key.public_key(), &f.provider, &writer),
            Ok(91)
        );
        assert_eq!(writer.posts.get(), 1);
        assert_eq!(
            writer.payload.borrow().as_ref().unwrap()["conclusion"],
            "neutral"
        );
        f.sql("UPDATE app_execution_grants SET enabled=0");
        assert!(reopened.intent(&id, key.public_key()).is_err());
    }
    #[test]
    fn changed_head_cannot_publish_signed_bytes() {
        let f = Fixture::new();
        f.enable();
        f.schedule().unwrap();
        let lease = f.queue.claim().unwrap().unwrap();
        let bundle = f.bundle(&lease.input);
        f.provider.0.borrow_mut().selected_base = "d".repeat(40);
        assert!(
            f.queue
                .stage(
                    &lease,
                    &bundle,
                    &engine(),
                    "https://reports.example.test",
                    &signer(),
                    &f.provider
                )
                .is_err()
        );
    }
}
