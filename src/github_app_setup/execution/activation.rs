//! Explicit operator capability, deliberately not mounted on the browser router.
//! Public keys and runtime pins come from protected operator configuration, never
//! consent/request JSON. Signing a host qualification is an independent operational
//! act: this module verifies its authority/bindings, not that Linux was qualified.
use super::*;

const APPROVAL_LIFETIME: i64 = 600;
const QUALIFICATION_LIFETIME: i64 = 30 * 86400;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivationRuntimePins {
    pub authentication_origin: String,
    pub report_origin: String,
    pub engine_digest: String,
    pub engine_metadata_digest: String,
    pub launcher_digest: String,
    pub interpreter_digest: String,
    pub host_identity: String,
    pub host_target: String,
}
impl ActivationRuntimePins {
    fn validate(&self) -> Result<()> {
        SetupConfig::new(1, "activation".into(), self.authentication_origin.clone())?;
        SetupConfig::new(1, "activation".into(), self.report_origin.clone())?;
        if self
            .authentication_origin
            .eq_ignore_ascii_case(&self.report_origin)
            || ![
                &self.engine_digest,
                &self.engine_metadata_digest,
                &self.launcher_digest,
                &self.interpreter_digest,
            ]
            .iter()
            .all(|s| valid_secret(s))
            || !valid_secret(&self.host_identity)
            || !matches!(
                self.host_target.as_str(),
                "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
            )
        {
            return Err(SetupError::Invalid);
        }
        Ok(())
    }
}

/// Independently verified release provenance, signed by a configured engine
/// verification authority. `provenance_digest` identifies the retained evidence,
/// not engine metadata and not a browser assertion of authenticity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivationEngineEvidence {
    pub schema: String,
    pub engine_digest: String,
    pub engine_metadata_digest: String,
    pub target: String,
    pub provenance_digest: String,
    pub verified_at: i64,
    pub expires_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivationHostEvidence {
    pub schema: String,
    pub runtime: ActivationRuntimePins,
    pub engine_evidence_digest: String,
    pub sandbox_profile: String,
    pub qualification_evidence_digest: String,
    pub qualified_at: i64,
    pub expires_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedActivationDocument<T> {
    pub document: T,
    /// Ed25519 over canonical JSON document bytes, base64url without padding.
    pub signature: String,
}
impl<T: Serialize> SignedActivationDocument<T> {
    fn verify(&self, key: &[u8]) -> Result<()> {
        let bytes = canonical_json(&self.document).map_err(|_| SetupError::Invalid)?;
        if bytes.len() > 65536 || self.signature.len() > 128 {
            return Err(SetupError::Invalid);
        }
        let signature = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|_| SetupError::Denied)?;
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&bytes, &signature)
            .map_err(|_| SetupError::Denied)
    }
    pub fn digest(&self) -> Result<String> {
        Ok(sha256(
            &canonical_json(self).map_err(|_| SetupError::Invalid)?,
        ))
    }
}

/// Three independently provisioned authorities; the browser has none of these
/// private keys. Do not load these roots from an approval/evidence document.
pub struct ActivationAuthorities {
    pub operator: [u8; 32],
    pub engine_verifier: [u8; 32],
    pub host_qualifier: [u8; 32],
}

