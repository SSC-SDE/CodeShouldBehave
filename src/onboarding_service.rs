//! Authenticated onboarding transport. No browser input becomes a local path.
use crate::{
    Action, ComponentDecision, ComponentProposal, Grant, HttpBackend, HttpFailure, HttpOperation,
    OnboardingSession, OnboardingState, PrincipalId, RepositoryId, SecurityDomain,
    ServiceAuthenticator, SnapshotId, Storage, authorize_repository,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub trait OnboardingAccessProvider: Send + Sync + 'static {
    /// Current installation AND viewer permissions, scoped to one security domain.
    fn grants(
        &self,
        principal: &PrincipalId,
        domain: &SecurityDomain,
    ) -> Result<Vec<Grant>, String>;
}
pub trait OnboardingSource: Send + Sync + 'static {
    fn drift(
        &self,
        _context: &Value,
        _target: &SnapshotId,
        _principal: &PrincipalId,
        _grants: &[Grant],
    ) -> Result<Value, String> {
        Err("Architecture context drift adapter unavailable".into())
    }
    fn providers(
        &self,
        _session: &OnboardingSession,
        _components: &[ComponentProposal],
        _principal: &PrincipalId,
        _grants: &[Grant],
    ) -> Result<(Vec<crate::ComponentProviderMapping>, Vec<String>), String> {
        Ok((
            vec![],
            vec![
                "Static hosting adapter unavailable; provider and deployment remain unknown".into(),
            ],
        ))
    }
    fn detect(
        &self,
        _session: &OnboardingSession,
        _principal: &PrincipalId,
        _grants: &[Grant],
    ) -> Result<(Vec<ComponentProposal>, Vec<String>), String> {
        Err("Detection unavailable".into())
    }

    fn scan(
        &self,
        _session: &OnboardingSession,
        _principal: &PrincipalId,
        _grants: &[Grant],
    ) -> Result<crate::OnboardingInventory, String> {
        Err("Scan unavailable".into())
    }

    fn resolve(&self, repository: &RepositoryId, reference: &str) -> Result<SnapshotId, String>;
}
/// Operator-selected clones/mirrors. Browser requests cannot add or change roots.
pub struct LocalOnboardingSource {
    roots: BTreeMap<RepositoryId, PathBuf>,
}
impl LocalOnboardingSource {
    pub fn new(roots: BTreeMap<RepositoryId, PathBuf>) -> Self {
        Self { roots }
    }
    pub(crate) fn root(&self, repository: &RepositoryId) -> Result<&std::path::Path, String> {
        self.roots
            .get(repository)
            .map(PathBuf::as_path)
            .ok_or_else(|| "Repository unavailable".into())
    }
}
impl OnboardingSource for LocalOnboardingSource {
    fn drift(
        &self,
        context: &Value,
        target: &SnapshotId,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<Value, String> {
        crate::onboarding::drift::assess(&self.roots, context, target, principal, grants)
    }
    fn providers(
        &self,
        session: &OnboardingSession,
        components: &[ComponentProposal],
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(Vec<crate::ComponentProviderMapping>, Vec<String>), String> {
        crate::onboarding::providers::detect(&self.roots, session, components, principal, grants)
    }
    fn detect(
        &self,
        session: &OnboardingSession,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(Vec<ComponentProposal>, Vec<String>), String> {
        crate::onboarding_detection::detect(&self.roots, session, principal, grants)
    }

    fn scan(
        &self,
        session: &OnboardingSession,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<crate::OnboardingInventory, String> {
        let index = session.inventory().map_or(0, |item| item.repository_index);
        let (repository, _, _) = crate::onboarding_scan::pin(session, index)?;
        crate::onboarding_scan::scan_chunk(
            self.root(repository)?,
            session,
            session.inventory(),
            &crate::Cancellation::default(),
            principal,
            grants,
        )
    }

    fn resolve(&self, repository: &RepositoryId, reference: &str) -> Result<SnapshotId, String> {
        if reference.is_empty()
            || reference.len() > 256
            || reference.starts_with('-')
            || reference.chars().any(char::is_control)
        {
            return Err("Invalid commit reference".into());
        }
        SnapshotId::new(crate::git::resolve(self.root(repository)?, reference)?)
    }
}

pub struct OnboardingHost<A, P, R> {
    storage: Mutex<Storage>,
    authenticator: A,
    access: P,
    source: R,
    fallback: Arc<dyn HttpBackend>,
}
impl<A, P, R> OnboardingHost<A, P, R> {
    pub fn new(
        storage: Storage,
        authenticator: A,
        access: P,
        source: R,
        fallback: Arc<dyn HttpBackend>,
    ) -> Self {
        Self {
            storage: Mutex::new(storage),
            authenticator,
            access,
            source,
            fallback,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    id: String,
    reference: String,
    scope_version: String,
    scope: crate::http::ScopePayload,
    #[serde(default)]
    sample_env_names: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mutation {
    security_domain: String,
    expected_revision: u64,
    component: Option<ComponentProposal>,
    proposal_fingerprint: Option<String>,
    known_gaps_accepted: Option<bool>,
    provider_index: Option<usize>,
    provider: Option<crate::ComponentProviderMapping>,
    integration_id: Option<String>,
    decision: Option<crate::onboarding::IntegrationDecision>,
    answer: Option<String>,
    #[serde(default)]
    activate_defaults: bool,
    repository: Option<RepositoryRequest>,
    reference: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryRequest {
    security_domain: String,
    provider: String,
    owner: String,
    name: String,
}
fn now() -> Result<i64, HttpFailure> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HttpFailure::Internal)?
        .as_millis();
    i64::try_from(ms).map_err(|_| HttpFailure::Internal)
}
fn failure(error: String) -> HttpFailure {
    let error = error.to_ascii_lowercase();
    if error.contains("stale") || error.contains("conflict") || error.contains("immutable") {
        HttpFailure::Conflict
    } else if error.contains("denied")
        || error.contains("unavailable")
        || error.contains("grant")
        || error.contains("outside")
    {
        HttpFailure::DeniedOrMissing
    } else if error.contains("limit") || error.contains("bound") {
        HttpFailure::TooLarge
    } else if error.contains("invalid")
        || error.contains("requires")
        || error.contains("require")
        || error.contains("must")
        || error.contains("unknown")
        || error.contains("cannot change")
    {
        HttpFailure::Malformed
    } else {
        HttpFailure::Internal
    }
}
fn repo_json(repository: &RepositoryId) -> Value {
    json!({"security_domain":repository.security_domain().as_str(),"provider":repository.provider(),"owner":repository.owner(),"name":repository.name()})
}
pub(crate) fn accepted_context_view(
    session: &OnboardingSession,
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<Value, String> {
    let scope = session.proposed_scope(principal, grants)?;
    let mut boundary_gaps = std::collections::BTreeSet::new();
    let allows = |index: usize, path: &str| {
        if crate::onboarding_scan::sensitive_path(path) {
            return false;
        }
        let Some(scope) = &scope else {
            return false;
        };
        if index == 0 {
            scope.primary_paths.allows(path)
        } else {
            session
                .scope()
                .contexts
                .get(index - 1)
                .is_some_and(|original| {
                    scope.contexts.iter().any(|pin| {
                        pin.name == original.name
                            && pin.repository == original.repository
                            && pin.commit == original.commit
                            && pin.paths.allows(path)
                    })
                })
        }
    };
    let components = session
        .proposals()
        .iter()
        .filter(|component| {
            matches!(
                component.decision,
                ComponentDecision::Confirmed | ComponentDecision::Corrected
            )
        })
        .filter_map(|component| {
            let mut component = component.clone();
            component
                .roots
                .retain(|path| allows(component.repository_index, path));
            if component.roots.is_empty() {
                return None;
            }
            let excluded_integration_indexes = component
                .signals
                .iter()
                .filter(|signal| {
                    session.integrations().iter().any(|item| {
                        item.component_id == component.id
                            && item.kind == signal.kind
                            && item.name == signal.name
                            && item.test_only == signal.test_only
                            && item.decision != crate::onboarding::IntegrationDecision::Include
                    })
                })
                .map(|signal| signal.evidence_index)
                .collect::<std::collections::BTreeSet<_>>();
            component.stack_candidates.retain(|stack| {
                !session.integrations().iter().any(|item| {
                    item.component_id == component.id
                        && item.decision != crate::onboarding::IntegrationDecision::Include
                        && (stack == &item.name || stack.starts_with(&format!("{} ·", item.name)))
                })
            });
            let old_evidence = std::mem::take(&mut component.evidence);
            let has_outside_evidence = old_evidence.iter().any(|evidence| !allows(evidence.repository_index, &evidence.path));
            let has_outside_limit = component.contradictions.iter().any(|limit|limit.strip_suffix(" content unavailable").is_some_and(|path|!allows(component.repository_index,path)));
            if has_outside_evidence || has_outside_limit {
                // Legacy stack/limit labels have no individual source references.
                // Keep them private when root narrowing removes their provenance.
                component.stack_candidates.clear();
                component.contradictions.clear();
                boundary_gaps.insert("Some component interpretation details are omitted because their original provenance is outside accepted paths; inspect the protected onboarding session".to_string());
            }
            let mut remap = BTreeMap::new();
            for (index, evidence) in old_evidence.into_iter().enumerate() {
                if !excluded_integration_indexes.contains(&index)
                    && allows(evidence.repository_index, &evidence.path)
                {
                    remap.insert(index, component.evidence.len());
                    component.evidence.push(evidence);
                }
            }
            component.signals.retain_mut(|signal| {
                if let Some(index) = remap.get(&signal.evidence_index) {
                    signal.evidence_index = *index;
                    true
                } else {
                    false
                }
            });
            Some(component)
        })
        .collect::<Vec<_>>();
    let included_ids = components
        .iter()
        .map(|component| component.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let integrations = session
        .integrations()
        .iter()
        .filter(|item| {
            included_ids.contains(item.component_id.as_str())
                && item.decision == crate::onboarding::IntegrationDecision::Include
        })
        .filter_map(|item| {
            let mut item = item.clone();
            item.evidence
                .retain(|evidence| allows(evidence.repository_index, &evidence.path));
            if item.evidence.is_empty() {
                None
            } else {
                Some(item)
            }
        })
        .collect::<Vec<_>>();
    let providers = session
        .provider_mappings()
        .iter()
        .filter(|item| {
            included_ids.contains(item.component_id.as_str())
                && matches!(
                    item.decision,
                    ComponentDecision::Confirmed | ComponentDecision::Corrected
                )
        })
        .filter_map(|item| {
            let mut item = item.clone();
            item.evidence
                .retain(|evidence| allows(evidence.repository_index, &evidence.path));
            if !allows(item.repository_index, &item.source_root) || item.evidence.is_empty() {
                boundary_gaps.insert("Some hosting interpretations are omitted because their source root or original provenance is outside accepted paths; a new boundary and explicit acceptance are required to include them".to_string());
                None
            } else {
                Some(item)
            }
        })
        .collect::<Vec<_>>();
    let (include, exclude, contexts, fingerprint) = if let Some(scope) = &scope {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"csb-accepted-context-v1:sensitive-path-policy-v1");
        hash.update(scope.canonical_bytes());
        (scope.primary_paths.includes().collect::<Vec<_>>(), scope.primary_paths.excludes().collect::<Vec<_>>(),
        scope.contexts.iter().map(|pin| json!({"name":pin.name,"repository":repo_json(&pin.repository),"commit":pin.commit.as_str(),"repository_index":session.scope().contexts.iter().position(|original|original.name==pin.name && original.repository==pin.repository && original.commit==pin.commit).map(|index|index+1),"include":pin.paths.includes().collect::<Vec<_>>(),"exclude":pin.paths.excludes().collect::<Vec<_>>()})).collect::<Vec<_>>(),
        hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>())
    } else {
        (vec![], vec![], vec![], String::new())
    };
    let pending = session.pending_decisions();
    let mut gaps = Vec::new();
    gaps.extend(boundary_gaps);
    if !session.gaps().is_empty() {
        gaps.push(format!("{} scan limitations were explicitly accepted; inspect the protected onboarding session for source-qualified details", session.gaps().len()));
    }
    if !session.scope().contracts.is_empty() {
        gaps.push("External contracts remain excluded; source confirmation does not authorize additional retrieval".into());
    }
    if !integrations.is_empty() {
        gaps.push("Included integrations are source observations; destinations, ownership and runtime direction remain unverified".into());
    }
    if providers
        .iter()
        .any(|item| item.verification != crate::ProviderVerification::Verified)
    {
        gaps.push("Hosting labels are user interpretations of static evidence; live verification remains disabled".into());
    }
    Ok(
        json!({"version":"csb-accepted-context-v1","session_id":session.id(),"session_revision":session.revision(),"commit":session.commit().as_str(),
        "repository":repo_json(&session.scope().primary_repository),"input_fingerprint":session.input_fingerprint(),"proposal_fingerprint":session.proposal_fingerprint(),
        "accepted_scope_fingerprint":fingerprint,"scope_version":session.scope().version.as_str(),"export_allowed":session.scope().export_allowed,"include":include,"exclude":exclude,"contexts":contexts,"contracts":[],"components":components,"integrations":integrations,"providers":providers,
        "confirmation":session.confirmation(),"usable":scope.is_some() && pending == (0,0,0),"sensitive_paths_excluded":true,"gaps":gaps}),
    )
}

/// Existing immutable defaults must satisfy today's scoped-metadata invariants.
/// Invalid older records are unavailable; never rewrite their accepted history.
fn accepted_context_scopes(value: &Value) -> Result<BTreeMap<usize, crate::PathScope>, String> {
    let denied = || {
        "Onboarding defaults unavailable; new boundary and explicit acceptance required".to_string()
    };
    let parse = |value: &Value| -> Result<crate::PathScope, String> {
        let list = |name: &str| -> Result<Vec<String>, String> {
            value[name]
                .as_array()
                .ok_or_else(denied)?
                .iter()
                .map(|path| path.as_str().map(str::to_owned).ok_or_else(denied))
                .collect()
        };
        crate::PathScope::new(list("include")?, list("exclude")?).map_err(|_| denied())
    };
    let mut scopes = BTreeMap::from([(0, parse(value)?)]);
    for related in value["contexts"].as_array().ok_or_else(denied)? {
        let index = related["repository_index"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index > 0 && *index <= 20)
            .ok_or_else(denied)?;
        if scopes.insert(index, parse(related)?).is_some() {
            return Err(denied());
        }
    }
    Ok(scopes)
}
pub(crate) fn validate_accepted_context_scope(value: &Value) -> Result<(), String> {
    let denied = || {
        "Onboarding defaults unavailable; new boundary and explicit acceptance required".to_string()
    };
    let scopes = accepted_context_scopes(value)?;
    let allows = |index: &Value, path: &Value| {
        index
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| scopes.get(&index))
            .zip(path.as_str())
            .is_some_and(|(scope, path)| scope.allows(path) && !crate::sensitive_path(path))
    };
    for component in value["components"].as_array().ok_or_else(denied)? {
        if component["roots"]
            .as_array()
            .ok_or_else(denied)?
            .iter()
            .any(|path| !allows(&component["repository_index"], path))
        {
            return Err(denied());
        }
        for evidence in component["evidence"].as_array().ok_or_else(denied)? {
            if !allows(&evidence["repository_index"], &evidence["path"]) {
                return Err(denied());
            }
        }
    }
    for kind in ["integrations", "providers"] {
        for item in value[kind].as_array().ok_or_else(denied)? {
            if kind == "providers" && !allows(&item["repository_index"], &item["source_root"]) {
                return Err(denied());
            }
            let evidence = item["evidence"].as_array().ok_or_else(denied)?;
            if evidence.is_empty()
                || evidence
                    .iter()
                    .any(|evidence| !allows(&evidence["repository_index"], &evidence["path"]))
            {
                return Err(denied());
            }
        }
    }
    Ok(())
}

/// Read the protected history only to check legacy label provenance; no original
/// label, path or observation is returned to the separately authorized viewer.
pub(crate) fn validate_accepted_context_interpretations(
    value: &Value,
    original: &[ComponentProposal],
) -> Result<(), String> {
    let denied = || {
        "Onboarding defaults unavailable; new boundary and explicit acceptance required".to_string()
    };
    let scopes = accepted_context_scopes(value)?;
    let allows = |index: usize, path: &str| {
        scopes
            .get(&index)
            .is_some_and(|scope| scope.allows(path) && !crate::sensitive_path(path))
    };
    for component in value["components"].as_array().ok_or_else(denied)? {
        let source = original
            .iter()
            .find(|source| component["id"] == source.id)
            .ok_or_else(denied)?;
        let lost_provenance = source
            .evidence
            .iter()
            .any(|evidence| !allows(evidence.repository_index, &evidence.path))
            || source.contradictions.iter().any(|limit| {
                limit
                    .strip_suffix(" content unavailable")
                    .is_some_and(|path| !allows(source.repository_index, path))
            });
        if lost_provenance
            && ["stack_candidates", "contradictions"].iter().any(|field| {
                component[field]
                    .as_array()
                    .is_none_or(|values| !values.is_empty())
            })
        {
            return Err(denied());
        }
    }
    Ok(())
}
fn view(
    session: &OnboardingSession,
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<Value, HttpFailure> {
    let scope = session.scope();
    let pending = session.pending_decisions();
    let context = accepted_context_view(session, principal, grants).map_err(failure)?;
    let has_derived_gaps = context["gaps"]
        .as_array()
        .is_some_and(|gaps| !gaps.is_empty());
    Ok(
        json!({"api_version":"v1","onboarding_version":crate::ONBOARDING_VERSION,"id":session.id(),"revision":session.revision(),
        "state":session.state(),"commit":session.commit().as_str(),"scan_version":session.scan_version(),
        "input_fingerprint":session.input_fingerprint(),"proposal_fingerprint":session.proposal_fingerprint(),
        "repository":repo_json(&scope.primary_repository),"scope_version":scope.version.as_str(),
        "include":scope.primary_paths.includes().collect::<Vec<_>>(),"exclude":scope.primary_paths.excludes().collect::<Vec<_>>(),
        "contexts":scope.contexts.iter().map(|pin|json!({"name":pin.name,"repository":repo_json(&pin.repository),"commit":pin.commit.as_str(),
            "include":pin.paths.includes().collect::<Vec<_>>(),"exclude":pin.paths.excludes().collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "inventory":session.inventory(),"candidate_roots":session.inventory().map(|item|item.candidate_roots()),"components":session.proposals(),"providers":session.provider_mappings(),"gaps":session.gaps(),"confirmation":session.confirmation(),
        "questions":session.questions(),"integrations":session.integrations(),"accepted_context":context,
        "confirmation_readiness":{"components_pending":pending.0,"providers_pending":pending.1,"integrations_pending":pending.2,"known_gaps_required":session.known_gaps_required() || has_derived_gaps,"ready":session.state()==OnboardingState::AwaitingConfirmation && pending==(0,0,0)},
        "review_defaults_active":false,"ai_dispatch_available":false}),
    )
}
impl<A, P, R> HttpBackend for OnboardingHost<A, P, R>
where
    A: ServiceAuthenticator + Send + Sync + 'static,
    P: OnboardingAccessProvider,
    R: OnboardingSource,
{
    fn execute(&self, bearer: &str, operation: HttpOperation) -> Result<Value, HttpFailure> {
        if !matches!(
            operation,
            HttpOperation::CreateOnboarding { .. }
                | HttpOperation::ReadOnboarding { .. }
                | HttpOperation::MutateOnboarding { .. }
        ) {
            return self.fallback.execute(bearer, operation);
        }
        let principal = self
            .authenticator
            .authenticate(bearer)
            .map_err(|_| HttpFailure::Authentication)?;
        let mut storage = self.storage.lock().map_err(|_| HttpFailure::Unavailable)?;
        match operation {
            HttpOperation::CreateOnboarding { request } => {
                let request: Create =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                let mut scope = crate::http::scope(request.scope)?;
                scope.version = crate::ScopeVersion::new(request.scope_version).map_err(failure)?;
                let grants = self
                    .access
                    .grants(&principal, scope.primary_repository.security_domain())
                    .map_err(failure)?;
                let scan_version = if request.sample_env_names {
                    crate::onboarding_scan::ENV_NAME_SCAN_VERSION
                } else {
                    crate::INVENTORY_VERSION
                };
                // Validate the entire approved boundary before invoking ANY repository reader.
                let preview = OnboardingSession::new(
                    request.id.clone(),
                    principal.clone(),
                    SnapshotId::new("0".repeat(40)).unwrap(),
                    scope,
                    scan_version.into(),
                    &grants,
                )
                .map_err(failure)?;
                let mut canonical_scope = preview.scope().clone();
                if let Some(version) = storage
                    .onboarding_scope_version(&preview, &principal, &grants)
                    .map_err(failure)?
                {
                    canonical_scope.version = version;
                }
                let commit = self
                    .source
                    .resolve(&preview.scope().primary_repository, &request.reference)
                    .map_err(failure)?;
                for pin in &preview.scope().contexts {
                    if self
                        .source
                        .resolve(&pin.repository, pin.commit.as_str())
                        .map_err(failure)?
                        != pin.commit
                    {
                        return Err(HttpFailure::Conflict);
                    }
                }
                let session = OnboardingSession::new(
                    request.id,
                    principal.clone(),
                    commit,
                    canonical_scope,
                    scan_version.into(),
                    &grants,
                )
                .map_err(failure)?;
                let grants = self
                    .access
                    .grants(
                        &principal,
                        session.scope().primary_repository.security_domain(),
                    )
                    .map_err(failure)?;
                storage
                    .save_onboarding_session(&session, None, now()?, &principal, &grants)
                    .map_err(failure)?;
                view(&session, &principal, &grants)
            }
            HttpOperation::ReadOnboarding { id, domain } => {
                let domain = SecurityDomain::new(domain).map_err(failure)?;
                let grants = self.access.grants(&principal, &domain).map_err(failure)?;
                let session = storage
                    .load_onboarding_session(&domain, &id, &principal, &grants)
                    .map_err(failure)?;
                let mut result = view(&session, &principal, &grants)?;
                result["review_defaults_active"] = json!(
                    storage
                        .onboarding_defaults_match(&session, &principal, &grants)
                        .map_err(failure)?
                );
                Ok(result)
            }
            HttpOperation::MutateOnboarding {
                id,
                action,
                request,
            } => {
                let allowed: &[&str] = match action.as_str() {
                    "decisions" => &["component"],
                    "provider-decisions" => &["provider_index", "provider"],
                    "integration-decisions" => &["integration_id", "decision", "answer"],
                    "confirm" => &[
                        "proposal_fingerprint",
                        "known_gaps_accepted",
                        "activate_defaults",
                    ],
                    "activate" => &["proposal_fingerprint"],
                    "defaults" => &["repository"],
                    "drift" => &["repository", "reference"],
                    "scan" | "detect" | "inspect" | "cancel" => &[],
                    _ => return Err(HttpFailure::Malformed),
                };
                if request.as_object().is_none_or(|object| {
                    object.keys().any(|key| {
                        key != "security_domain"
                            && key != "expected_revision"
                            && !allowed.contains(&key.as_str())
                    })
                }) {
                    return Err(HttpFailure::Malformed);
                }
                let request: Mutation =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                let domain = SecurityDomain::new(request.security_domain).map_err(failure)?;
                let grants = self.access.grants(&principal, &domain).map_err(failure)?;
                if action == "defaults" {
                    let repository = request.repository.ok_or(HttpFailure::Malformed)?;
                    if repository.security_domain != domain.as_str() {
                        return Err(HttpFailure::Malformed);
                    }
                    let repository = RepositoryId::new(
                        domain,
                        repository.provider,
                        repository.owner,
                        repository.name,
                    )
                    .map_err(failure)?;
                    return storage
                        .load_onboarding_defaults(&repository, &principal, &grants)
                        .map_err(failure);
                }
                if action == "drift" {
                    let context = if let Some(repository) = request.repository {
                        if repository.security_domain != domain.as_str() {
                            return Err(HttpFailure::Malformed);
                        }
                        let repository = RepositoryId::new(
                            domain.clone(),
                            repository.provider,
                            repository.owner,
                            repository.name,
                        )
                        .map_err(failure)?;
                        storage
                            .load_onboarding_defaults(&repository, &principal, &grants)
                            .map_err(failure)?
                    } else {
                        let session = storage
                            .load_onboarding_session(&domain, &id, &principal, &grants)
                            .map_err(failure)?;
                        if session.state() != OnboardingState::Confirmed {
                            return Err(HttpFailure::Malformed);
                        }
                        accepted_context_view(&session, &principal, &grants).map_err(failure)?
                    };
                    if context["session_id"] != id
                        || context["session_revision"] != request.expected_revision
                    {
                        return Err(HttpFailure::Conflict);
                    }
                    let repo = &context["repository"];
                    let repository = RepositoryId::new(
                        domain.clone(),
                        repo["provider"].as_str().ok_or(HttpFailure::Internal)?,
                        repo["owner"].as_str().ok_or(HttpFailure::Internal)?,
                        repo["name"].as_str().ok_or(HttpFailure::Internal)?,
                    )
                    .map_err(failure)?;
                    let target = self
                        .source
                        .resolve(
                            &repository,
                            request.reference.as_deref().ok_or(HttpFailure::Malformed)?,
                        )
                        .map_err(failure)?;
                    let fresh = self.access.grants(&principal, &domain).map_err(failure)?;
                    let result = self
                        .source
                        .drift(&context, &target, &principal, &fresh)
                        .map_err(failure)?;
                    let current = self.access.grants(&principal, &domain).map_err(failure)?;
                    crate::onboarding::drift::validate_access(&context, &principal, &current)
                        .map_err(failure)?;
                    return Ok(result);
                }
                let mut session = storage
                    .load_onboarding_session(&domain, &id, &principal, &grants)
                    .map_err(failure)?;
                let before = session.revision();
                match action.as_str() {
                    "detect" => {
                        if request.component.is_some()
                            || request.proposal_fingerprint.is_some()
                            || request.known_gaps_accepted.is_some()
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        if request.expected_revision != session.revision() {
                            return Err(HttpFailure::Conflict);
                        }
                        let (proposals, mut gaps) = self
                            .source
                            .detect(&session, &principal, &grants)
                            .map_err(failure)?;
                        let (mappings, provider_gaps) = self
                            .source
                            .providers(&session, &proposals, &principal, &grants)
                            .map_err(failure)?;
                        gaps.extend(provider_gaps);
                        gaps.sort();
                        gaps.dedup();
                        if gaps.len() > 100 {
                            gaps.truncate(99);
                            gaps.push("Additional proposal gaps exceed the display budget; coverage remains incomplete".into());
                        }
                        let fresh = self.access.grants(&principal, &domain).map_err(failure)?;
                        session
                            .set_proposal(
                                request.expected_revision,
                                proposals,
                                mappings,
                                gaps,
                                &principal,
                                &fresh,
                            )
                            .map_err(failure)?;
                    }
                    "inspect" => {
                        if request.component.is_some()
                            || request.proposal_fingerprint.is_some()
                            || request.known_gaps_accepted.is_some()
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        session
                            .transition(
                                request.expected_revision,
                                OnboardingState::AwaitingConfirmation,
                                &principal,
                                &grants,
                            )
                            .map_err(failure)?;
                    }
                    "scan" => {
                        if request.component.is_some()
                            || request.proposal_fingerprint.is_some()
                            || request.known_gaps_accepted.is_some()
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        if request.expected_revision != session.revision() {
                            return Err(HttpFailure::Conflict);
                        }
                        let inventory = self
                            .source
                            .scan(&session, &principal, &grants)
                            .map_err(failure)?;
                        let fresh = self.access.grants(&principal, &domain).map_err(failure)?;
                        session
                            .record_inventory(
                                request.expected_revision,
                                inventory,
                                &principal,
                                &fresh,
                            )
                            .map_err(failure)?;
                    }
                    "decisions" => {
                        if request.proposal_fingerprint.is_some()
                            || request.known_gaps_accepted.is_some()
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        let replacement = request.component.ok_or(HttpFailure::Malformed)?;
                        let old = session
                            .proposals()
                            .iter()
                            .find(|item| item.id == replacement.id)
                            .ok_or(HttpFailure::Malformed)?;
                        // Browser corrections may change labels/roots/decision, never invent detector evidence.
                        if replacement.evidence != old.evidence
                            || replacement.signals != old.signals
                            || replacement.repository_index != old.repository_index
                            || replacement.stack_candidates != old.stack_candidates
                            || replacement.contradictions != old.contradictions
                            || replacement.confidence != old.confidence
                            || replacement.decision == ComponentDecision::Pending
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        session
                            .decide_component(
                                request.expected_revision,
                                replacement,
                                &principal,
                                &grants,
                            )
                            .map_err(failure)?;
                    }
                    "provider-decisions" => {
                        session
                            .decide_provider(
                                request.expected_revision,
                                request.provider_index.ok_or(HttpFailure::Malformed)?,
                                request.provider.ok_or(HttpFailure::Malformed)?,
                                &principal,
                                &grants,
                            )
                            .map_err(failure)?;
                    }
                    "integration-decisions" => {
                        session
                            .decide_integration(
                                request.expected_revision,
                                request
                                    .integration_id
                                    .as_deref()
                                    .ok_or(HttpFailure::Malformed)?,
                                request.decision.ok_or(HttpFailure::Malformed)?,
                                request.answer,
                                &principal,
                                &grants,
                            )
                            .map_err(failure)?;
                    }
                    "activate" => {
                        if request.expected_revision != session.revision()
                            || request.proposal_fingerprint.as_deref()
                                != Some(session.proposal_fingerprint().as_str())
                        {
                            return Err(HttpFailure::Conflict);
                        }
                        if session.state() != OnboardingState::Confirmed {
                            return Err(HttpFailure::Malformed);
                        }
                    }
                    "confirm" => {
                        if request.component.is_some() {
                            return Err(HttpFailure::Malformed);
                        }
                        authorize_repository(
                            Action::ConfirmArchitecture,
                            &principal,
                            &session.scope().primary_repository,
                            &grants,
                            None,
                            &session.scope().primary_paths,
                        )
                        .map_err(failure)?;
                        if !grants.iter().any(|grant| {
                            grant.principal == principal
                                && grant.repository == session.scope().primary_repository
                                && grant.actions.contains(&Action::ConfirmArchitecture)
                                && crate::onboarding::scope_is_covered(
                                    &session.scope().primary_paths,
                                    &grant.paths,
                                )
                        }) {
                            return Err(HttpFailure::DeniedOrMissing);
                        }
                        let context = accepted_context_view(&session, &principal, &grants)
                            .map_err(failure)?;
                        if request.known_gaps_accepted != Some(true)
                            && context["gaps"]
                                .as_array()
                                .is_some_and(|gaps| !gaps.is_empty())
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        session
                            .confirm(
                                request.expected_revision,
                                request
                                    .proposal_fingerprint
                                    .as_deref()
                                    .ok_or(HttpFailure::Malformed)?,
                                request.known_gaps_accepted.ok_or(HttpFailure::Malformed)?,
                                now()?,
                                &principal,
                                &grants,
                            )
                            .map_err(failure)?;
                    }
                    "cancel" => {
                        if request.component.is_some()
                            || request.proposal_fingerprint.is_some()
                            || request.known_gaps_accepted.is_some()
                        {
                            return Err(HttpFailure::Malformed);
                        }
                        session
                            .transition(
                                request.expected_revision,
                                OnboardingState::Cancelled,
                                &principal,
                                &grants,
                            )
                            .map_err(failure)?;
                    }
                    _ => return Err(HttpFailure::Malformed),
                }
                let grants = self.access.grants(&principal, &domain).map_err(failure)?;
                storage
                    .save_onboarding_session_with_defaults(
                        &session,
                        Some(before),
                        now()?,
                        &principal,
                        &grants,
                        request.activate_defaults || action == "activate",
                    )
                    .map_err(failure)?;
                let mut result = view(&session, &principal, &grants)?;
                result["review_defaults_active"] = json!(
                    storage
                        .onboarding_defaults_match(&session, &principal, &grants)
                        .map_err(failure)?
                );
                Ok(result)
            }
            _ => Err(HttpFailure::Internal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PathScope, ScopeRecord, ScopeVersion, router};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use std::{
        collections::BTreeSet,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt;
    struct Auth;
    impl ServiceAuthenticator for Auth {
        fn authenticate(&self, token: &str) -> Result<PrincipalId, String> {
            if token == "test-token" {
                PrincipalId::new("maintainer")
            } else if token == "viewer-token" {
                PrincipalId::new("viewer")
            } else {
                Err("Authentication required".into())
            }
        }
    }
    struct Access(Arc<Mutex<Vec<Grant>>>);
    impl OnboardingAccessProvider for Access {
        fn grants(&self, _: &PrincipalId, _: &SecurityDomain) -> Result<Vec<Grant>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
    }
    struct Source(Arc<AtomicUsize>);
    impl OnboardingSource for Source {
        fn resolve(&self, _: &RepositoryId, reference: &str) -> Result<SnapshotId, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            SnapshotId::new(if reference == "new-head" { "c" } else { "a" }.repeat(40))
        }
        fn scan(
            &self,
            session: &OnboardingSession,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<crate::OnboardingInventory, String> {
            session.check_access(principal, grants)?;
            if session.id() != "recovery-session" {
                return Err("Scan unavailable".into());
            }
            let mut inventory = crate::OnboardingInventory::empty(session);
            inventory.complete = true;
            inventory.items.push(crate::InventoryItem {
                repository_index: 0,
                path: "src/lib.rs".into(),
                blob: "b".repeat(40),
                kind: "source".into(),
                status: "indexed".into(),
                variable_names: vec![],
            });
            Ok(inventory)
        }
        fn drift(
            &self,
            context: &Value,
            target: &SnapshotId,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<Value, String> {
            crate::onboarding::drift::validate_access(context, principal, grants)?;
            // Transport fixture only; real Git structural assessment is tested in drift.rs.
            Ok(
                json!({"version":"csb-onboarding-drift-v1","session_id":context["session_id"],"session_revision":context["session_revision"],"source_commit":context["commit"],"target_commit":target.as_str(),"input_fingerprint":context["input_fingerprint"],"proposal_fingerprint":context["proposal_fingerprint"],"accepted_scope_fingerprint":context["accepted_scope_fingerprint"],"status":"unchanged","reuse_allowed":true,"reconfirmation_required":false,"affected_components":[],"gaps":[],"defaults_changed":false,"scope_expanded":false}),
            )
        }
    }
    struct Fallback;
    impl HttpBackend for Fallback {
        fn execute(&self, _: &str, _: HttpOperation) -> Result<Value, HttpFailure> {
            Err(HttpFailure::DeniedOrMissing)
        }
    }
    type TestSetup = (
        Arc<OnboardingHost<Auth, Access, Source>>,
        Arc<Mutex<Vec<Grant>>>,
        Arc<AtomicUsize>,
        PathBuf,
    );
    fn setup(confirmed_authority: bool) -> TestSetup {
        static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "csb-onboarding-http-{}-{}-{}",
            std::process::id(),
            now().unwrap(),
            NEXT_TEST.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("test").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let actions = if confirmed_authority {
            BTreeSet::from([Action::Index, Action::ConfirmArchitecture])
        } else {
            BTreeSet::from([Action::Index])
        };
        let grants = Arc::new(Mutex::new(vec![Grant {
            principal: PrincipalId::new("maintainer").unwrap(),
            repository,
            actions,
            paths: PathScope::all(),
        }]));
        let count = Arc::new(AtomicUsize::new(0));
        let storage = Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        (
            Arc::new(OnboardingHost::new(
                storage,
                Auth,
                Access(grants.clone()),
                Source(count.clone()),
                Arc::new(Fallback),
            )),
            grants,
            count,
            root,
        )
    }
    fn create() -> Value {
        json!({"id":"session-1","reference":"main","scope_version":"boundary-1","scope":{
        "primary_repository":{"security_domain":"test","provider":"github","owner":"owner","name":"repo"},
        "primary_include":["src"],"primary_exclude":["src/private"],"contexts":[],"contracts":[],"export_allowed":false,"model_destinations":[]}})
    }
    #[test]
    fn repeated_boundary_reuses_registered_scope_without_rewriting_confirmed_session() {
        let (host, grants, count, root) = setup(true);
        let first = host
            .execute(
                "test-token",
                HttpOperation::CreateOnboarding { request: create() },
            )
            .unwrap();
        let principal = PrincipalId::new("maintainer").unwrap();
        let current = grants.lock().unwrap().clone();
        {
            let mut storage = host.storage.lock().unwrap();
            let mut session = storage
                .load_onboarding_session(
                    current[0].repository.security_domain(),
                    "session-1",
                    &principal,
                    &current,
                )
                .unwrap();
            session
                .transition(0, OnboardingState::Scanning, &principal, &current)
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(0), 1, &principal, &current)
                .unwrap();
            session
                .set_proposal(
                    1,
                    vec![],
                    vec![],
                    vec!["Fixture has no component proposal".into()],
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(1), 2, &principal, &current)
                .unwrap();
            session
                .transition(
                    2,
                    OnboardingState::AwaitingConfirmation,
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(2), 3, &principal, &current)
                .unwrap();
            session
                .confirm(
                    3,
                    &session.proposal_fingerprint(),
                    true,
                    4,
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(3), 4, &principal, &current)
                .unwrap();
        }
        let original = host
            .execute(
                "test-token",
                HttpOperation::ReadOnboarding {
                    id: "session-1".into(),
                    domain: "test".into(),
                },
            )
            .unwrap();
        let mut second = create();
        second["id"] = json!("session-2");
        second["reference"] = json!("new-head");
        second["scope_version"] = json!("new-random-boundary-version");
        let created = host
            .execute(
                "test-token",
                HttpOperation::CreateOnboarding { request: second },
            )
            .unwrap();
        assert_eq!(created["scope_version"], first["scope_version"]);
        assert_eq!(created["id"], "session-2");
        assert_eq!(created["commit"], "c".repeat(40));
        assert_ne!(created["input_fingerprint"], first["input_fingerprint"]);
        assert_eq!(
            host.execute(
                "test-token",
                HttpOperation::ReadOnboarding {
                    id: "session-1".into(),
                    domain: "test".into()
                }
            )
            .unwrap(),
            original
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        grants.lock().unwrap().clear();
        let mut denied = create();
        denied["id"] = json!("denied");
        assert_eq!(
            host.execute(
                "test-token",
                HttpOperation::CreateOnboarding { request: denied }
            ),
            Err(HttpFailure::DeniedOrMissing)
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn accepted_related_context_keeps_original_repository_ordinal_after_exclusion() {
        let (host, grants, _, root) = setup(true);
        let principal = PrincipalId::new("maintainer").unwrap();
        let mut current = grants.lock().unwrap().clone();
        let repository = current[0].repository.clone();
        let mut scope = ScopeRecord {
            version: ScopeVersion::new("related-ordinal-boundary").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        for name in ["a-excluded", "b-retained"] {
            let related = RepositoryId::new(
                repository.security_domain().clone(),
                "github",
                "owner",
                name,
            )
            .unwrap();
            scope.contexts.push(crate::ContextPin {
                name: name.into(),
                repository: related.clone(),
                commit: SnapshotId::new("c".repeat(40)).unwrap(),
                paths: PathScope::all(),
            });
            current.push(Grant {
                principal: principal.clone(),
                repository: related,
                actions: BTreeSet::from([Action::Index]),
                paths: PathScope::all(),
            });
        }
        let mut session = OnboardingSession::new(
            "ordinal-session".into(),
            principal.clone(),
            SnapshotId::new("a".repeat(40)).unwrap(),
            scope,
            "v1".into(),
            &current,
        )
        .unwrap();
        let mut storage = host.storage.lock().unwrap();
        storage
            .save_onboarding_session(&session, None, 0, &principal, &current)
            .unwrap();
        session
            .transition(0, OnboardingState::Scanning, &principal, &current)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(0), 1, &principal, &current)
            .unwrap();
        let components = (0..=2)
            .map(|index| ComponentProposal {
                id: format!("component-{index}"),
                name: format!("Component {index}"),
                role: "backend".into(),
                repository_index: index,
                roots: vec!["src".into()],
                stack_candidates: vec!["Rust".into()],
                signals: vec![],
                evidence: vec![crate::OnboardingEvidence {
                    repository_index: index,
                    path: "src/lib.rs".into(),
                    blob: "b".repeat(40),
                    start_line: 1,
                    end_line: 1,
                    producer_version: "fixture-v1".into(),
                }],
                confidence: crate::EvidenceConfidence::Candidate,
                contradictions: vec![],
                decision: ComponentDecision::Pending,
            })
            .collect::<Vec<_>>();
        let mapping = crate::ComponentProviderMapping {
            component_id: "component-2".into(),
            provider: "Vercel".into(),
            project_id: None,
            service_id: None,
            environment: "unknown".into(),
            repository_index: 2,
            source_root: "src".into(),
            observed_commit: Some("c".repeat(40)),
            verified_at_ms: None,
            verification: crate::ProviderVerification::NotRequested,
            evidence: components[2].evidence.clone(),
            notes: vec![],
            decision: ComponentDecision::Pending,
        };
        session
            .set_proposal(
                1,
                components.clone(),
                vec![mapping.clone()],
                vec![],
                &principal,
                &current,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(1), 2, &principal, &current)
            .unwrap();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &current,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(2), 3, &principal, &current)
            .unwrap();
        for mut component in components {
            let revision = session.revision();
            component.decision = if component.repository_index == 1 {
                ComponentDecision::Rejected
            } else {
                ComponentDecision::Confirmed
            };
            session
                .decide_component(revision, component, &principal, &current)
                .unwrap();
            storage
                .save_onboarding_session(
                    &session,
                    Some(revision),
                    revision as i64 + 1,
                    &principal,
                    &current,
                )
                .unwrap();
        }
        let revision = session.revision();
        let mut mapping = mapping;
        mapping.decision = ComponentDecision::Confirmed;
        session
            .decide_provider(revision, 0, mapping, &principal, &current)
            .unwrap();
        storage
            .save_onboarding_session(
                &session,
                Some(revision),
                revision as i64 + 1,
                &principal,
                &current,
            )
            .unwrap();
        let context = accepted_context_view(&session, &principal, &current).unwrap();
        assert_eq!(context["contexts"].as_array().unwrap().len(), 1);
        assert_eq!(context["contexts"][0]["repository_index"], 2);
        assert_eq!(context["providers"][0]["repository_index"], 2);
        validate_accepted_context_scope(&context).unwrap();
        let revision = session.revision();
        session
            .confirm(
                revision,
                &session.proposal_fingerprint(),
                true,
                20,
                &principal,
                &current,
            )
            .unwrap();
        storage
            .save_onboarding_session_with_defaults(
                &session,
                Some(revision),
                20,
                &principal,
                &current,
                true,
            )
            .unwrap();
        let viewer = PrincipalId::new("viewer").unwrap();
        let narrow = [
            repository.clone(),
            session.scope().contexts[1].repository.clone(),
        ]
        .into_iter()
        .map(|repository| Grant {
            principal: viewer.clone(),
            repository,
            actions: BTreeSet::from([Action::Index]),
            paths: PathScope::new(vec!["src".into()], vec![]).unwrap(),
        })
        .collect::<Vec<_>>();
        let active = storage
            .load_onboarding_defaults(&repository, &viewer, &narrow)
            .unwrap();
        assert_eq!(active["contexts"][0]["repository_index"], 2);
        assert!(!active.to_string().contains("a-excluded"));
        let mut missing_index = active.clone();
        missing_index["contexts"][0]
            .as_object_mut()
            .unwrap()
            .remove("repository_index");
        assert!(validate_accepted_context_scope(&missing_index).is_err());
        drop(storage);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepted_metadata_filters_provider_roots_and_labels_and_denies_legacy_leaks() {
        let (host, grants, _, root) = setup(true);
        let principal = PrincipalId::new("maintainer").unwrap();
        let current = grants.lock().unwrap().clone();
        let repository = current[0].repository.clone();
        let scope = ScopeRecord {
            version: ScopeVersion::new("metadata-boundary").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let mut session = OnboardingSession::new(
            "metadata-session".into(),
            principal.clone(),
            SnapshotId::new("a".repeat(40)).unwrap(),
            scope,
            "v1".into(),
            &current,
        )
        .unwrap();
        let save = |session: &OnboardingSession, expected| {
            host.storage
                .lock()
                .unwrap()
                .save_onboarding_session(
                    session,
                    expected,
                    session.revision() as i64,
                    &principal,
                    &current,
                )
                .unwrap()
        };
        save(&session, None);
        let mut inventory = crate::OnboardingInventory::empty(&session);
        inventory.complete = true;
        for path in ["src/lib.rs", "private/hidden.rs"] {
            inventory.items.push(crate::InventoryItem {
                repository_index: 0,
                path: path.into(),
                blob: "b".repeat(40),
                kind: "source".into(),
                status: "indexed".into(),
                variable_names: vec![],
            });
        }
        session
            .record_inventory(0, inventory, &principal, &current)
            .unwrap();
        save(&session, Some(0));
        let evidence = |path: &str| crate::OnboardingEvidence {
            repository_index: 0,
            path: path.into(),
            blob: "b".repeat(40),
            start_line: 1,
            end_line: 1,
            producer_version: "fixture-v1".into(),
        };
        let mut api = ComponentProposal {
            id: "api".into(),
            name: "User API".into(),
            role: "backend".into(),
            repository_index: 0,
            roots: vec![".".into()],
            stack_candidates: vec!["PRIVATE_STACK_SENTINEL · static import".into()],
            signals: vec![],
            evidence: vec![evidence("src/lib.rs"), evidence("private/hidden.rs")],
            confidence: crate::EvidenceConfidence::Candidate,
            contradictions: vec!["private/missing.rs content unavailable".into()],
            decision: ComponentDecision::Pending,
        };
        let mut excluded = ComponentProposal {
            id: "excluded".into(),
            name: "PRIVATE_COMPONENT_SENTINEL".into(),
            role: "backend".into(),
            repository_index: 0,
            roots: vec!["private".into()],
            stack_candidates: vec![],
            signals: vec![],
            evidence: vec![evidence("private/hidden.rs")],
            confidence: crate::EvidenceConfidence::Candidate,
            contradictions: vec![],
            decision: ComponentDecision::Pending,
        };
        let mappings = [
            ("Narrowed ancestor", ".", "src/lib.rs"),
            ("PRIVATE_PROVIDER_SENTINEL", "src", "private/hidden.rs"),
            ("No retained provenance", "src", "private/hidden.rs"),
            ("Safe provider", "src", "src/lib.rs"),
        ]
        .into_iter()
        .map(
            |(provider, source_root, path)| crate::ComponentProviderMapping {
                component_id: "api".into(),
                provider: provider.into(),
                project_id: None,
                service_id: None,
                environment: "unknown".into(),
                repository_index: 0,
                source_root: source_root.into(),
                observed_commit: Some("a".repeat(40)),
                verified_at_ms: None,
                verification: crate::ProviderVerification::NotRequested,
                evidence: vec![evidence(path)],
                notes: vec![],
                decision: ComponentDecision::Pending,
            },
        )
        .collect::<Vec<_>>();
        session
            .set_proposal(
                1,
                vec![api.clone(), excluded.clone()],
                mappings.clone(),
                vec![],
                &principal,
                &current,
            )
            .unwrap();
        save(&session, Some(1));
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &current,
            )
            .unwrap();
        save(&session, Some(2));
        api.roots = vec!["src".into()];
        api.decision = ComponentDecision::Corrected;
        session
            .decide_component(3, api, &principal, &current)
            .unwrap();
        save(&session, Some(3));
        excluded.decision = ComponentDecision::Rejected;
        session
            .decide_component(4, excluded, &principal, &current)
            .unwrap();
        save(&session, Some(4));
        for (index, mut mapping) in mappings.into_iter().enumerate() {
            let revision = session.revision();
            mapping.decision = ComponentDecision::Confirmed;
            if index == 1 {
                mapping.source_root = "private".into();
                mapping.decision = ComponentDecision::Corrected;
            }
            session
                .decide_provider(revision, index, mapping, &principal, &current)
                .unwrap();
            save(&session, Some(revision));
        }
        let preview = view(&session, &principal, &current).unwrap();
        assert_eq!(
            preview["confirmation_readiness"]["known_gaps_required"],
            true
        );
        let context = &preview["accepted_context"];
        assert_eq!(context["providers"].as_array().unwrap().len(), 1);
        assert_eq!(context["providers"][0]["provider"], "Safe provider");
        assert_eq!(context["components"][0]["name"], "User API");
        assert_eq!(context["components"][0]["stack_candidates"], json!([]));
        assert_eq!(context["components"][0]["contradictions"], json!([]));
        assert!(!context.to_string().contains("PRIVATE_"));
        assert!(!context["gaps"].to_string().contains("private"));
        let revision = session.revision();
        let confirm = |accepted| {
            host.execute("test-token",HttpOperation::MutateOnboarding{id:session.id().into(),action:"confirm".into(),request:json!({"security_domain":"test","expected_revision":revision,"proposal_fingerprint":session.proposal_fingerprint(),"known_gaps_accepted":accepted,"activate_defaults":true})})
        };
        assert_eq!(confirm(false), Err(HttpFailure::Malformed));
        confirm(true).unwrap();
        let viewer = PrincipalId::new("viewer").unwrap();
        let narrow = vec![Grant {
            principal: viewer.clone(),
            repository: repository.clone(),
            actions: BTreeSet::from([Action::Index]),
            paths: PathScope::new(vec!["src".into()], vec![]).unwrap(),
        }];
        let storage = host.storage.lock().unwrap();
        let active = storage
            .load_onboarding_defaults(&repository, &viewer, &narrow)
            .unwrap();
        assert!(!active.to_string().contains("PRIVATE_"));
        let (context_id,payload):(String,Vec<u8>)=storage.connection.query_row("SELECT context.context_id,context.payload FROM onboarding_defaults selected JOIN onboarding_accepted_contexts context ON context.repository_id=selected.repository_id AND context.context_id=selected.context_id",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        let history:Vec<u8>=storage.connection.query_row("SELECT payload FROM onboarding_history WHERE session_id='metadata-session' ORDER BY revision DESC LIMIT 1",[],|row|row.get(0)).unwrap();
        for field in ["stack_candidates", "contradictions"] {
            let mut legacy = active.clone();
            legacy["components"][0][field] = json!(["PRIVATE_METADATA_SENTINEL"]);
            storage
                .connection
                .execute(
                    "UPDATE onboarding_accepted_contexts SET payload=?1",
                    [serde_json::to_vec(&legacy).unwrap()],
                )
                .unwrap();
            assert!(
                storage
                    .load_onboarding_defaults(&repository, &viewer, &narrow)
                    .is_err()
            );
            assert_eq!(
                storage
                    .connection
                    .query_row("SELECT context_id FROM onboarding_defaults", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap(),
                context_id
            );
            assert_eq!(storage.connection.query_row("SELECT payload FROM onboarding_history WHERE session_id='metadata-session' ORDER BY revision DESC LIMIT 1",[],|row|row.get::<_,Vec<u8>>(0)).unwrap(),history);
        }
        let mut legacy = active.clone();
        legacy["providers"][0]["source_root"] = json!("private");
        storage
            .connection
            .execute(
                "UPDATE onboarding_accepted_contexts SET payload=?1",
                [serde_json::to_vec(&legacy).unwrap()],
            )
            .unwrap();
        assert!(
            storage
                .load_onboarding_defaults(&repository, &viewer, &narrow)
                .is_err()
        );
        let mut legacy = active.clone();
        legacy["providers"][0]["evidence"] = json!([]);
        storage
            .connection
            .execute(
                "UPDATE onboarding_accepted_contexts SET payload=?1",
                [serde_json::to_vec(&legacy).unwrap()],
            )
            .unwrap();
        assert!(
            storage
                .load_onboarding_defaults(&repository, &viewer, &narrow)
                .is_err()
        );
        storage
            .connection
            .execute(
                "UPDATE onboarding_accepted_contexts SET payload=?1",
                [payload.clone()],
            )
            .unwrap();
        assert_eq!(
            storage
                .load_onboarding_defaults(&repository, &viewer, &narrow)
                .unwrap(),
            active
        );
        assert_eq!(
            storage
                .connection
                .query_row(
                    "SELECT payload FROM onboarding_accepted_contexts",
                    [],
                    |row| row.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            payload
        );
        let original_session = storage
            .load_onboarding_session(
                repository.security_domain(),
                "metadata-session",
                &principal,
                &current,
            )
            .unwrap();
        assert_eq!(original_session.proposals()[0].evidence.len(), 2);
        assert!(
            original_session.proposals()[0].stack_candidates[0].contains("PRIVATE_STACK_SENTINEL")
        );
        assert_eq!(
            original_session.provider_mappings()[1].source_root,
            "private"
        );
        // Recovery must remain possible while the old pointer is deliberately
        // invalid. It changes only after a new, explicit activation.
        let mut legacy = active.clone();
        legacy["providers"][0]["source_root"] = json!("private");
        let invalid_payload = serde_json::to_vec(&legacy).unwrap();
        storage
            .connection
            .execute(
                "UPDATE onboarding_accepted_contexts SET payload=?1",
                [&invalid_payload],
            )
            .unwrap();
        drop(storage);
        let mut recovery = create();
        recovery["id"] = json!("recovery-session");
        recovery["reference"] = json!("new-head");
        recovery["scope_version"] = json!("recovery-boundary");
        let recovery = host
            .execute(
                "test-token",
                HttpOperation::CreateOnboarding { request: recovery },
            )
            .unwrap();
        assert_eq!(recovery["commit"], "c".repeat(40));
        let scanned = host
            .execute(
                "test-token",
                HttpOperation::MutateOnboarding {
                    id: "recovery-session".into(),
                    action: "scan".into(),
                    request: json!({"security_domain":"test","expected_revision":0}),
                },
            )
            .unwrap();
        assert_eq!(scanned["review_defaults_active"], false);
        let resumed = host
            .execute(
                "test-token",
                HttpOperation::ReadOnboarding {
                    id: "recovery-session".into(),
                    domain: "test".into(),
                },
            )
            .unwrap();
        assert_eq!(resumed["state"], "scanning");
        assert_eq!(resumed["review_defaults_active"], false);
        let mut recovered = {
            let mut storage = host.storage.lock().unwrap();
            let mut recovered = storage
                .load_onboarding_session(
                    repository.security_domain(),
                    "recovery-session",
                    &principal,
                    &current,
                )
                .unwrap();
            let component = ComponentProposal {
                id: "recovered-api".into(),
                name: "Recovered API".into(),
                role: "backend".into(),
                repository_index: 0,
                roots: vec!["src".into()],
                stack_candidates: vec![],
                signals: vec![],
                evidence: vec![evidence("src/lib.rs")],
                confidence: crate::EvidenceConfidence::Candidate,
                contradictions: vec![],
                decision: ComponentDecision::Pending,
            };
            recovered
                .set_proposal(
                    1,
                    vec![component.clone()],
                    vec![],
                    vec![],
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&recovered, Some(1), 10, &principal, &current)
                .unwrap();
            recovered
                .transition(
                    2,
                    OnboardingState::AwaitingConfirmation,
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&recovered, Some(2), 11, &principal, &current)
                .unwrap();
            let mut component = component;
            component.decision = ComponentDecision::Confirmed;
            recovered
                .decide_component(3, component, &principal, &current)
                .unwrap();
            storage
                .save_onboarding_session(&recovered, Some(3), 12, &principal, &current)
                .unwrap();
            recovered
        };
        let confirmed = host.execute("test-token", HttpOperation::MutateOnboarding { id:recovered.id().into(), action:"confirm".into(), request:json!({"security_domain":"test","expected_revision":recovered.revision(),"proposal_fingerprint":recovered.proposal_fingerprint(),"known_gaps_accepted":true,"activate_defaults":false}) }).unwrap();
        assert_eq!(confirmed["review_defaults_active"], false);
        {
            let storage = host.storage.lock().unwrap();
            assert!(
                storage
                    .load_onboarding_defaults(&repository, &viewer, &narrow)
                    .is_err()
            );
            assert_eq!(
                storage
                    .connection
                    .query_row("SELECT context_id FROM onboarding_defaults", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap(),
                context_id
            );
            assert_eq!(
                storage
                    .connection
                    .query_row(
                        "SELECT payload FROM onboarding_accepted_contexts WHERE context_id=?1",
                        [&context_id],
                        |row| row.get::<_, Vec<u8>>(0)
                    )
                    .unwrap(),
                invalid_payload
            );
            recovered = storage
                .load_onboarding_session(
                    repository.security_domain(),
                    "recovery-session",
                    &principal,
                    &current,
                )
                .unwrap();
        }
        let activated = host.execute("test-token", HttpOperation::MutateOnboarding { id:recovered.id().into(), action:"activate".into(), request:json!({"security_domain":"test","expected_revision":recovered.revision(),"proposal_fingerprint":recovered.proposal_fingerprint()}) }).unwrap();
        assert_eq!(activated["review_defaults_active"], true);
        {
            let storage = host.storage.lock().unwrap();
            assert_eq!(
                storage
                    .load_onboarding_defaults(&repository, &viewer, &narrow)
                    .unwrap()["session_id"],
                "recovery-session"
            );
            assert_eq!(
                storage
                    .connection
                    .query_row(
                        "SELECT payload FROM onboarding_accepted_contexts WHERE context_id=?1",
                        [&context_id],
                        |row| row.get::<_, Vec<u8>>(0)
                    )
                    .unwrap(),
                invalid_payload
            );
            assert_eq!(storage.connection.query_row("SELECT payload FROM onboarding_history WHERE session_id='metadata-session' ORDER BY revision DESC LIMIT 1", [], |row|row.get::<_, Vec<u8>>(0)).unwrap(), history);
        }
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mutation_contracts_preserve_provenance_and_resume_explicit_defaults() {
        let (host, grants, count, root) = setup(true);
        let principal = PrincipalId::new("maintainer").unwrap();
        let current = grants.lock().unwrap().clone();
        let scope = ScopeRecord {
            version: ScopeVersion::new("s1").unwrap(),
            primary_repository: current[0].repository.clone(),
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let mut session = OnboardingSession::new(
            "session-1".into(),
            principal.clone(),
            SnapshotId::new("a".repeat(40)).unwrap(),
            scope,
            "v1".into(),
            &current,
        )
        .unwrap();
        let mut storage = host.storage.lock().unwrap();
        storage
            .save_onboarding_session(&session, None, 0, &principal, &current)
            .unwrap();
        session
            .transition(0, OnboardingState::Scanning, &principal, &current)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(0), 1, &principal, &current)
            .unwrap();
        let component = ComponentProposal {
            id: "api".into(),
            name: "API".into(),
            role: "backend".into(),
            repository_index: 0,
            roots: vec!["src".into()],
            stack_candidates: vec![],
            signals: vec![crate::ComponentSignal {
                kind: "database".into(),
                name: "SQLx".into(),
                reason: "Observed import; schema unknown".into(),
                evidence_index: 0,
                test_only: false,
            }],
            evidence: vec![crate::OnboardingEvidence {
                repository_index: 0,
                path: "src/lib.rs".into(),
                blob: "b".repeat(40),
                start_line: 1,
                end_line: 3,
                producer_version: "fixture-v1".into(),
            }],
            confidence: crate::EvidenceConfidence::Candidate,
            contradictions: vec![],
            decision: ComponentDecision::Pending,
        };
        let provider = crate::ComponentProviderMapping {
            component_id: "api".into(),
            provider: "Vercel".into(),
            project_id: None,
            service_id: None,
            environment: "unknown".into(),
            repository_index: 0,
            source_root: "src".into(),
            observed_commit: Some("a".repeat(40)),
            verified_at_ms: None,
            verification: crate::ProviderVerification::NotRequested,
            evidence: component.evidence.clone(),
            notes: vec![],
            decision: ComponentDecision::Pending,
        };
        session
            .set_proposal(
                1,
                vec![component.clone()],
                vec![provider.clone()],
                vec![],
                &principal,
                &current,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(1), 2, &principal, &current)
            .unwrap();
        drop(storage);
        let call = |action: &str, request: Value| {
            host.execute(
                "test-token",
                HttpOperation::MutateOnboarding {
                    id: "session-1".into(),
                    action: action.into(),
                    request,
                },
            )
        };
        let inspected = call(
            "inspect",
            json!({"security_domain":"test","expected_revision":2}),
        )
        .unwrap();
        assert_eq!(inspected["questions"].as_array().unwrap().len(), 3);
        assert_eq!(inspected["confirmation_readiness"]["components_pending"], 1);
        let mut component = component;
        component.decision = ComponentDecision::Confirmed;
        let decided = call(
            "decisions",
            json!({"security_domain":"test","expected_revision":3,"component":component}),
        )
        .unwrap();
        assert_eq!(decided["confirmation_readiness"]["providers_pending"], 1);
        let mut provider = provider;
        provider.decision = ComponentDecision::Corrected;
        provider.environment = "preview".into();
        let mut forged = provider.clone();
        forged.verified_at_ms = Some(1);
        assert_eq!(
            call(
                "provider-decisions",
                json!({"security_domain":"test","expected_revision":4,"provider_index":0,"provider":forged})
            ),
            Err(HttpFailure::Malformed)
        );
        assert_eq!(
            call(
                "provider-decisions",
                json!({"security_domain":"test","expected_revision":3,"provider_index":0,"provider":provider})
            ),
            Err(HttpFailure::Conflict)
        );
        call("provider-decisions",json!({"security_domain":"test","expected_revision":4,"provider_index":0,"provider":provider})).unwrap();
        let id = inspected["integrations"][0]["id"].as_str().unwrap();
        let ready=call("integration-decisions",json!({"security_domain":"test","expected_revision":5,"integration_id":id,"decision":"exclude"})).unwrap();
        assert_eq!(ready["confirmation_readiness"]["ready"], true);
        assert_eq!(ready["accepted_context"]["usable"], true);
        assert_eq!(
            call(
                "confirm",
                json!({"security_domain":"test","expected_revision":6,"proposal_fingerprint":ready["proposal_fingerprint"],"known_gaps_accepted":false,"activate_defaults":true})
            ),
            Err(HttpFailure::Malformed)
        );
        let accepted=call("confirm",json!({"security_domain":"test","expected_revision":6,"proposal_fingerprint":ready["proposal_fingerprint"],"known_gaps_accepted":true,"activate_defaults":true})).unwrap();
        assert_eq!(accepted["review_defaults_active"], true);
        let resumed = host
            .execute(
                "test-token",
                HttpOperation::ReadOnboarding {
                    id: "session-1".into(),
                    domain: "test".into(),
                },
            )
            .unwrap();
        assert_eq!(resumed["review_defaults_active"], true);
        let active=call("defaults",json!({"security_domain":"test","expected_revision":0,"repository":{"security_domain":"test","provider":"github","owner":"owner","name":"repo"}})).unwrap();
        assert_eq!(active["session_id"], "session-1");
        assert_eq!(active["integrations"], json!([]));
        assert_eq!(active["providers"][0]["verification"], "not_requested");
        let mut viewer_grant = current[0].clone();
        viewer_grant.principal = PrincipalId::new("viewer").unwrap();
        viewer_grant.actions = BTreeSet::from([Action::Index]);
        grants.lock().unwrap().push(viewer_grant);
        let drift = |revision| {
            host.execute(
                "viewer-token",
                HttpOperation::MutateOnboarding {
                    id: "session-1".into(),
                    action: "drift".into(),
                    request: json!({"security_domain":"test","expected_revision":revision,"reference":"new-head","repository":{"security_domain":"test","provider":"github","owner":"owner","name":"repo"}}),
                },
            )
        };
        let reads_before = count.load(Ordering::SeqCst);
        assert_eq!(drift(6), Err(HttpFailure::Conflict));
        assert_eq!(count.load(Ordering::SeqCst), reads_before);
        let assessed = drift(7).unwrap();
        assert_eq!(assessed["source_commit"], "a".repeat(40));
        assert_eq!(assessed["target_commit"], "c".repeat(40));
        assert_eq!(assessed["defaults_changed"], false);
        assert_eq!(assessed["scope_expanded"], false);
        assert_eq!(assessed["session_revision"], 7);
        assert_eq!(
            host.execute(
                "viewer-token",
                HttpOperation::ReadOnboarding {
                    id: "session-1".into(),
                    domain: "test".into()
                }
            ),
            Err(HttpFailure::DeniedOrMissing)
        );
        assert_eq!(
            call("defaults",json!({"security_domain":"test","expected_revision":0,"repository":{"security_domain":"test","provider":"github","owner":"owner","name":"repo"}})).unwrap(),
            active
        );
        assert_eq!(
            host.execute(
                "test-token",
                HttpOperation::ReadOnboarding {
                    id: "session-1".into(),
                    domain: "test".into()
                }
            )
            .unwrap(),
            resumed
        );
        grants.lock().unwrap().clear();
        let reads_before = count.load(Ordering::SeqCst);
        assert_eq!(drift(7), Err(HttpFailure::DeniedOrMissing));
        assert_eq!(count.load(Ordering::SeqCst), reads_before);
        assert_eq!(
            call(
                "defaults",
                json!({"security_domain":"test","expected_revision":0,"repository":{"security_domain":"test","provider":"github","owner":"owner","name":"repo"}})
            ),
            Err(HttpFailure::DeniedOrMissing)
        );
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }
    async fn call(
        host: Arc<dyn HttpBackend>,
        method: &str,
        url: &str,
        body: Option<Value>,
        token: &str,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(url)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                body.map(|value| value.to_string()).unwrap_or_default(),
            ))
            .unwrap();
        let response = router(host).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    #[tokio::test]
    async fn browser_routes_authenticate_pin_resume_and_reject_stale_writes() {
        let (host, grants, count, root) = setup(true);
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding",
                Some(create()),
                "bad"
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let (status, created) = call(
            host.clone(),
            "POST",
            "/v1/onboarding",
            Some(create()),
            "test-token",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(created["commit"], "a".repeat(40));
        assert_eq!(created["review_defaults_active"], false);
        let read = call(
            host.clone(),
            "GET",
            "/v1/onboarding/session-1?security_domain=test",
            None,
            "test-token",
        )
        .await;
        assert_eq!(read.1, created);
        let mutation = json!({"security_domain":"test","expected_revision":0});
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding/session-1/cancel",
                Some(mutation.clone()),
                "test-token"
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding/session-1/cancel",
                Some(mutation),
                "test-token"
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        grants.lock().unwrap().clear();
        assert_eq!(
            call(
                host.clone(),
                "GET",
                "/v1/onboarding/session-1?security_domain=test",
                None,
                "test-token"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding",
                Some(create()),
                "test-token"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn confirmation_requires_explicit_maintainer_grant_and_current_fingerprint() {
        let (host, grants, _, root) = setup(false);
        let principal = PrincipalId::new("maintainer").unwrap();
        let current = grants.lock().unwrap().clone();
        let scope = ScopeRecord {
            version: ScopeVersion::new("s1").unwrap(),
            primary_repository: current[0].repository.clone(),
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let mut session = OnboardingSession::new(
            "session-1".into(),
            principal.clone(),
            SnapshotId::new("a".repeat(40)).unwrap(),
            scope,
            "v1".into(),
            &current,
        )
        .unwrap();
        {
            let mut storage = host.storage.lock().unwrap();
            storage
                .save_onboarding_session(&session, None, 1, &principal, &current)
                .unwrap();
            session
                .transition(0, OnboardingState::Scanning, &principal, &current)
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(0), 2, &principal, &current)
                .unwrap();
            session
                .set_proposal(
                    1,
                    vec![],
                    vec![],
                    vec!["No supported manifest".into()],
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(1), 3, &principal, &current)
                .unwrap();
            session
                .transition(
                    2,
                    OnboardingState::AwaitingConfirmation,
                    &principal,
                    &current,
                )
                .unwrap();
            storage
                .save_onboarding_session(&session, Some(2), 4, &principal, &current)
                .unwrap();
        }
        let confirmation = json!({"security_domain":"test","expected_revision":3,"proposal_fingerprint":session.proposal_fingerprint(),"known_gaps_accepted":true});
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding/session-1/confirm",
                Some(confirmation.clone()),
                "test-token"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        grants.lock().unwrap()[0]
            .actions
            .insert(Action::ConfirmArchitecture);
        let mut stale = confirmation.clone();
        stale["proposal_fingerprint"] = json!("old");
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding/session-1/confirm",
                Some(stale),
                "test-token"
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        let accepted = call(
            host.clone(),
            "POST",
            "/v1/onboarding/session-1/confirm",
            Some(confirmation.clone()),
            "test-token",
        )
        .await;
        assert_eq!(accepted.0, StatusCode::OK);
        assert_eq!(accepted.1["state"], "confirmed");
        assert_eq!(
            call(
                host.clone(),
                "POST",
                "/v1/onboarding/session-1/confirm",
                Some(confirmation),
                "test-token"
            )
            .await
            .1,
            accepted.1
        );
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }
}
