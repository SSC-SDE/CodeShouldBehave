//! Explicit source-scan permission and inactive architecture acceptance.
use super::*;
use crate::{
    ComponentDecision, ComponentProposal, IntegrationDecision, OnboardingInventory, OnboardingState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanInput {
    pub installation_id: i64,
    pub repository_id: i64,
    pub scope_revision: i64,
    pub allow_selected_source_scan: bool,
}
#[derive(Serialize)]
pub struct ArchitecturePreview {
    pub preview_id: String,
    pub scope_revision: i64,
    pub expires_at_s: i64,
    pub result: ScanResult,
    pub activation: &'static str,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureDecision {
    pub id: String,
    pub name: String,
    pub roots: Vec<String>,
    pub decision: ComponentDecision,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureConfirmation {
    pub installation_id: i64,
    pub repository_id: i64,
    pub scope_revision: i64,
    pub preview_id: String,
    pub components: Vec<ArchitectureDecision>,
    pub accept_known_gaps: bool,
    pub defer_integrations: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPreview {
    repository_owner: String,
    repository_name: String,
    document: ScopeDocument,
    inventory: OnboardingInventory,
    result: ScanResult,
}
fn prepare(
    stored: &StoredPreview,
    actor: i64,
) -> Result<(
    crate::OnboardingSession,
    crate::PrincipalId,
    Vec<crate::Grant>,
)> {
    let (mut s, p, g) = scan_worker::session(
        &stored.document,
        actor,
        &stored.repository_owner,
        &stored.repository_name,
    )?;
    s.record_inventory(0, stored.inventory.clone(), &p, &g)
        .map_err(|_| SetupError::Invalid)?;
    if stored.result.version != scan_worker::SCAN_VERSION
        || stored.result.input_fingerprint != stored.document.inventory_fingerprint
    {
        return Err(SetupError::Invalid);
    }
    for e in stored
        .result
        .components
        .iter()
        .flat_map(|c| &c.evidence)
        .chain(stored.result.providers.iter().flat_map(|c| &c.evidence))
    {
        if e.repository_index != 0
            || !stored
                .inventory
                .items
                .iter()
                .any(|f| f.path == e.path && f.blob == e.blob && f.status == "indexed")
        {
            return Err(SetupError::Invalid);
        }
    }
    s.set_proposal(
        s.revision(),
        stored.result.components.clone(),
        stored.result.providers.clone(),
        stored.result.gaps.clone(),
        &p,
        &g,
    )
    .map_err(|_| SetupError::Invalid)?;
    s.transition(s.revision(), OnboardingState::AwaitingConfirmation, &p, &g)
        .map_err(|_| SetupError::Invalid)?;
    Ok((s, p, g))
}
impl<P: GitHubSetupProvider> AppSetupHost<P> {
    pub fn preview_architecture(
        &self,
        token: &str,
        input: ScanInput,
    ) -> Result<ArchitecturePreview> {
        if !input.allow_selected_source_scan || input.scope_revision <= 0 {
            return Err(SetupError::Invalid);
        }
        let worker = self.scan_worker.as_ref().ok_or(SetupError::Unavailable)?;
        let _admission = self
            .scan_admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        let saved = self
            .saved_scope(token, input.installation_id, input.repository_id)?
            .ok_or(SetupError::Conflict)?;
        if saved.revision != input.scope_revision {
            return Err(SetupError::Conflict);
        }
        let inventory = self.scope_inventory(token, input.installation_id, input.repository_id)?;
        if digest(&serde_json::to_string(&inventory).map_err(|_| SetupError::Unavailable)?)
            != saved.document.inventory_fingerprint
        {
            return Err(SetupError::Conflict);
        }
        let entries = inventory
            .entries
            .iter()
            .filter(|f| saved.document.files.contains(&f.path))
            .cloned()
            .collect::<Vec<_>>();
        let to_read = entries
            .iter()
            .filter(|f| scan_worker::supported_source(&f.path))
            .cloned()
            .collect::<Vec<_>>();
        if to_read.is_empty() || to_read.len() > scan_worker::MAX_SOURCE_FILES {
            return Err(SetupError::Capacity);
        }
        let choice = self.authorized_choice(token, input.installation_id, input.repository_id)?;
        let session = self.session(token, now()?)?;
        let mut files = self
            .provider
            .scoped_sources(&session.credential, &choice, &to_read)?;
        if files.len() != to_read.len()
            || files
                .iter()
                .any(|f| !to_read.iter().any(|e| e.path == f.path && e.oid == f.oid))
        {
            return Err(SetupError::Unavailable);
        }
        files.extend(
            entries
                .iter()
                .filter(|f| !scan_worker::supported_source(&f.path))
                .map(|f| PreviewFile {
                    path: f.path.clone(),
                    oid: f.oid.clone(),
                    content: None,
                }),
        );
        self.authorized_choice(token, input.installation_id, input.repository_id)?;
        let request = ScanRequest {
            actor_id: session.credential.user_id,
            repository_owner: choice.owner,
            repository_name: choice.name,
            document: saved.document,
            files,
        };
        let (model, _, _) = scan_worker::session(
            &request.document,
            request.actor_id,
            &request.repository_owner,
            &request.repository_name,
        )?;
        let inventory = scan_worker::inventory(&request.document, &request.files, &model)?;
        let result = worker.preview(&request)?;
        let stored = StoredPreview {
            repository_owner: request.repository_owner,
            repository_name: request.repository_name,
            document: request.document,
            inventory,
            result: result.clone(),
        };
        prepare(&stored, request.actor_id)?;
        let encoded = serde_json::to_string(&stored).map_err(|_| SetupError::Unavailable)?;
        if encoded.len() > 2 * 1024 * 1024 {
            return Err(SetupError::Capacity);
        }
        // Revalidate current access and exact commit after a potentially slow worker.
        let fresh = self.scope_inventory(token, input.installation_id, input.repository_id)?;
        if digest(&serde_json::to_string(&fresh).map_err(|_| SetupError::Unavailable)?)
            != stored.document.inventory_fingerprint
        {
            return Err(SetupError::Conflict);
        }
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        let actor = scope::fence(&sessions, token)?.credential.user_id;
        let mut dbh = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = dbh
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        if scope::latest(
            &tx,
            self.config.app_id,
            input.installation_id,
            input.repository_id,
        )?
        .map(|s| s.revision)
            != Some(input.scope_revision)
        {
            return Err(SetupError::Conflict);
        }
        let at = now()?;
        tx.execute("DELETE FROM app_architecture_previews WHERE expires_at_s<=?1 OR (app_id=?2 AND installation_id=?3 AND repository_id=?4 AND actor_id=?5)",params![at,self.config.app_id,input.installation_id,input.repository_id,actor]).map_err(db)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM app_architecture_previews", [], |r| {
                r.get(0)
            })
            .map_err(db)?;
        if count >= 64 {
            return Err(SetupError::Capacity);
        }
        let id = secret()?;
        tx.execute(
            "INSERT INTO app_architecture_previews VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                self.config.app_id,
                input.installation_id,
                input.repository_id,
                input.scope_revision,
                actor,
                at + 900,
                encoded
            ],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(ArchitecturePreview {
            preview_id: id,
            scope_revision: input.scope_revision,
            expires_at_s: at + 900,
            result,
            activation: "off",
        })
    }
    pub fn confirm_architecture(
        &self,
        token: &str,
        input: ArchitectureConfirmation,
    ) -> Result<Value> {
        if !input.accept_known_gaps || !input.defer_integrations || !valid_secret(&input.preview_id)
        {
            return Err(SetupError::Invalid);
        }
        let actor = self.session(token, now()?)?.credential.user_id;
        let stored: StoredPreview = {
            let dbh = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            let doc:Option<String>=dbh.connection.query_row("SELECT document FROM app_architecture_previews WHERE id=?1 AND app_id=?2 AND installation_id=?3 AND repository_id=?4 AND scope_revision=?5 AND actor_id=?6 AND expires_at_s>?7",params![input.preview_id,self.config.app_id,input.installation_id,input.repository_id,input.scope_revision,actor,now()?],|r|r.get(0)).optional().map_err(db)?;
            serde_json::from_str(&doc.ok_or(SetupError::Conflict)?)
                .map_err(|_| SetupError::Unavailable)?
        };
        let fresh = self.scope_inventory(token, input.installation_id, input.repository_id)?;
        if digest(&serde_json::to_string(&fresh).map_err(|_| SetupError::Unavailable)?)
            != stored.document.inventory_fingerprint
        {
            return Err(SetupError::Conflict);
        }
        let choice = self.authorized_choice(token, input.installation_id, input.repository_id)?;
        if choice.owner != stored.repository_owner || choice.name != stored.repository_name {
            return Err(SetupError::Conflict);
        }
        let (mut session, principal, grants) = prepare(&stored, actor)?;
        if input.components.len() != session.proposals().len() || input.components.is_empty() {
            return Err(SetupError::Invalid);
        }
        let mut seen = std::collections::BTreeSet::new();
        for decision in input.components {
            if !seen.insert(decision.id.clone()) || decision.decision == ComponentDecision::Pending
            {
                return Err(SetupError::Invalid);
            }
            let mut component: ComponentProposal = session
                .proposals()
                .iter()
                .find(|c| c.id == decision.id)
                .cloned()
                .ok_or(SetupError::Invalid)?;
            component.name = decision.name;
            component.roots = decision.roots;
            component.decision = decision.decision;
            session
                .decide_component(session.revision(), component, &principal, &grants)
                .map_err(|_| SetupError::Invalid)?;
        }
        for (index, mut mapping) in session.provider_mappings().to_vec().into_iter().enumerate() {
            mapping.decision = ComponentDecision::Deferred;
            session
                .decide_provider(session.revision(), index, mapping, &principal, &grants)
                .map_err(|_| SetupError::Invalid)?;
        }
        for integration in session.integrations().to_vec() {
            session
                .decide_integration(
                    session.revision(),
                    &integration.id,
                    IntegrationDecision::Defer,
                    None,
                    &principal,
                    &grants,
                )
                .map_err(|_| SetupError::Invalid)?;
        }
        session
            .confirm(
                session.revision(),
                &session.proposal_fingerprint(),
                true,
                now()? * 1000,
                &principal,
                &grants,
            )
            .map_err(|_| SetupError::Invalid)?;
        let context =
            crate::onboarding_service::accepted_context_view(&session, &principal, &grants)
                .map_err(|_| SetupError::Invalid)?;
        if context["usable"] != true {
            return Err(SetupError::Invalid);
        }
        let encoded = serde_json::to_string(&context).map_err(|_| SetupError::Unavailable)?;
        if encoded.len() > 2 * 1024 * 1024 {
            return Err(SetupError::Capacity);
        }
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        scope::fence(&sessions, token)?;
        let mut dbh = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = dbh
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        if scope::latest(
            &tx,
            self.config.app_id,
            input.installation_id,
            input.repository_id,
        )?
        .map(|s| s.revision)
            != Some(input.scope_revision)
        {
            return Err(SetupError::Conflict);
        }
        let at = now()?;
        let used=tx.execute("DELETE FROM app_architecture_previews WHERE id=?1 AND actor_id=?2 AND expires_at_s>?3",params![input.preview_id,actor,at]).map_err(db)?;
        if used != 1 {
            return Err(SetupError::Conflict);
        }
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_architecture_acceptances WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND scope_revision=?4)",params![self.config.app_id,input.installation_id,input.repository_id,input.scope_revision],|r|r.get(0)).map_err(db)?;
        if exists {
            return Err(SetupError::Conflict);
        }
        tx.execute("INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![self.config.app_id,input.installation_id,input.repository_id,input.scope_revision,actor,at,encoded]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(
            json!({"scope_revision":input.scope_revision,"architecture_confirmed":true,"commit":stored.document.commit,"context":context,"activation":"off","publication_consent":false}),
        )
    }
    pub fn architecture_status(
        &self,
        token: &str,
        installation: i64,
        repository: i64,
    ) -> Result<Value> {
        let saved = self.saved_scope(token, installation, repository)?;
        let Some(saved) = saved else {
            return Ok(json!({"architecture_confirmed":false,"activation":"off"}));
        };
        let sessions = self.sessions.lock().map_err(|_| SetupError::Unavailable)?;
        scope::fence(&sessions, token)?;
        let dbh = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        scope::unblocked(
            &dbh.connection,
            self.config.app_id,
            installation,
            repository,
        )?;
        let context:Option<String>=dbh.connection.query_row("SELECT context FROM app_architecture_acceptances WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND scope_revision=?4",params![self.config.app_id,installation,repository,saved.revision],|r|r.get(0)).optional().map_err(db)?;
        Ok(
            json!({"scope_revision":saved.revision,"architecture_confirmed":context.is_some(),"commit":saved.document.commit,"context":context.map(|s|serde_json::from_str::<Value>(&s)).transpose().map_err(|_|SetupError::Unavailable)?,"activation":"off","publication_consent":false}),
        )
    }
}