/// Trusted provider output only, never a deserialized browser/file snapshot.
/// Implementations must freshly verify numeric App/installation/repository and
/// actor admin access, public nonarchived repository, unsuspended installation,
/// Contents read, Pull requests read and Checks write. Network implementations
/// must reserve durable nonrefundable tickets and use bounded broker passes.
pub trait AppActivationProvider {
    fn current_activation_access(
        &self,
        app: i64,
        installation: i64,
        repository: i64,
        actor: i64,
    ) -> Result<CurrentActivationAccess>;
}
#[derive(Clone, Debug)]
pub struct CurrentActivationAccess {
    pub app_id: i64,
    pub installation_id: i64,
    pub repository_id: i64,
    pub actor_id: i64,
    pub owner: String,
    pub name: String,
    pub public: bool,
    pub archived: bool,
    pub suspended: bool,
    pub actor_admin: bool,
    pub contents: String,
    pub pull_requests: String,
    pub checks: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivationDocument {
    pub schema: String,
    pub app_id: i64,
    pub installation_id: i64,
    pub repository_id: i64,
    pub actor_id: i64,
    pub scope_revision: i64,
    pub scope_digest: String,
    pub context_digest: String,
    pub consent_revision: i64,
    pub consent_digest: String,
    pub expected_activation_revision: i64,
    pub runtime: ActivationRuntimePins,
    pub engine_evidence_digest: String,
    pub host_evidence_digest: String,
    pub reviewed_at: i64,
    pub expires_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivationCommit {
    pub revision: i64,
    pub approval_digest: String,
    pub already_committed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ActivationSelection {
    pub installation_id: i64,
    pub repository_id: i64,
    pub expected_consent_revision: i64,
    pub expected_activation_revision: i64,
}

/// Local privileged API. Construct only in a dedicated operator process, not the
/// public host/router. Calling `preview` cannot write a grant or consume approval.
pub struct AppActivationOperator {
    queue: AppExecutionQueue,
    runtime: ActivationRuntimePins,
    authorities: ActivationAuthorities,
}
impl AppActivationOperator {
    pub fn new(
        storage: Storage,
        app_id: i64,
        runtime: ActivationRuntimePins,
        authorities: ActivationAuthorities,
    ) -> Result<Self> {
        runtime.validate()?;
        if authorities.operator == authorities.engine_verifier
            || authorities.operator == authorities.host_qualifier
            || authorities.engine_verifier == authorities.host_qualifier
            || [
                &authorities.operator,
                &authorities.engine_verifier,
                &authorities.host_qualifier,
            ]
            .iter()
            .any(|key| key.iter().all(|byte| *byte == 0))
        {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            queue: AppExecutionQueue::new(storage, app_id)?,
            runtime,
            authorities,
        })
    }

    /// Output is unsigned. An operator must review the complete exact document
    /// and sign its canonical JSON outside the browser before calling commit.
    pub fn preview(
        &self,
        selection: ActivationSelection,
        engine: &SignedActivationDocument<ActivationEngineEvidence>,
        host: &SignedActivationDocument<ActivationHostEvidence>,
        provider: &dyn AppActivationProvider,
    ) -> Result<ActivationDocument> {
        let at = now()?;
        self.validate_evidence(engine, host, at)?;
        let storage = self
            .queue
            .storage
            .lock()
            .map_err(|_| SetupError::Unavailable)?;
        let tx = storage.connection.unchecked_transaction().map_err(db)?;
        let (mut document, _) =
            self.current_document(&tx, selection, engine, host, provider, at)?;
        document.reviewed_at = at;
        document.expires_at = (at + APPROVAL_LIFETIME)
            .min(engine.document.expires_at)
            .min(host.document.expires_at);
        // Read transaction is deliberately rolled back; no durable preview rows.
        Ok(document)
    }

    pub fn commit(
        &self,
        approval: &SignedActivationDocument<ActivationDocument>,
        engine: &SignedActivationDocument<ActivationEngineEvidence>,
        host: &SignedActivationDocument<ActivationHostEvidence>,
        provider: &dyn AppActivationProvider,
    ) -> Result<ActivationCommit> {
        approval.verify(&self.authorities.operator)?;
        let at = now()?;
        self.validate_evidence(engine, host, at)?;
        let requested = &approval.document;
        if requested.schema != "csb-app-operator-activation-v1"
            || requested.app_id != self.queue.app_id
            || requested.runtime != self.runtime
            || requested.reviewed_at > at
            || requested.reviewed_at <= 0
            || requested.expires_at <= at
            || requested.expires_at <= requested.reviewed_at
            || requested.expires_at - requested.reviewed_at > APPROVAL_LIFETIME
        {
            return Err(SetupError::Denied);
        }
        let approval_digest = approval.digest()?;
        let mut storage = self
            .queue
            .storage
            .lock()
            .map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let consumed: Option<(i64, String)> = tx
            .query_row(
                "SELECT revision,document FROM app_execution_activations WHERE approval_digest=?1",
                [&approval_digest],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db)?;
        let expected_revision = if let Some((revision, raw)) = &consumed {
            if raw != &serde_json::to_string(requested).map_err(|_| SetupError::Invalid)?
                || Some(*revision) != requested.expected_activation_revision.checked_add(1)
            {
                return Err(SetupError::Conflict);
            }
            *revision
        } else {
            requested.expected_activation_revision
        };
        let (mut current, policy) = self.current_document(
            &tx,
            ActivationSelection {
                installation_id: requested.installation_id,
                repository_id: requested.repository_id,
                expected_consent_revision: requested.consent_revision,
                expected_activation_revision: expected_revision,
            },
            engine,
            host,
            provider,
            at,
        )?;
        // Only timestamps/revision differ for an exact, still-current retry.
        current.reviewed_at = requested.reviewed_at;
        current.expires_at = requested.expires_at;
        current.expected_activation_revision = requested.expected_activation_revision;
        if &current != requested
            || now()? >= requested.expires_at
            || now()? >= engine.document.expires_at
            || now()? >= host.document.expires_at
        {
            return Err(SetupError::Conflict);
        }
        if let Some((revision, _)) = consumed {
            let enabled: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM app_execution_grants WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND enabled=1 AND activation_revision=?4 AND consent_digest=?5)",
                params![self.queue.app_id, requested.installation_id, requested.repository_id, revision, requested.consent_digest], |r| r.get(0),
            ).map_err(db)?;
            if !enabled {
                return Err(SetupError::Conflict);
            }
            tx.commit().map_err(db)?;
            return Ok(ActivationCommit {
                revision,
                approval_digest,
                already_committed: true,
            });
        }
        let revision = requested
            .expected_activation_revision
            .checked_add(1)
            .ok_or(SetupError::Invalid)?;
        tx.execute(
            "INSERT INTO app_execution_activations(app_id,installation_id,repository_id,revision,consent_revision,consent_digest,approval_digest,document,approval,engine_evidence,host_evidence,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![self.queue.app_id, requested.installation_id, requested.repository_id, revision,
                requested.consent_revision, requested.consent_digest, approval_digest,
                serde_json::to_string(requested).map_err(|_| SetupError::Invalid)?,
                serde_json::to_string(approval).map_err(|_| SetupError::Invalid)?,
                serde_json::to_string(engine).map_err(|_| SetupError::Invalid)?,
                serde_json::to_string(host).map_err(|_| SetupError::Invalid)?, at],
        ).map_err(db)?;
        tx.execute(
            "INSERT INTO app_execution_grants(app_id,installation_id,repository_id,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,enabled,activation_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9) ON CONFLICT(app_id,installation_id,repository_id) DO UPDATE SET scope_revision=excluded.scope_revision,consent_digest=excluded.consent_digest,engine_digest=excluded.engine_digest,engine_metadata_digest=excluded.engine_metadata_digest,policy=excluded.policy,enabled=1,activation_revision=excluded.activation_revision",
            params![self.queue.app_id, requested.installation_id, requested.repository_id,
                requested.scope_revision, requested.consent_digest, self.runtime.engine_digest,
                self.runtime.engine_metadata_digest, serde_json::to_string(&policy).map_err(|_| SetupError::Invalid)?, revision],
        ).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(ActivationCommit {
            revision,
            approval_digest,
            already_committed: false,
        })
    }

    fn validate_evidence(
        &self,
        engine: &SignedActivationDocument<ActivationEngineEvidence>,
        host: &SignedActivationDocument<ActivationHostEvidence>,
        at: i64,
    ) -> Result<()> {
        verify_activation_qualification(&self.runtime, &self.authorities, engine, host, at)
    }

    fn current_document(
        &self,
        c: &rusqlite::Connection,
        selection: ActivationSelection,
        engine: &SignedActivationDocument<ActivationEngineEvidence>,
        host: &SignedActivationDocument<ActivationHostEvidence>,
        provider: &dyn AppActivationProvider,
        at: i64,
    ) -> Result<(ActivationDocument, PublicPolicy)> {
        let ActivationSelection {
            installation_id: installation,
            repository_id: repository,
            expected_consent_revision: consent_revision,
            expected_activation_revision: activation_revision,
        } = selection;
        if installation <= 0 || repository <= 0 || consent_revision <= 0 || activation_revision < 0
        {
            return Err(SetupError::Invalid);
        }
        let app = self.queue.app_id;
        super::super::scope::unblocked(c, app, installation, repository)?;
        let latest_activation: i64 = c.query_row(
            "SELECT coalesce(max(revision),0) FROM app_execution_activations WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",
            params![app, installation, repository], |r| r.get(0),
        ).map_err(db)?;
        if latest_activation != activation_revision {
            return Err(SetupError::Conflict);
        }
        let row: Option<(i64, i64, i64, String, String, bool)> = c.query_row(
            "SELECT revision,actor_id,created_at,document,digest,revoked FROM app_publication_consents WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 ORDER BY revision DESC LIMIT 1",
            params![app, installation, repository], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional().map_err(db)?;
        let (revision, actor, created, raw, digest, revoked) = row.ok_or(SetupError::Denied)?;
        if revision != consent_revision || revoked || actor <= 0 || created > at {
            return Err(SetupError::Conflict);
        }
        let consent: Value = serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
        if sha256(&canonical_json(&consent).map_err(|_| SetupError::Invalid)?) != digest
            || consent["schema"] != "csb-app-publication-consent-v1"
            || consent["app_id"].as_i64() != Some(app)
            || consent["installation_id"].as_i64() != Some(installation)
            || consent["repository_id"].as_i64() != Some(repository)
            || consent["actor_id"].as_i64() != Some(actor)
            || consent["expected_revision"].as_i64() != Some(revision - 1)
            || consent["activation"] != "off"
            || consent["expires_at"]
                .as_i64()
                .is_none_or(|expiry| expiry <= created)
            || consent["operator"]["report_origin"].as_str() != Some(&self.runtime.report_origin)
            || consent["operator"]["engine_digest"].as_str() != Some(&self.runtime.engine_digest)
            || consent["operator"]["engine_metadata_digest"].as_str()
                != Some(&self.runtime.engine_metadata_digest)
        {
            return Err(SetupError::Denied);
        }
        let saved = super::super::scope::latest(c, app, installation, repository)?
            .ok_or(SetupError::Denied)?;
        if saved.actor_id != actor
            || consent["scope_revision"].as_i64() != Some(saved.revision)
            || consent["scope_commit"].as_str() != Some(&saved.document.commit)
            || saved.document.installation_id != installation
            || saved.document.repository_id != repository
        {
            return Err(SetupError::Conflict);
        }
        let (context_actor, context): (i64, String) = c.query_row(
            "SELECT actor_id,context FROM app_architecture_acceptances WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND scope_revision=?4",
            params![app, installation, repository, saved.revision], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(db)?.ok_or(SetupError::Denied)?;
        let context_digest = sha256(context.as_bytes());
        let accepted: Value = serde_json::from_str(&context).map_err(|_| SetupError::Invalid)?;
        if context_actor != actor
            || consent["context_digest"].as_str() != Some(&context_digest)
            || accepted["usable"] != true
            || accepted["confirmation"]["actor"].as_str()
                != Some(format!("github-user-{actor}").as_str())
        {
            return Err(SetupError::Denied);
        }
        let policy: PublicPolicy =
            serde_json::from_value(consent["policy"].clone()).map_err(|_| SetupError::Invalid)?;
        policy.validate().map_err(|_| SetupError::Denied)?;
        if policy.repository.id != repository as u64
            || policy.include != saved.document.include
            || policy.exclude != saved.document.exclude
            || policy.baseline_mode != "merge_base"
        {
            return Err(SetupError::Denied);
        }
        let access = provider.current_activation_access(app, installation, repository, actor)?;
        if access.app_id != app
            || access.installation_id != installation
            || access.repository_id != repository
            || access.actor_id != actor
            || access.owner != policy.repository.owner
            || access.name != policy.repository.name
            || !access.public
            || access.archived
            || access.suspended
            || !access.actor_admin
            || access.contents != "read"
            || access.pull_requests != "read"
            || access.checks != "write"
        {
            return Err(SetupError::Denied);
        }
        Ok((
            ActivationDocument {
                schema: "csb-app-operator-activation-v1".into(),
                app_id: app,
                installation_id: installation,
                repository_id: repository,
                actor_id: actor,
                scope_revision: saved.revision,
                scope_digest: sha256(
                    &canonical_json(&saved.document).map_err(|_| SetupError::Invalid)?,
                ),
                context_digest,
                consent_revision: revision,
                consent_digest: digest,
                expected_activation_revision: activation_revision,
                runtime: self.runtime.clone(),
                engine_evidence_digest: engine.digest()?,
                host_evidence_digest: host.digest()?,
                reviewed_at: at,
                expires_at: at + APPROVAL_LIFETIME,
            },
            policy,
        ))
    }
}
fn evidence_times(issued: i64, expires: i64, at: i64) -> bool {
    issued > 0
        && issued <= at
        && expires > at
        && expires > issued
        && expires - issued <= QUALIFICATION_LIFETIME
}

/// Offline evidence authentication only: makes no provider request and never
/// creates a grant. Success is not independent qualification of the current host.
pub fn verify_activation_qualification(
    runtime: &ActivationRuntimePins,
    authorities: &ActivationAuthorities,
    engine: &SignedActivationDocument<ActivationEngineEvidence>,
    host: &SignedActivationDocument<ActivationHostEvidence>,
    at: i64,
) -> Result<()> {
    runtime.validate()?;
    if authorities.operator == authorities.engine_verifier
        || authorities.operator == authorities.host_qualifier
        || authorities.engine_verifier == authorities.host_qualifier
    {
        return Err(SetupError::Invalid);
    }
    engine.verify(&authorities.engine_verifier)?;
    host.verify(&authorities.host_qualifier)?;
    let e = &engine.document;
    let h = &host.document;
    if e.schema != "csb-app-signed-engine-verification-v1"
        || h.schema != "csb-app-host-qualification-v1"
        || e.engine_digest != runtime.engine_digest
        || e.engine_metadata_digest != runtime.engine_metadata_digest
        || e.target != runtime.host_target
        || &h.runtime != runtime
        || h.engine_evidence_digest != engine.digest()?
        || h.sandbox_profile != "csb-linux-bwrap-prlimit-v1"
        || !valid_secret(&e.provenance_digest)
        || !valid_secret(&h.qualification_evidence_digest)
        || !evidence_times(e.verified_at, e.expires_at, at)
        || !evidence_times(h.qualified_at, h.expires_at, at)
    {
        return Err(SetupError::Denied);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
