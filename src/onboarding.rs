//! Versioned onboarding proposals. Confirmation does not activate review defaults,
//! authorize provider access, or dispatch a model request.
use crate::{Action, Grant, PathScope, PrincipalId, ScopeRecord, SnapshotId, authorize_repository};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[cfg(feature = "server")]
pub(crate) mod drift;
#[cfg(feature = "server")]
pub(crate) mod providers;
mod questions;
pub use questions::{IntegrationCandidate, IntegrationDecision, OnboardingQuestion};

pub const ONBOARDING_VERSION: &str = "csb-onboarding-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(rename_all = "snake_case"))]
pub enum OnboardingState {
    Draft,
    Scanning,
    ProposalReady,
    AwaitingConfirmation,
    Confirmed,
    Partial,
    Failed,
    Cancelled,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(rename_all = "snake_case"))]
pub enum ComponentDecision {
    Pending,
    Confirmed,
    Corrected,
    Rejected,
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(rename_all = "snake_case"))]
pub enum EvidenceConfidence {
    Observed,
    Corroborated,
    Candidate,
    UserSupplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(rename_all = "snake_case"))]
pub enum ProviderVerification {
    NotRequested,
    Verified,
    Mismatch,
    Failed,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct OnboardingEvidence {
    /// Zero is the primary repo; N+1 is the Nth exact context pin.
    pub repository_index: usize,
    pub path: String,
    pub blob: String,
    pub start_line: u32,
    pub end_line: u32,
    pub producer_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct ComponentProposal {
    pub id: String,
    pub name: String,
    pub role: String,
    pub repository_index: usize,
    pub roots: Vec<String>,
    pub stack_candidates: Vec<String>,
    #[cfg_attr(feature = "server", serde(default))]
    pub signals: Vec<ComponentSignal>,
    pub evidence: Vec<OnboardingEvidence>,
    pub confidence: EvidenceConfidence,
    pub contradictions: Vec<String>,
    pub decision: ComponentDecision,
}

/// Qualified observations, not resolved service edges or runtime proof.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct ComponentSignal {
    pub kind: String,
    pub name: String,
    pub reason: String,
    pub evidence_index: usize,
    pub test_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct ComponentProviderMapping {
    pub component_id: String,
    pub provider: String,
    pub project_id: Option<String>,
    pub service_id: Option<String>,
    pub environment: String,
    pub repository_index: usize,
    pub source_root: String,
    pub observed_commit: Option<String>,
    pub verified_at_ms: Option<i64>,
    pub verification: ProviderVerification,
    #[cfg_attr(feature = "server", serde(default))]
    pub evidence: Vec<OnboardingEvidence>,
    #[cfg_attr(feature = "server", serde(default))]
    pub notes: Vec<String>,
    #[cfg_attr(feature = "server", serde(default = "pending_decision"))]
    pub decision: ComponentDecision,
}

#[cfg(feature = "server")]
fn pending_decision() -> ComponentDecision {
    ComponentDecision::Pending
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
pub struct OnboardingConfirmation {
    pub actor: String,
    pub confirmed_at_ms: i64,
    pub input_fingerprint: String,
    pub proposal_fingerprint: String,
    pub known_gaps_accepted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnboardingSession {
    id: String,
    principal: PrincipalId,
    commit: SnapshotId,
    scope: ScopeRecord,
    scan_version: String,
    revision: u64,
    state: OnboardingState,
    proposals: Vec<ComponentProposal>,
    provider_mappings: Vec<ComponentProviderMapping>,
    integrations: Vec<IntegrationCandidate>,
    question_version: Option<String>,
    gaps: Vec<String>,
    confirmation: Option<OnboardingConfirmation>,
    inventory: Option<crate::OnboardingInventory>,
}

fn label(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err("Onboarding label must contain 1–256 printable characters".into())
    } else {
        Ok(())
    }
}

pub(crate) fn scope_is_covered(requested: &PathScope, granted: &PathScope) -> bool {
    let under = |path: &str, root: &str| {
        root == "."
            || path == root
            || path
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    requested
        .includes()
        .all(|root| granted.includes().any(|allowed| under(root, allowed)))
        && granted.excludes().all(|denied| {
            !requested
                .includes()
                .any(|root| under(denied, root) || under(root, denied))
                || requested.excludes().any(|excluded| under(denied, excluded))
        })
}

impl OnboardingSession {
    pub fn new(
        id: String,
        principal: PrincipalId,
        commit: SnapshotId,
        mut scope: ScopeRecord,
        scan_version: String,
        grants: &[Grant],
    ) -> Result<Self, String> {
        label(&id)?;
        label(&scan_version)?;
        if id.len() > 128 {
            return Err("Onboarding ID exceeds 128 characters".into());
        }
        for contract in &scope.contracts {
            label(contract)?;
        }
        scope.contexts.sort_by(|left, right| {
            (&left.name, &left.repository).cmp(&(&right.name, &right.repository))
        });
        if scope.contexts.len() > 20
            || scope.contracts.len() > 100
            || !scope.model_destinations.is_empty()
        {
            return Err(
                "Onboarding requires bounded context and inactive model destinations".into(),
            );
        }
        let mut names = BTreeSet::new();
        for context in &scope.contexts {
            label(&context.name)?;
            if !names.insert(&context.name)
                || context.repository.security_domain()
                    != scope.primary_repository.security_domain()
            {
                return Err("Context names must be unique within the security domain".into());
            }
        }
        let session = Self {
            id,
            principal,
            commit,
            scope,
            scan_version,
            revision: 0,
            state: OnboardingState::Draft,
            proposals: vec![],
            provider_mappings: vec![],
            integrations: vec![],
            question_version: None,
            gaps: vec![],
            confirmation: None,
            inventory: None,
        };
        session.check_access(&session.principal, grants)?;
        Ok(session)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn commit(&self) -> &SnapshotId {
        &self.commit
    }
    pub fn gaps(&self) -> &[String] {
        &self.gaps
    }
    pub fn scan_version(&self) -> &str {
        &self.scan_version
    }
    pub fn inventory(&self) -> Option<&crate::OnboardingInventory> {
        self.inventory.as_ref()
    }
    pub fn record_inventory(
        &mut self,
        expected: u64,
        inventory: crate::OnboardingInventory,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        self.check_access(principal, grants)?;
        self.check_revision(expected)?;
        if !matches!(
            self.state,
            OnboardingState::Draft | OnboardingState::Scanning | OnboardingState::Partial
        ) || self.inventory.as_ref().is_some_and(|item| item.complete)
        {
            return Err("Invalid scan state".into());
        }
        inventory.validate(self)?;
        self.state = if inventory.complete {
            OnboardingState::Scanning
        } else {
            OnboardingState::Partial
        };
        self.inventory = Some(inventory);
        self.revision += 1;
        Ok(())
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn state(&self) -> OnboardingState {
        self.state
    }
    pub fn scope(&self) -> &ScopeRecord {
        &self.scope
    }
    pub fn proposals(&self) -> &[ComponentProposal] {
        &self.proposals
    }
    pub fn provider_mappings(&self) -> &[ComponentProviderMapping] {
        &self.provider_mappings
    }
    pub fn integrations(&self) -> &[IntegrationCandidate] {
        &self.integrations
    }
    pub fn questions(&self) -> Vec<OnboardingQuestion> {
        questions::plan(self)
    }
    #[cfg(feature = "server")]
    fn validate_integrations(&self) -> Result<(), String> {
        if self.question_version.is_none() {
            if !self.integrations.is_empty() {
                return Err("Invalid legacy integration state".into());
            }
            return Ok(());
        }
        if self.question_version.as_deref() != Some(questions::QUESTION_VERSION) {
            return Err("Unknown question planner version".into());
        }
        let (expected, _) = questions::integrations(self);
        if expected.len() != self.integrations.len() {
            return Err("Stored integrations do not match observed evidence".into());
        }
        for (observed, stored) in expected.iter().zip(&self.integrations) {
            let mut unchanged = stored.clone();
            unchanged.decision = IntegrationDecision::Pending;
            unchanged.answer = None;
            if &unchanged != observed {
                return Err("Stored integration evidence was changed".into());
            }
            if let Some(answer) = &stored.answer {
                label(answer)?;
            }
        }
        Ok(())
    }
    pub fn pending_decisions(&self) -> (usize, usize, usize) {
        (
            self.proposals
                .iter()
                .filter(|item| item.decision == ComponentDecision::Pending)
                .count(),
            self.provider_mappings
                .iter()
                .filter(|item| {
                    self.component_is_included(&item.component_id)
                        && item.decision == ComponentDecision::Pending
                })
                .count(),
            self.integrations
                .iter()
                .filter(|item| {
                    self.component_is_included(&item.component_id)
                        && item.decision == IntegrationDecision::Pending
                })
                .count(),
        )
    }
    pub fn known_gaps_required(&self) -> bool {
        !self.gaps.is_empty()
            || !self.scope.contracts.is_empty()
            || self.proposals.iter().any(|item| {
                item.decision == ComponentDecision::Deferred || !item.contradictions.is_empty()
            })
            || self.provider_mappings.iter().any(|item| {
                self.component_is_included(&item.component_id)
                    && (!item.notes.is_empty()
                        || item.verification != ProviderVerification::Verified)
            })
            || self.integrations.iter().any(|item| {
                self.component_is_included(&item.component_id)
                    && matches!(
                        item.decision,
                        IntegrationDecision::Include | IntegrationDecision::Defer
                    )
            })
    }
    pub fn confirmation(&self) -> Option<&OnboardingConfirmation> {
        self.confirmation.as_ref()
    }

    pub fn input_fingerprint(&self) -> String {
        let mut digest = Sha256::new();
        for bytes in [
            ONBOARDING_VERSION.as_bytes(),
            self.principal.as_str().as_bytes(),
            self.scope.version.as_str().as_bytes(),
            self.commit.as_str().as_bytes(),
            self.scan_version.as_bytes(),
            &self.scope.canonical_bytes(),
        ] {
            digest.update((bytes.len() as u64).to_be_bytes());
            digest.update(bytes);
        }
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn check_access(&self, principal: &PrincipalId, grants: &[Grant]) -> Result<(), String> {
        if principal != &self.principal {
            return Err("Onboarding session access denied".into());
        }
        for index in 0..=self.scope.contexts.len() {
            let (repository, paths) = if index == 0 {
                (&self.scope.primary_repository, &self.scope.primary_paths)
            } else {
                let context = &self.scope.contexts[index - 1];
                (&context.repository, &context.paths)
            };
            if paths.includes().count() > 100 || paths.excludes().count() > 100 {
                return Err("Scan path scope exceeds its bounds".into());
            }
            for path in paths.includes().chain(paths.excludes()) {
                if path.chars().any(char::is_control) {
                    return Err("Invalid scope path".into());
                }
            }
            authorize_repository(Action::Index, principal, repository, grants, None, paths)?;
            if !grants.iter().any(|grant| {
                grant.principal == *principal
                    && grant.repository == *repository
                    && grant.actions.contains(&Action::Index)
                    && scope_is_covered(paths, &grant.paths)
            }) {
                return Err("Initial scan scope exceeds the current grant".into());
            }
        }
        Ok(())
    }

    fn check_revision(&self, expected: u64) -> Result<(), String> {
        if self.revision != expected {
            Err("Stale onboarding revision; reload the proposal".into())
        } else {
            Ok(())
        }
    }

    fn selected_paths(&self, index: usize) -> Result<&PathScope, String> {
        if index == 0 {
            Ok(&self.scope.primary_paths)
        } else {
            self.scope
                .contexts
                .get(index - 1)
                .map(|pin| &pin.paths)
                .ok_or("Unknown context repository".into())
        }
    }

    fn validate_path(&self, index: usize, path: &str) -> Result<(), String> {
        PathScope::new(vec![path.to_owned()], vec![])?;
        if path.chars().any(char::is_control)
            || crate::onboarding_scan::sensitive_path(path)
            || !self.selected_paths(index)?.allows(path)
        {
            return Err("Proposal evidence or root is outside the authorized scope".into());
        }
        Ok(())
    }

    fn validate_component_roots(
        &self,
        component: &ComponentProposal,
        changed_roots: bool,
    ) -> Result<(), String> {
        let Some(inventory) = &self.inventory else {
            if changed_roots {
                return Err("Corrected roots require a completed pinned inventory".into());
            }
            // Older model-only records may retain their originally observed boundary;
            // they cannot introduce a new root without an inventory.
            return Ok(());
        };
        if !inventory.complete {
            return Err("Component roots require a completed pinned inventory".into());
        }
        inventory.validate(self)?;
        for root in &component.roots {
            self.validate_path(component.repository_index, root)?;
            let source_under_root = |path: &str| {
                root == "."
                    || path == root
                    || path
                        .strip_prefix(root)
                        .is_some_and(|rest| rest.starts_with('/'))
            };
            if !inventory.items.iter().any(|item| {
                item.repository_index == component.repository_index
                    && item.status == "indexed"
                    && !crate::onboarding_scan::sensitive_path(&item.path)
                    && ((item.kind == "source" && source_under_root(&item.path))
                        || (item.kind == "manifest"
                            && item.path.rsplit_once('/').map_or(".", |(parent, _)| parent)
                                == root))
            }) {
                return Err("Unknown component root; choose an indexed source path or explicit manifest root from the pinned inventory".into());
            }
        }
        Ok(())
    }

    fn validate_proposals(
        &self,
        proposals: &[ComponentProposal],
        mappings: &[ComponentProviderMapping],
        gaps: &[String],
    ) -> Result<(), String> {
        if proposals.len() > 100 || mappings.len() > 200 || gaps.len() > 100 {
            return Err("Onboarding proposal exceeds the bounded payload".into());
        }
        let mut ids = BTreeSet::new();
        for component in proposals {
            for value in [&component.id, &component.name, &component.role] {
                label(value)?;
            }
            if !ids.insert(&component.id)
                || component.roots.is_empty()
                || component.roots.len() > 20
                || component.evidence.len() > 100
                || component.stack_candidates.len() > 20
                || component.contradictions.len() > 20
                || component.signals.len() > 64
            {
                return Err("Invalid component identity or bounds".into());
            }
            for root in &component.roots {
                self.validate_path(component.repository_index, root)?;
            }
            for value in component
                .stack_candidates
                .iter()
                .chain(&component.contradictions)
            {
                label(value)?;
            }
            if component.evidence.is_empty()
                && component.confidence != EvidenceConfidence::UserSupplied
            {
                return Err("Detected component requires source evidence".into());
            }
            for evidence in &component.evidence {
                self.validate_path(evidence.repository_index, &evidence.path)?;
                SnapshotId::new(evidence.blob.clone())?;
                label(&evidence.producer_version)?;
                if evidence.start_line == 0 || evidence.end_line < evidence.start_line {
                    return Err("Invalid evidence span".into());
                }
            }
            for signal in &component.signals {
                for value in [&signal.kind, &signal.name, &signal.reason] {
                    label(value)?;
                }
                if signal.evidence_index >= component.evidence.len() {
                    return Err("Invalid component signal evidence reference".into());
                }
            }
        }
        for mapping in mappings {
            label(&mapping.provider)?;
            label(&mapping.environment)?;
            let component = proposals
                .iter()
                .find(|item| item.id == mapping.component_id)
                .ok_or("Provider mapping has no component")?;
            if mapping.repository_index != component.repository_index {
                return Err("Provider mapping repository differs from component".into());
            }
            self.validate_path(mapping.repository_index, &mapping.source_root)?;
            if mapping.evidence.len() > 16 || mapping.notes.len() > 16 {
                return Err("Provider proposal exceeds evidence bounds".into());
            }
            for note in &mapping.notes {
                label(note)?;
            }
            for evidence in &mapping.evidence {
                self.validate_path(evidence.repository_index, &evidence.path)?;
                SnapshotId::new(evidence.blob.clone())?;
                label(&evidence.producer_version)?;
                if evidence.repository_index != mapping.repository_index
                    || evidence.start_line == 0
                    || evidence.end_line < evidence.start_line
                {
                    return Err("Invalid provider evidence span or repository".into());
                }
            }
            for value in mapping.project_id.iter().chain(&mapping.service_id) {
                label(value)?;
            }
            if let Some(commit) = &mapping.observed_commit {
                SnapshotId::new(commit.clone())?;
            }
            if mapping.verification == ProviderVerification::Verified
                && (mapping.project_id.is_none()
                    || mapping.verified_at_ms.is_none_or(|time| time < 0)
                    || mapping.observed_commit.is_none())
            {
                return Err("Verified mapping requires provider observation provenance".into());
            }
        }
        for gap in gaps {
            label(gap)?;
        }
        Ok(())
    }

    pub fn transition(
        &mut self,
        expected: u64,
        next: OnboardingState,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        self.check_access(principal, grants)?;
        self.check_revision(expected)?;
        let allowed = matches!(
            (self.state, next),
            (
                OnboardingState::Draft,
                OnboardingState::Scanning
                    | OnboardingState::Cancelled
                    | OnboardingState::Superseded
            ) | (
                OnboardingState::Scanning,
                OnboardingState::Partial
                    | OnboardingState::Failed
                    | OnboardingState::Cancelled
                    | OnboardingState::Superseded
            ) | (
                OnboardingState::Partial,
                OnboardingState::Scanning
                    | OnboardingState::Cancelled
                    | OnboardingState::Superseded
            ) | (
                OnboardingState::ProposalReady,
                OnboardingState::AwaitingConfirmation
                    | OnboardingState::Cancelled
                    | OnboardingState::Superseded
            ) | (
                OnboardingState::AwaitingConfirmation,
                OnboardingState::Cancelled | OnboardingState::Superseded
            )
        );
        if !allowed {
            return Err("Invalid onboarding transition".into());
        }
        self.state = next;
        self.revision += 1;
        Ok(())
    }

    pub fn set_proposal(
        &mut self,
        expected: u64,
        proposals: Vec<ComponentProposal>,
        mappings: Vec<ComponentProviderMapping>,
        gaps: Vec<String>,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        self.check_access(principal, grants)?;
        self.check_revision(expected)?;
        if !matches!(
            self.state,
            OnboardingState::Scanning | OnboardingState::Partial
        ) || proposals
            .iter()
            .any(|item| item.decision != ComponentDecision::Pending)
            || mappings.iter().any(|item| {
                item.verification != ProviderVerification::NotRequested
                    || item.decision != ComponentDecision::Pending
            })
        {
            return Err("Scanner proposals must start unconfirmed and provider-unverified".into());
        }
        self.validate_proposals(&proposals, &mappings, &gaps)?;
        self.proposals = proposals;
        self.provider_mappings = mappings;
        self.gaps = gaps;
        let (integrations, truncated) = questions::integrations(self);
        self.integrations = integrations;
        self.question_version = Some(questions::QUESTION_VERSION.into());
        if truncated {
            if self.gaps.len() == 100 {
                self.gaps.pop();
            }
            self.gaps.push("Additional integration candidates exceed the 200-question budget and remain excluded; narrow scope to inspect them".into());
        }
        self.state = OnboardingState::ProposalReady;
        self.revision += 1;
        Ok(())
    }

    pub fn decide_component(
        &mut self,
        expected: u64,
        replacement: ComponentProposal,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        self.check_access(principal, grants)?;
        self.check_revision(expected)?;
        if self.state != OnboardingState::AwaitingConfirmation {
            return Err("Component decisions require an awaiting-confirmation session".into());
        }
        let index = self
            .proposals
            .iter()
            .position(|item| item.id == replacement.id)
            .ok_or("Unknown component")?;
        let old = &self.proposals[index];
        if replacement.decision == ComponentDecision::Pending
            || (replacement.decision == ComponentDecision::Confirmed
                && (replacement.name != old.name
                    || replacement.role != old.role
                    || replacement.roots != old.roots))
        {
            return Err("Component changes require a corrected decision".into());
        }
        if replacement.evidence != old.evidence
            || replacement.signals != old.signals
            || replacement.stack_candidates != old.stack_candidates
            || replacement.contradictions != old.contradictions
            || replacement.confidence != old.confidence
            || replacement.repository_index != old.repository_index
        {
            return Err("Component corrections cannot change observed evidence".into());
        }
        if matches!(
            replacement.decision,
            ComponentDecision::Confirmed | ComponentDecision::Corrected
        ) || replacement.roots != old.roots
        {
            self.validate_component_roots(&replacement, replacement.roots != old.roots)?;
        }
        let mut proposals = self.proposals.clone();
        proposals[index] = replacement;
        self.validate_proposals(&proposals, &self.provider_mappings, &self.gaps)?;
        self.proposals = proposals;
        self.revision += 1;
        Ok(())
    }

    /// User interpretation may correct hosting labels, never synthesize provider observations.
    pub fn decide_provider(
        &mut self,
        expected: u64,
        index: usize,
        replacement: ComponentProviderMapping,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        self.check_access(principal, grants)?;
        self.check_revision(expected)?;
        if self.state != OnboardingState::AwaitingConfirmation {
            return Err("Provider decisions require an awaiting-confirmation session".into());
        }
        let old = self
            .provider_mappings
            .get(index)
            .ok_or("Unknown provider mapping")?;
        if replacement.component_id != old.component_id
            || replacement.repository_index != old.repository_index
            || replacement.evidence != old.evidence
            || replacement.notes != old.notes
            || replacement.observed_commit != old.observed_commit
            || replacement.verification != old.verification
            || replacement.verified_at_ms != old.verified_at_ms
            || replacement.project_id != old.project_id
            || replacement.service_id != old.service_id
            || replacement.decision == ComponentDecision::Pending
        {
            return Err(
                "Provider corrections cannot change observed evidence or verification".into(),
            );
        }
        if replacement.decision == ComponentDecision::Confirmed
            && (replacement.provider != old.provider
                || replacement.environment != old.environment
                || replacement.source_root != old.source_root)
        {
            return Err("Provider changes require a corrected decision".into());
        }
        let mut mappings = self.provider_mappings.clone();
        mappings[index] = replacement;
        self.validate_proposals(&self.proposals, &mappings, &self.gaps)?;
        self.provider_mappings = mappings;
        self.revision += 1;
        Ok(())
    }

    pub fn decide_integration(
        &mut self,
        expected: u64,
        id: &str,
        decision: IntegrationDecision,
        answer: Option<String>,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        self.check_access(principal, grants)?;
        self.check_revision(expected)?;
        if self.state != OnboardingState::AwaitingConfirmation
            || decision == IntegrationDecision::Pending
        {
            return Err("Integration decisions require an awaiting-confirmation session and an explicit answer".into());
        }
        if let Some(value) = &answer {
            label(value)?;
        }
        let candidate = self
            .integrations
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or("Unknown integration")?;
        candidate.decision = decision;
        candidate.answer = answer;
        self.revision += 1;
        Ok(())
    }

    fn component_is_included(&self, id: &str) -> bool {
        self.proposals.iter().any(|item| {
            item.id == id
                && matches!(
                    item.decision,
                    ComponentDecision::Confirmed | ComponentDecision::Corrected
                )
        })
    }

    /// Exact, narrowed scope for a later review. There is no default activation here.
    /// Rejected/deferred roots take precedence over overlapping accepted roots.
    pub fn proposed_scope(
        &self,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<Option<ScopeRecord>, String> {
        self.check_access(principal, grants)?;
        for component in self
            .proposals
            .iter()
            .filter(|item| self.component_is_included(&item.id))
        {
            self.validate_component_roots(component, false)?;
        }
        let paths = |index: usize| -> Result<Option<PathScope>, String> {
            let selected = self.selected_paths(index)?;
            let includes = self
                .proposals
                .iter()
                .filter(|item| {
                    item.repository_index == index && self.component_is_included(&item.id)
                })
                .flat_map(|item| item.roots.clone())
                .collect::<Vec<_>>();
            if includes.is_empty() {
                return Ok(None);
            }
            let mut excludes = selected.excludes().map(str::to_owned).collect::<Vec<_>>();
            excludes.extend(
                self.proposals
                    .iter()
                    .filter(|item| {
                        item.repository_index == index && !self.component_is_included(&item.id)
                    })
                    .flat_map(|item| item.roots.clone()),
            );
            let narrowed = PathScope::new(includes, excludes)?;
            if narrowed.includes().all(|root| !narrowed.allows(root)) {
                return Ok(None);
            }
            if !scope_is_covered(&narrowed, selected) {
                return Err("Accepted context must not expand the scanned scope".into());
            }
            Ok(Some(narrowed))
        };
        let Some(primary_paths) = paths(0)? else {
            return Ok(None);
        };
        let mut scope = self.scope.clone();
        scope.primary_paths = primary_paths;
        scope.contexts.clear();
        for (index, context) in self.scope.contexts.iter().enumerate() {
            if let Some(paths) = paths(index + 1)? {
                let mut pin = context.clone();
                pin.paths = paths;
                scope.contexts.push(pin);
            }
        }
        // Source-confirmed integrations never authorize retrieval of external contracts.
        scope.contracts.clear();
        scope.model_destinations.clear();
        Ok(Some(scope))
    }

    pub fn accepted_scope(
        &self,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<ScopeRecord, String> {
        if self.state != OnboardingState::Confirmed {
            return Err("Architecture has not been confirmed".into());
        }
        self.proposed_scope(principal, grants)?
            .ok_or("Confirmed architecture has no accepted primary paths".into())
    }

    pub fn proposal_fingerprint(&self) -> String {
        let mut digest = Sha256::new();
        fn field(digest: &mut Sha256, value: &str) {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value);
        }
        fn list(digest: &mut Sha256, values: &[String]) {
            digest.update((values.len() as u64).to_be_bytes());
            for value in values {
                field(digest, value);
            }
        }
        field(&mut digest, &self.input_fingerprint());
        digest.update((self.proposals.len() as u64).to_be_bytes());
        for proposal in &self.proposals {
            for value in [&proposal.id, &proposal.name, &proposal.role] {
                field(&mut digest, value);
            }
            digest.update((proposal.repository_index as u64).to_be_bytes());
            list(&mut digest, &proposal.roots);
            list(&mut digest, &proposal.stack_candidates);
            // Preserve historical fingerprints for records written before signal support.
            if !proposal.signals.is_empty() {
                field(&mut digest, "component-signals-v1");
                digest.update((proposal.signals.len() as u64).to_be_bytes());
                for signal in &proposal.signals {
                    for value in [&signal.kind, &signal.name, &signal.reason] {
                        field(&mut digest, value);
                    }
                    digest.update((signal.evidence_index as u64).to_be_bytes());
                    digest.update([u8::from(signal.test_only)]);
                }
            }
            field(
                &mut digest,
                match proposal.confidence {
                    EvidenceConfidence::Observed => "observed",
                    EvidenceConfidence::Corroborated => "corroborated",
                    EvidenceConfidence::Candidate => "candidate",
                    EvidenceConfidence::UserSupplied => "user_supplied",
                },
            );
            field(
                &mut digest,
                match proposal.decision {
                    ComponentDecision::Pending => "pending",
                    ComponentDecision::Confirmed => "confirmed",
                    ComponentDecision::Corrected => "corrected",
                    ComponentDecision::Rejected => "rejected",
                    ComponentDecision::Deferred => "deferred",
                },
            );
            list(&mut digest, &proposal.contradictions);
            digest.update((proposal.evidence.len() as u64).to_be_bytes());
            for evidence in &proposal.evidence {
                digest.update((evidence.repository_index as u64).to_be_bytes());
                field(&mut digest, &evidence.path);
                field(&mut digest, &evidence.blob);
                digest.update(evidence.start_line.to_be_bytes());
                digest.update(evidence.end_line.to_be_bytes());
                field(&mut digest, &evidence.producer_version);
            }
        }
        digest.update((self.provider_mappings.len() as u64).to_be_bytes());
        for mapping in &self.provider_mappings {
            if !mapping.evidence.is_empty()
                || !mapping.notes.is_empty()
                || mapping.decision != ComponentDecision::Pending
            {
                field(&mut digest, "provider-static-evidence-v1");
                field(
                    &mut digest,
                    match mapping.decision {
                        ComponentDecision::Pending => "pending",
                        ComponentDecision::Confirmed => "confirmed",
                        ComponentDecision::Corrected => "corrected",
                        ComponentDecision::Rejected => "rejected",
                        ComponentDecision::Deferred => "deferred",
                    },
                );
                list(&mut digest, &mapping.notes);
                digest.update((mapping.evidence.len() as u64).to_be_bytes());
                for evidence in &mapping.evidence {
                    digest.update((evidence.repository_index as u64).to_be_bytes());
                    field(&mut digest, &evidence.path);
                    field(&mut digest, &evidence.blob);
                    digest.update(evidence.start_line.to_be_bytes());
                    digest.update(evidence.end_line.to_be_bytes());
                    field(&mut digest, &evidence.producer_version);
                }
            }
            for value in [
                &mapping.component_id,
                &mapping.provider,
                &mapping.environment,
                &mapping.source_root,
            ] {
                field(&mut digest, value);
            }
            digest.update((mapping.repository_index as u64).to_be_bytes());
            for value in [
                &mapping.project_id,
                &mapping.service_id,
                &mapping.observed_commit,
            ] {
                digest.update([u8::from(value.is_some())]);
                if let Some(value) = value {
                    field(&mut digest, value);
                }
            }
            digest.update([u8::from(mapping.verified_at_ms.is_some())]);
            if let Some(value) = mapping.verified_at_ms {
                digest.update(value.to_be_bytes());
            }
            field(
                &mut digest,
                match mapping.verification {
                    ProviderVerification::NotRequested => "not_requested",
                    ProviderVerification::Verified => "verified",
                    ProviderVerification::Mismatch => "mismatch",
                    ProviderVerification::Failed => "failed",
                    ProviderVerification::Stale => "stale",
                },
            );
        }
        list(&mut digest, &self.gaps);
        if let Some(version) = &self.question_version {
            field(&mut digest, version);
            for integration in &self.integrations {
                for value in [&integration.id, &format!("{:?}", integration.decision)] {
                    field(&mut digest, value);
                }
                digest.update([u8::from(integration.answer.is_some())]);
                if let Some(answer) = &integration.answer {
                    field(&mut digest, answer);
                }
            }
        }
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn confirm(
        &mut self,
        expected: u64,
        fingerprint: &str,
        known_gaps_accepted: bool,
        at_ms: i64,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<OnboardingConfirmation, String> {
        self.check_access(principal, grants)?;
        if !grants.iter().any(|grant| {
            grant.principal == *principal
                && grant.repository == self.scope.primary_repository
                && grant.actions.contains(&Action::ConfirmArchitecture)
                && scope_is_covered(&self.scope.primary_paths, &grant.paths)
        }) {
            return Err("Architecture confirmation access denied".into());
        }
        if let Some(confirmation) = &self.confirmation {
            if expected.checked_add(1) == Some(self.revision)
                && confirmation.proposal_fingerprint == fingerprint
                && confirmation.known_gaps_accepted == known_gaps_accepted
            {
                return Ok(confirmation.clone());
            }
            return Err("Confirmation differs from the immutable accepted version".into());
        }
        self.check_revision(expected)?;
        if self.state != OnboardingState::AwaitingConfirmation
            || fingerprint != self.proposal_fingerprint()
            || at_ms < 0
        {
            return Err("Stale or invalid confirmation proposal".into());
        }
        if self.question_version.is_none() && !questions::integrations(self).0.is_empty() {
            return Err(
                "Architecture requires a new onboarding scan for explicit integration choices"
                    .into(),
            );
        }
        for component in self
            .proposals
            .iter()
            .filter(|item| self.component_is_included(&item.id))
        {
            self.validate_component_roots(component, false)?;
        }
        if self
            .proposals
            .iter()
            .any(|item| item.decision == ComponentDecision::Pending)
        {
            return Err("Every component requires an explicit decision".into());
        }
        if self.provider_mappings.iter().any(|mapping| {
            self.component_is_included(&mapping.component_id)
                && mapping.decision == ComponentDecision::Pending
        }) {
            return Err("Every provider mapping requires an explicit decision; static detection cannot confirm hosting".into());
        }
        if self.integrations.iter().any(|item| {
            self.component_is_included(&item.component_id)
                && item.decision == IntegrationDecision::Pending
        }) {
            return Err("Every included component integration requires an explicit include, exclude or defer decision".into());
        }
        if self.known_gaps_required() && !known_gaps_accepted {
            return Err("Known gaps require explicit acceptance".into());
        }
        let confirmation = OnboardingConfirmation {
            actor: principal.as_str().into(),
            confirmed_at_ms: at_ms,
            input_fingerprint: self.input_fingerprint(),
            proposal_fingerprint: fingerprint.into(),
            known_gaps_accepted,
        };
        self.confirmation = Some(confirmation.clone());
        self.state = OnboardingState::Confirmed;
        self.revision += 1;
        Ok(confirmation)
    }

    pub fn accepted_components(&self) -> Result<Vec<&ComponentProposal>, String> {
        if self.state != OnboardingState::Confirmed {
            return Err("Architecture has not been confirmed".into());
        }
        Ok(self
            .proposals
            .iter()
            .filter(|item| {
                matches!(
                    item.decision,
                    ComponentDecision::Confirmed | ComponentDecision::Corrected
                )
            })
            .collect())
    }
}

#[cfg(feature = "server")]
mod persistence {
    use super::*;
    use crate::{ContextPin, RepositoryId, ScopeVersion, SecurityDomain, Storage};
    use rusqlite::{OptionalExtension, TransactionBehavior, params};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RepositoryWire {
        domain: String,
        provider: String,
        owner: String,
        name: String,
    }
    impl RepositoryWire {
        fn from_model(repo: &RepositoryId) -> Self {
            Self {
                domain: repo.security_domain().as_str().into(),
                provider: repo.provider().into(),
                owner: repo.owner().into(),
                name: repo.name().into(),
            }
        }
        fn to_model(&self) -> Result<RepositoryId, String> {
            RepositoryId::new(
                SecurityDomain::new(&self.domain)?,
                &self.provider,
                &self.owner,
                &self.name,
            )
        }
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PinWire {
        name: String,
        repository: RepositoryWire,
        commit: String,
        includes: Vec<String>,
        excludes: Vec<String>,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SessionWire {
        version: String,
        id: String,
        principal: String,
        repository: RepositoryWire,
        commit: String,
        scope_version: String,
        includes: Vec<String>,
        excludes: Vec<String>,
        contexts: Vec<PinWire>,
        contracts: BTreeSet<String>,
        export_allowed: bool,
        scan_version: String,
        revision: u64,
        state: OnboardingState,
        proposals: Vec<ComponentProposal>,
        provider_mappings: Vec<ComponentProviderMapping>,
        #[serde(default)]
        integrations: Vec<IntegrationCandidate>,
        #[serde(default)]
        question_version: Option<String>,
        gaps: Vec<String>,
        confirmation: Option<OnboardingConfirmation>,
        #[serde(default)]
        inventory: Option<crate::OnboardingInventory>,
    }
    impl SessionWire {
        fn from_model(session: &OnboardingSession) -> Self {
            Self {
                version: ONBOARDING_VERSION.into(),
                id: session.id.clone(),
                principal: session.principal.as_str().into(),
                repository: RepositoryWire::from_model(&session.scope.primary_repository),
                commit: session.commit.as_str().into(),
                scope_version: session.scope.version.as_str().into(),
                includes: session
                    .scope
                    .primary_paths
                    .includes()
                    .map(str::to_owned)
                    .collect(),
                excludes: session
                    .scope
                    .primary_paths
                    .excludes()
                    .map(str::to_owned)
                    .collect(),
                contexts: session
                    .scope
                    .contexts
                    .iter()
                    .map(|pin| PinWire {
                        name: pin.name.clone(),
                        repository: RepositoryWire::from_model(&pin.repository),
                        commit: pin.commit.as_str().into(),
                        includes: pin.paths.includes().map(str::to_owned).collect(),
                        excludes: pin.paths.excludes().map(str::to_owned).collect(),
                    })
                    .collect(),
                contracts: session.scope.contracts.clone(),
                export_allowed: session.scope.export_allowed,
                scan_version: session.scan_version.clone(),
                revision: session.revision,
                state: session.state,
                proposals: session.proposals.clone(),
                provider_mappings: session.provider_mappings.clone(),
                integrations: session.integrations.clone(),
                question_version: session.question_version.clone(),
                gaps: session.gaps.clone(),
                confirmation: session.confirmation.clone(),
                inventory: session.inventory.clone(),
            }
        }
        fn into_model(
            self,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<OnboardingSession, String> {
            if self.version != ONBOARDING_VERSION || self.principal != principal.as_str() {
                return Err("Onboarding session access denied or version unsupported".into());
            }
            let contexts = self
                .contexts
                .into_iter()
                .map(|pin| {
                    Ok(ContextPin {
                        name: pin.name,
                        repository: pin.repository.to_model()?,
                        commit: SnapshotId::new(pin.commit)?,
                        paths: PathScope::new(pin.includes, pin.excludes)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let scope = ScopeRecord {
                version: ScopeVersion::new(self.scope_version)?,
                primary_repository: self.repository.to_model()?,
                primary_paths: PathScope::new(self.includes, self.excludes)?,
                contexts,
                contracts: self.contracts,
                export_allowed: self.export_allowed,
                model_destinations: BTreeSet::new(),
            };
            let mut session = OnboardingSession::new(
                self.id,
                principal.clone(),
                SnapshotId::new(self.commit)?,
                scope,
                self.scan_version,
                grants,
            )?;
            session.validate_proposals(&self.proposals, &self.provider_mappings, &self.gaps)?;
            session.revision = self.revision;
            session.state = self.state;
            session.proposals = self.proposals;
            session.provider_mappings = self.provider_mappings;
            session.gaps = self.gaps;
            session.question_version = self.question_version;
            session.integrations = self.integrations;
            session.validate_integrations()?;
            if (session.state == OnboardingState::Confirmed) != self.confirmation.is_some() {
                return Err("Invalid stored confirmation state".into());
            }
            if let Some(receipt) = &self.confirmation {
                if receipt.actor != principal.as_str()
                    || receipt.confirmed_at_ms < 0
                    || receipt.input_fingerprint != session.input_fingerprint()
                    || receipt.proposal_fingerprint != session.proposal_fingerprint()
                    || session
                        .proposals
                        .iter()
                        .any(|item| item.decision == ComponentDecision::Pending)
                    || session.provider_mappings.iter().any(|item| {
                        session.component_is_included(&item.component_id)
                            && item.decision == ComponentDecision::Pending
                    })
                    || session.integrations.iter().any(|item| {
                        session.component_is_included(&item.component_id)
                            && item.decision == IntegrationDecision::Pending
                    })
                {
                    return Err("Stored confirmation does not match its evidence".into());
                }
            }
            if let Some(inventory) = &self.inventory {
                inventory.validate(&session)?;
            }
            session.inventory = self.inventory;
            session.confirmation = self.confirmation;
            Ok(session)
        }
    }

    impl Storage {
        /// A scope's canonical boundary already has an immutable registered version.
        /// Onboarding may reuse that identity; arbitrary scope-version collisions remain errors.
        pub(crate) fn onboarding_scope_version(
            &self,
            session: &OnboardingSession,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<Option<ScopeVersion>, String> {
            session.check_access(principal, grants)?;
            let repo = &session.scope.primary_repository;
            let version: Option<String> = self.connection.query_row(
                "SELECT scope.version FROM scopes scope JOIN repositories repo ON repo.id=scope.repository_id WHERE repo.security_domain=?1 AND repo.provider=?2 AND repo.owner=?3 AND repo.name=?4 AND scope.canonical=?5",
                params![repo.security_domain().as_str(),repo.provider(),repo.owner(),repo.name(),session.scope.canonical_bytes()],
                |row| row.get(0),
            ).optional().map_err(|error|error.to_string())?;
            version.map(ScopeVersion::new).transpose()
        }

        /// Persist each mutation separately. Compare-and-save and history append
        /// are one transaction; this does not make the scope an active PR default.
        pub fn save_onboarding_session(
            &mut self,
            session: &OnboardingSession,
            expected_revision: Option<u64>,
            at_ms: i64,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<bool, String> {
            self.save_onboarding_session_with_defaults(
                session,
                expected_revision,
                at_ms,
                principal,
                grants,
                false,
            )
        }

        /// Confirmation/history and an optional explicit active-context selection commit together.
        pub fn save_onboarding_session_with_defaults(
            &mut self,
            session: &OnboardingSession,
            expected_revision: Option<u64>,
            at_ms: i64,
            principal: &PrincipalId,
            grants: &[Grant],
            activate_defaults: bool,
        ) -> Result<bool, String> {
            session.check_access(principal, grants)?;
            if session.state == OnboardingState::Confirmed
                && !grants.iter().any(|grant| {
                    grant.principal == *principal
                        && grant.repository == session.scope.primary_repository
                        && grant.actions.contains(&Action::ConfirmArchitecture)
                        && scope_is_covered(&session.scope.primary_paths, &grant.paths)
                })
            {
                return Err("Architecture confirmation access denied".into());
            }
            if at_ms < 0 || session.revision > i64::MAX as u64 {
                return Err("Invalid onboarding timestamp or revision".into());
            }
            let payload = serde_json::to_vec(&SessionWire::from_model(session))
                .map_err(|_| "Cannot encode onboarding session")?;
            if payload.len() > 1048576 {
                return Err("Onboarding session exceeds 1 MiB".into());
            }
            let accepted_payload = if activate_defaults {
                if session.state != OnboardingState::Confirmed || session.confirmation.is_none() {
                    return Err("Review defaults require a confirmed architecture".into());
                }
                if session.question_version.is_none()
                    && !questions::integrations(session).0.is_empty()
                {
                    return Err("Review defaults require a new onboarding scan for explicit integration choices".into());
                }
                if !grants.iter().any(|grant| {
                    grant.principal == *principal
                        && grant.repository == session.scope.primary_repository
                        && grant.actions.contains(&Action::ConfirmArchitecture)
                        && scope_is_covered(&session.scope.primary_paths, &grant.paths)
                }) {
                    return Err("Architecture defaults access denied".into());
                }
                session.accepted_scope(principal, grants)?;
                let context =
                    crate::onboarding_service::accepted_context_view(session, principal, grants)?;
                if context["usable"] != true {
                    return Err("Review defaults require usable accepted context".into());
                }
                crate::onboarding_service::validate_accepted_context_scope(&context)?;
                if context["gaps"]
                    .as_array()
                    .is_some_and(|gaps| !gaps.is_empty())
                    && session
                        .confirmation
                        .as_ref()
                        .is_none_or(|confirmation| !confirmation.known_gaps_accepted)
                {
                    return Err("Derived accepted-context gaps require explicit acceptance".into());
                }
                Some(serde_json::to_vec(&context).map_err(|_| "Cannot encode accepted context")?)
            } else {
                None
            };
            let repo = &session.scope.primary_repository;
            let repository_id = self.register_repository(
                repo.security_domain().as_str(),
                repo.provider(),
                repo.owner(),
                repo.name(),
            )?;
            let scope_id = self.register_scope(
                repository_id,
                session.scope.version.as_str(),
                &session.scope.canonical_bytes(),
            )?;
            let domain = repo.security_domain().as_str();
            let manifest = session.input_fingerprint();
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            let existing:Option<(i64,String,Vec<u8>)>=transaction.query_row("SELECT revision,manifest_fingerprint,payload FROM onboarding_sessions WHERE security_domain=?1 AND session_id=?2",params![domain,session.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|error|error.to_string())?;
            let created = existing.is_none();
            let mut changed = true;
            if let Some((revision, old_manifest, old_payload)) = existing {
                if old_manifest != manifest {
                    return Err(
                        "Session input is immutable; create a new onboarding session".into(),
                    );
                }
                if old_payload == payload {
                    changed = false;
                } else {
                    if expected_revision != Some(revision as u64)
                        || (revision as u64).checked_add(1) != Some(session.revision)
                    {
                        return Err("Stale onboarding save; reload the latest session".into());
                    }
                    transaction.execute("UPDATE onboarding_sessions SET revision=?3,payload=?4 WHERE security_domain=?1 AND session_id=?2",params![domain,session.id,session.revision as i64,payload]).map_err(|error|error.to_string())?;
                }
            } else {
                if expected_revision.is_some()
                    || session.revision != 0
                    || session.state != OnboardingState::Draft
                {
                    return Err("A new stored session must be an initial draft".into());
                }
                transaction.execute("INSERT INTO onboarding_sessions(security_domain,session_id,repository_id,scope_id,manifest_fingerprint,revision,payload) VALUES (?1,?2,?3,?4,?5,0,?6)",params![domain,session.id,repository_id,scope_id,manifest,payload]).map_err(|error|error.to_string())?;
            }
            if changed {
                transaction.execute("INSERT INTO onboarding_history(security_domain,session_id,revision,actor,recorded_at_ms,payload) VALUES (?1,?2,?3,?4,?5,?6)",params![domain,session.id,session.revision as i64,principal.as_str(),at_ms,payload]).map_err(|error|error.to_string())?;
            }
            if let Some(accepted_payload) = accepted_payload {
                let context_id = format!(
                    "{}:{}:{}",
                    session.id,
                    session.revision,
                    session.proposal_fingerprint()
                );
                let existing: Option<Vec<u8>> = transaction.query_row("SELECT payload FROM onboarding_accepted_contexts WHERE repository_id=?1 AND context_id=?2", params![repository_id,context_id], |row| row.get(0)).optional().map_err(|error|error.to_string())?;
                if existing
                    .as_ref()
                    .is_some_and(|old| old != &accepted_payload)
                {
                    return Err("Accepted context is immutable".into());
                }
                if existing.is_none() {
                    transaction.execute("INSERT INTO onboarding_accepted_contexts(repository_id,context_id,session_id,session_revision,actor,accepted_at_ms,payload) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![repository_id,context_id,session.id,session.revision as i64,principal.as_str(),at_ms,accepted_payload]).map_err(|error|error.to_string())?;
                }
                transaction.execute("INSERT INTO onboarding_defaults(repository_id,context_id,selected_by,selected_at_ms) VALUES (?1,?2,?3,?4) ON CONFLICT(repository_id) DO UPDATE SET context_id=excluded.context_id,selected_by=excluded.selected_by,selected_at_ms=excluded.selected_at_ms",params![repository_id,context_id,principal.as_str(),at_ms]).map_err(|error|error.to_string())?;
            }
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(created)
        }

        /// Shared repository defaults do not confer access to their creator's private session.
        pub fn load_onboarding_defaults(
            &self,
            repository: &RepositoryId,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<serde_json::Value, String> {
            if !grants.iter().any(|grant| {
                grant.principal == *principal
                    && grant.repository == *repository
                    && grant.actions.contains(&Action::Index)
            }) {
                return Err("Repository defaults access denied".into());
            }
            let payload: Vec<u8> = self.connection.query_row("SELECT context.payload FROM onboarding_defaults selected JOIN onboarding_accepted_contexts context ON context.repository_id=selected.repository_id AND context.context_id=selected.context_id JOIN repositories repository ON repository.id=selected.repository_id WHERE repository.security_domain=?1 AND repository.provider=?2 AND repository.owner=?3 AND repository.name=?4",params![repository.security_domain().as_str(),repository.provider(),repository.owner(),repository.name()],|row|row.get(0)).map_err(|_|"Onboarding defaults unavailable")?;
            let value: serde_json::Value =
                serde_json::from_slice(&payload).map_err(|_| "Invalid stored defaults")?;
            let parse_repository = |value: &serde_json::Value| -> Result<RepositoryId, String> {
                let field = |key: &str| {
                    value[key]
                        .as_str()
                        .ok_or_else(|| "Invalid stored repository".to_string())
                };
                RepositoryId::new(
                    SecurityDomain::new(field("security_domain")?)?,
                    field("provider")?,
                    field("owner")?,
                    field("name")?,
                )
            };
            if parse_repository(&value["repository"])? != *repository
                || value["usable"] != true
                || value["version"] != "csb-accepted-context-v1"
            {
                return Err("Invalid stored defaults identity".into());
            }
            let validate_paths = |repository: &RepositoryId,
                                  value: &serde_json::Value|
             -> Result<(), String> {
                let list = |key: &str| -> Result<Vec<String>, String> {
                    value[key]
                        .as_array()
                        .ok_or("Invalid stored paths")?
                        .iter()
                        .map(|item| {
                            item.as_str()
                                .map(str::to_owned)
                                .ok_or("Invalid stored path".into())
                        })
                        .collect()
                };
                let paths = PathScope::new(list("include")?, list("exclude")?)?;
                authorize_repository(Action::Index, principal, repository, grants, None, &paths)?;
                if !grants.iter().any(|grant| {
                    grant.principal == *principal
                        && grant.repository == *repository
                        && grant.actions.contains(&Action::Index)
                        && scope_is_covered(&paths, &grant.paths)
                }) {
                    return Err("Repository defaults exceed the current grant".into());
                }
                Ok(())
            };
            validate_paths(repository, &value)?;
            for context in value["contexts"]
                .as_array()
                .ok_or("Invalid stored contexts")?
            {
                let related = parse_repository(&context["repository"])?;
                if related.security_domain() != repository.security_domain() {
                    return Err("Context defaults security domain mismatch".into());
                }
                validate_paths(&related, context)?;
            }
            crate::onboarding_service::validate_accepted_context_scope(&value)?;
            // Inspect immutable private history for validation only. This grants
            // no session access and emits none of the creator's broader evidence.
            let denied = || {
                "Onboarding defaults unavailable; new boundary and explicit acceptance required"
                    .to_string()
            };
            let id = value["session_id"].as_str().ok_or_else(denied)?;
            let revision = value["session_revision"].as_i64().ok_or_else(denied)?;
            let original: Vec<u8> = self.connection.query_row("SELECT payload FROM onboarding_history WHERE security_domain=?1 AND session_id=?2 AND revision=?3",params![repository.security_domain().as_str(),id,revision],|row|row.get(0)).map_err(|_|denied())?;
            if original.len() > 1048576 {
                return Err(denied());
            }
            let original: SessionWire = serde_json::from_slice(&original).map_err(|_| denied())?;
            if original.id != id
                || original.revision != revision as u64
                || original.repository.to_model()? != *repository
                || original.state != OnboardingState::Confirmed
                || value["commit"] != original.commit
                || serde_json::to_value(&original.confirmation).map_err(|_| denied())?
                    != value["confirmation"]
            {
                return Err(denied());
            }
            crate::onboarding_service::validate_accepted_context_interpretations(
                &value,
                &original.proposals,
            )?;
            Ok(value)
        }

        pub fn onboarding_defaults_match(
            &self,
            session: &OnboardingSession,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<bool, String> {
            session.check_access(principal, grants)?;
            let repository = &session.scope.primary_repository;
            let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM onboarding_defaults selected JOIN repositories repository ON repository.id=selected.repository_id WHERE repository.security_domain=?1 AND repository.provider=?2 AND repository.owner=?3 AND repository.name=?4)",params![repository.security_domain().as_str(),repository.provider(),repository.owner(),repository.name()],|row|row.get(0)).map_err(|error|error.to_string())?;
            if !exists {
                return Ok(false);
            }
            let context = match self.load_onboarding_defaults(repository, principal, grants) {
                Ok(context) => context,
                // This is only a display predicate. An unavailable or currently
                // unauthorized older default must not obstruct a new boundary.
                // Direct defaults loading and PR analysis still fail closed.
                Err(_) => return Ok(false),
            };
            Ok(context["session_id"] == session.id
                && context["session_revision"] == session.revision)
        }

        pub fn load_onboarding_session(
            &self,
            domain: &SecurityDomain,
            id: &str,
            principal: &PrincipalId,
            grants: &[Grant],
        ) -> Result<OnboardingSession, String> {
            label(id)?;
            let (revision,manifest,payload):(i64,String,Vec<u8>)=self.connection.query_row("SELECT revision,manifest_fingerprint,payload FROM onboarding_sessions WHERE security_domain=?1 AND session_id=?2",params![domain.as_str(),id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(|_|"Onboarding session unavailable")?;
            let wire: SessionWire = serde_json::from_slice(&payload)
                .map_err(|_| "Invalid stored onboarding payload")?;
            let session = wire.into_model(principal, grants)?;
            if session.revision != revision as u64
                || session.input_fingerprint() != manifest
                || session.id != id
                || session.scope.primary_repository.security_domain() != domain
            {
                return Err("Stored onboarding identity mismatch".into());
            }
            Ok(session)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RepositoryId, ScopeVersion, SecurityDomain};
    fn fixture() -> (OnboardingSession, PrincipalId, Vec<Grant>) {
        let principal = PrincipalId::new("maintainer").unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("local").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope-1").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::new(vec!["src".into()], vec!["src/private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let grants = vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::ConfirmArchitecture]),
            paths: scope.primary_paths.clone(),
        }];
        (
            OnboardingSession::new(
                "onboarding-1".into(),
                principal.clone(),
                SnapshotId::new("a".repeat(40)).unwrap(),
                scope,
                "scan-1".into(),
                &grants,
            )
            .unwrap(),
            principal,
            grants,
        )
    }
    fn component(id: &str) -> ComponentProposal {
        ComponentProposal {
            id: id.into(),
            name: "Rust engine".into(),
            role: "library".into(),
            repository_index: 0,
            roots: vec!["src".into()],
            stack_candidates: vec!["Rust".into()],
            signals: vec![],
            evidence: vec![OnboardingEvidence {
                repository_index: 0,
                path: "src/lib.rs".into(),
                blob: "b".repeat(40),
                start_line: 1,
                end_line: 20,
                producer_version: "rust-manifest-v1".into(),
            }],
            confidence: EvidenceConfidence::Candidate,
            contradictions: vec![],
            decision: ComponentDecision::Pending,
        }
    }
    fn provider() -> ComponentProviderMapping {
        ComponentProviderMapping {
            component_id: "engine".into(),
            provider: "Vercel".into(),
            project_id: None,
            service_id: None,
            environment: "unknown".into(),
            repository_index: 0,
            source_root: "src".into(),
            observed_commit: Some("a".repeat(40)),
            verified_at_ms: None,
            verification: ProviderVerification::NotRequested,
            evidence: component("engine").evidence,
            notes: vec!["Static config is not a live provider observation".into()],
            decision: ComponentDecision::Pending,
        }
    }

    #[test]
    fn corrected_roots_require_indexed_pinned_evidence_and_reject_invented_or_sensitive_paths() {
        let (mut session, principal, grants) = fixture();
        let mut inventory = crate::OnboardingInventory::empty(&session);
        inventory.complete = true;
        inventory.items = vec![
            crate::InventoryItem {
                repository_index: 0,
                path: "src/server/lib.rs".into(),
                blob: "b".repeat(40),
                kind: "source".into(),
                status: "indexed".into(),
                variable_names: vec![],
            },
            crate::InventoryItem {
                repository_index: 0,
                path: "src/manifest-only/Cargo.toml".into(),
                blob: "c".repeat(40),
                kind: "manifest".into(),
                status: "indexed".into(),
                variable_names: vec![],
            },
            crate::InventoryItem {
                repository_index: 0,
                path: "src/deferred/lib.rs".into(),
                blob: "d".repeat(40),
                kind: "source".into(),
                status: "deferred".into(),
                variable_names: vec![],
            },
            crate::InventoryItem {
                repository_index: 0,
                path: "src/config/vercel.json".into(),
                blob: "e".repeat(40),
                kind: "hosting_config".into(),
                status: "indexed".into(),
                variable_names: vec![],
            },
        ];
        session
            .record_inventory(0, inventory, &principal, &grants)
            .unwrap();
        let original = component("engine");
        session
            .set_proposal(
                1,
                vec![original.clone()],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        let mut correction = original.clone();
        correction.decision = ComponentDecision::Corrected;
        for invalid_root in [
            "src/not-present",
            "src/deferred",
            "src/config",
            "src/credentials",
            "src/private",
        ] {
            correction.roots = vec![invalid_root.into()];
            assert!(
                session
                    .decide_component(3, correction.clone(), &principal, &grants)
                    .is_err(),
                "{invalid_root}"
            );
            assert_eq!(session.revision(), 3);
            assert_eq!(session.proposals()[0].roots, original.roots);
        }
        correction.roots = vec!["src/server".into()];
        session
            .decide_component(3, correction.clone(), &principal, &grants)
            .unwrap();
        assert_eq!(session.proposals()[0].evidence, original.evidence);
        assert_eq!(session.proposals()[0].confidence, original.confidence);
        correction.roots = vec!["src/manifest-only".into()];
        session
            .decide_component(4, correction, &principal, &grants)
            .unwrap();
        session
            .confirm(
                5,
                &session.proposal_fingerprint(),
                false,
                1,
                &principal,
                &grants,
            )
            .unwrap();
        let accepted = session.accepted_scope(&principal, &grants).unwrap();
        assert!(
            accepted
                .primary_paths
                .allows("src/manifest-only/Cargo.toml")
        );
        assert!(!accepted.primary_paths.allows("src/server/lib.rs"));
        let (mut unscanned, principal, grants) = fixture();
        unscanned
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        unscanned
            .set_proposal(
                1,
                vec![original.clone()],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        unscanned
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        let mut correction = original;
        correction.roots = vec!["src/server".into()];
        correction.decision = ComponentDecision::Corrected;
        assert!(
            unscanned
                .decide_component(3, correction, &principal, &grants)
                .is_err()
        );
    }

    #[test]
    fn questions_are_evidence_bound_and_decisions_do_not_expand_context() {
        let (mut session, principal, grants) = fixture();
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        let mut engine = component("engine");
        engine.signals.push(ComponentSignal {
            kind: "external contract".into(),
            name: "HTTP client".into(),
            reason: "An import is observed; no remote destination is contacted".into(),
            evidence_index: 0,
            test_only: false,
        });
        let mut shared = component("shared");
        shared.roots = vec!["src/shared".into()];
        session
            .set_proposal(
                1,
                vec![engine.clone(), shared.clone()],
                vec![provider()],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        assert_eq!(session.questions().len(), 4);
        assert_eq!(session.integrations().len(), 1);
        let questions = session.questions();
        let id = session.integrations()[0].id.clone();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        engine.name = "Reviewed API".into();
        engine.role = "backend".into();
        engine.decision = ComponentDecision::Corrected;
        session
            .decide_component(3, engine, &principal, &grants)
            .unwrap();
        assert_eq!(questions[0].fingerprint, session.questions()[0].fingerprint);
        assert_eq!(id, session.integrations()[0].id);
        shared.decision = ComponentDecision::Rejected;
        session
            .decide_component(4, shared, &principal, &grants)
            .unwrap();
        let mut corrected = provider();
        corrected.provider = "self-hosted".into();
        corrected.environment = "production".into();
        corrected.decision = ComponentDecision::Corrected;
        let mut forged = corrected.clone();
        forged.verification = ProviderVerification::Verified;
        forged.verified_at_ms = Some(2);
        forged.project_id = Some("invented".into());
        assert!(
            session
                .decide_provider(5, 0, forged, &principal, &grants)
                .is_err()
        );
        let mut outside = corrected.clone();
        outside.source_root = "outside".into();
        assert!(
            session
                .decide_provider(5, 0, outside, &principal, &grants)
                .is_err()
        );
        session
            .decide_provider(5, 0, corrected, &principal, &grants)
            .unwrap();
        assert!(
            session
                .confirm(
                    6,
                    &session.proposal_fingerprint(),
                    true,
                    3,
                    &principal,
                    &grants
                )
                .is_err()
        );
        assert!(
            session
                .decide_integration(
                    5,
                    &id,
                    IntegrationDecision::Exclude,
                    None,
                    &principal,
                    &grants
                )
                .is_err()
        );
        session
            .decide_integration(
                6,
                &id,
                IntegrationDecision::Exclude,
                Some("Keep remote contract out of context".into()),
                &principal,
                &grants,
            )
            .unwrap();
        session
            .confirm(
                7,
                &session.proposal_fingerprint(),
                true,
                3,
                &principal,
                &grants,
            )
            .unwrap();
        let scope = session.accepted_scope(&principal, &grants).unwrap();
        assert!(scope.primary_paths.allows("src/server.rs"));
        assert!(!scope.primary_paths.allows("src/shared/lib.rs"));
        assert!(!scope.primary_paths.allows("src/private/credentials"));
        assert!(!scope.primary_paths.allows("outside/file.rs"));
        assert!(
            scope.contexts.is_empty()
                && scope.contracts.is_empty()
                && scope.model_destinations.is_empty()
        );
        assert!(session.accepted_scope(&principal, &[]).is_err());
    }

    #[cfg(feature = "server")]
    #[test]
    fn accepted_defaults_are_atomic_durable_filtered_and_authorized_for_each_viewer() {
        use crate::Storage;
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "csb-defaults-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut storage =
            Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        let (mut session, principal, grants) = fixture();
        let repo = session.scope.primary_repository.clone();
        storage
            .save_onboarding_session(&session, None, 0, &principal, &grants)
            .unwrap();
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(0), 1, &principal, &grants)
            .unwrap();
        let mut engine = component("engine");
        engine
            .stack_candidates
            .push("HTTP client · static import".into());
        engine.signals.push(ComponentSignal {
            kind: "external contract".into(),
            name: "HTTP client".into(),
            reason: "Source import only; no retrieval".into(),
            evidence_index: 0,
            test_only: false,
        });
        let mut excluded = component("shared");
        excluded.name = "REJECTED_SERVICE_SENTINEL".into();
        excluded.roots = vec!["src/shared".into()];
        session
            .set_proposal(
                1,
                vec![engine.clone(), excluded.clone()],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(1), 2, &principal, &grants)
            .unwrap();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(2), 3, &principal, &grants)
            .unwrap();
        engine.decision = ComponentDecision::Confirmed;
        session
            .decide_component(3, engine, &principal, &grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(3), 4, &principal, &grants)
            .unwrap();
        excluded.decision = ComponentDecision::Deferred;
        session
            .decide_component(4, excluded, &principal, &grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(4), 5, &principal, &grants)
            .unwrap();
        let id = session.integrations()[0].id.clone();
        session
            .decide_integration(
                5,
                &id,
                IntegrationDecision::Exclude,
                None,
                &principal,
                &grants,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(5), 6, &principal, &grants)
            .unwrap();
        drop(storage);
        let mut storage =
            Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        let mut resumed = storage
            .load_onboarding_session(repo.security_domain(), session.id(), &principal, &grants)
            .unwrap();
        assert_eq!(
            resumed.integrations()[0].decision,
            IntegrationDecision::Exclude
        );
        assert_eq!(resumed.proposals()[1].decision, ComponentDecision::Deferred);
        assert!(
            storage
                .load_onboarding_defaults(&repo, &principal, &grants)
                .is_err()
        );
        let fingerprint = resumed.proposal_fingerprint();
        resumed
            .confirm(6, &fingerprint, true, 7, &principal, &grants)
            .unwrap();
        storage.connection.execute_batch("CREATE TRIGGER reject_defaults BEFORE INSERT ON onboarding_defaults BEGIN SELECT RAISE(ABORT,'transaction failure fixture'); END;").unwrap();
        assert!(
            storage
                .save_onboarding_session_with_defaults(
                    &resumed,
                    Some(6),
                    7,
                    &principal,
                    &grants,
                    true
                )
                .is_err()
        );
        assert_eq!(
            storage
                .load_onboarding_session(repo.security_domain(), session.id(), &principal, &grants)
                .unwrap()
                .state(),
            OnboardingState::AwaitingConfirmation
        );
        let history: i64 = storage
            .connection
            .query_row("SELECT count(*) FROM onboarding_history", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(history, 7);
        let contexts: i64 = storage
            .connection
            .query_row(
                "SELECT count(*) FROM onboarding_accepted_contexts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(contexts, 0);
        storage
            .connection
            .execute_batch("DROP TRIGGER reject_defaults")
            .unwrap();
        storage
            .save_onboarding_session_with_defaults(&resumed, Some(6), 7, &principal, &grants, true)
            .unwrap();
        storage
            .save_onboarding_session_with_defaults(&resumed, Some(6), 8, &principal, &grants, true)
            .unwrap();
        let history: i64 = storage
            .connection
            .query_row("SELECT count(*) FROM onboarding_history", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(history, 8);
        let reviewer = PrincipalId::new("another-reviewer").unwrap();
        let viewer_grants = vec![Grant {
            principal: reviewer.clone(),
            repository: repo.clone(),
            actions: BTreeSet::from([Action::Index]),
            paths: PathScope::new(
                vec!["src".into()],
                vec!["src/private".into(), "src/shared".into()],
            )
            .unwrap(),
        }];
        let active = storage
            .load_onboarding_defaults(&repo, &reviewer, &viewer_grants)
            .unwrap();
        assert_eq!(active["confirmation"]["actor"], principal.as_str());
        assert_eq!(active["session_revision"], 7);
        assert_eq!(active["integrations"], serde_json::json!([]));
        assert_eq!(active["components"][0]["signals"], serde_json::json!([]));
        assert!(!active.to_string().contains("REJECTED_SERVICE_SENTINEL"));
        assert!(!active.to_string().contains("HTTP client"));
        assert!(
            storage
                .load_onboarding_session(
                    repo.security_domain(),
                    session.id(),
                    &reviewer,
                    &viewer_grants
                )
                .is_err()
        );
        assert!(
            storage
                .load_onboarding_defaults(&repo, &reviewer, &[])
                .is_err()
        );
        let mut narrow = viewer_grants.clone();
        narrow[0].paths = PathScope::new(vec!["src/one".into()], vec![]).unwrap();
        assert!(
            storage
                .load_onboarding_defaults(&repo, &reviewer, &narrow)
                .is_err()
        );
        assert!(
            storage
                .save_onboarding_session_with_defaults(
                    &resumed,
                    Some(6),
                    8,
                    &reviewer,
                    &viewer_grants,
                    true
                )
                .is_err()
        );
        drop(storage);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn signal_provenance_is_bounded_immutable_and_part_of_confirmation_identity() {
        let (mut session, principal, grants) = fixture();
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        let mut proposal = component("engine");
        #[cfg(feature = "server")]
        {
            let mut legacy = serde_json::to_value(&proposal).unwrap();
            legacy.as_object_mut().unwrap().remove("signals");
            assert_eq!(
                serde_json::from_value::<ComponentProposal>(legacy).unwrap(),
                proposal
            );
        }
        proposal.signals.push(ComponentSignal {
            kind: "database".into(),
            name: "SQLite client".into(),
            reason: "A static import is observed; ownership remains unresolved".into(),
            evidence_index: 1,
            test_only: false,
        });
        assert!(
            session
                .set_proposal(
                    1,
                    vec![proposal.clone()],
                    vec![],
                    vec![],
                    &principal,
                    &grants
                )
                .is_err()
        );
        proposal.signals[0].evidence_index = 0;
        let mut other = session.clone();
        other
            .set_proposal(
                1,
                vec![component("engine")],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        session
            .set_proposal(
                1,
                vec![proposal.clone()],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        assert_ne!(session.proposal_fingerprint(), other.proposal_fingerprint());
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        proposal.decision = ComponentDecision::Corrected;
        proposal.signals[0].test_only = true;
        assert!(
            session
                .decide_component(3, proposal.clone(), &principal, &grants)
                .is_err()
        );
        assert_eq!(session.revision(), 3);
        proposal.signals[0].test_only = false;
        proposal.role = "backend".into();
        session
            .decide_component(3, proposal, &principal, &grants)
            .unwrap();
        assert_eq!(session.proposals()[0].role, "backend");
        assert!(!session.proposals()[0].signals[0].test_only);
    }

    #[test]
    fn decisions_are_versioned_confirmation_is_idempotent_and_rejected_components_stay_out() {
        let (mut session, principal, grants) = fixture();
        assert!(session.accepted_components().is_err());
        assert!(
            session
                .transition(0, OnboardingState::Confirmed, &principal, &grants)
                .is_err()
        );
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        session
            .set_proposal(
                1,
                vec![component("engine"), component("excluded")],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        assert!(
            session
                .confirm(
                    3,
                    &session.proposal_fingerprint(),
                    false,
                    10,
                    &principal,
                    &grants
                )
                .is_err()
        );
        let mut first = component("engine");
        first.decision = ComponentDecision::Confirmed;
        session
            .decide_component(3, first, &principal, &grants)
            .unwrap();
        let stale = session.proposal_fingerprint();
        let mut second = component("excluded");
        second.decision = ComponentDecision::Rejected;
        session
            .decide_component(4, second, &principal, &grants)
            .unwrap();
        assert!(
            session
                .confirm(5, &stale, false, 10, &principal, &grants)
                .is_err()
        );
        let fingerprint = session.proposal_fingerprint();
        let receipt = session
            .confirm(5, &fingerprint, false, 10, &principal, &grants)
            .unwrap();
        assert_eq!(
            session
                .confirm(5, &fingerprint, false, 20, &principal, &grants)
                .unwrap(),
            receipt
        );
        assert_eq!(session.accepted_components().unwrap().len(), 1);
        assert!(
            session
                .transition(6, OnboardingState::Scanning, &principal, &grants)
                .is_err()
        );
        assert!(
            session
                .confirm(5, &fingerprint, false, 10, &principal, &[])
                .is_err()
        );
    }
    #[test]
    fn initial_scan_cannot_expand_grants_and_evidence_cannot_escape_context() {
        let (mut session, principal, grants) = fixture();
        let mut broad = session.scope.clone();
        broad.primary_paths = PathScope::all();
        assert!(
            OnboardingSession::new(
                "broad".into(),
                principal.clone(),
                session.commit.clone(),
                broad,
                "scan-1".into(),
                &grants
            )
            .is_err()
        );
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        let mut proposal = component("engine");
        proposal.evidence[0].path = "src/private/key.rs".into();
        assert!(
            session
                .set_proposal(1, vec![proposal], vec![], vec![], &principal, &grants)
                .is_err()
        );
        let mut proposal = component("engine");
        proposal.repository_index = 1;
        assert!(
            session
                .set_proposal(1, vec![proposal], vec![], vec![], &principal, &grants)
                .is_err()
        );
        assert_eq!(session.revision(), 1);
    }
    #[test]
    fn partial_scans_and_known_gaps_cannot_silently_confirm() {
        let (mut session, principal, grants) = fixture();
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        session
            .transition(1, OnboardingState::Partial, &principal, &grants)
            .unwrap();
        session
            .set_proposal(
                2,
                vec![component("engine")],
                vec![],
                vec!["Unsupported deployment configuration".into()],
                &principal,
                &grants,
            )
            .unwrap();
        session
            .transition(
                3,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        let mut proposal = component("engine");
        proposal.decision = ComponentDecision::Confirmed;
        session
            .decide_component(4, proposal, &principal, &grants)
            .unwrap();
        let fingerprint = session.proposal_fingerprint();
        assert!(
            session
                .confirm(5, &fingerprint, false, 10, &principal, &grants)
                .is_err()
        );
        session
            .confirm(5, &fingerprint, true, 10, &principal, &grants)
            .unwrap();
    }

    #[test]
    fn context_pins_are_sorted_before_evidence_indices_are_assigned() {
        let (session, principal, mut grants) = fixture();
        let mut scope = session.scope.clone();
        for name in ["z-library", "a-library"] {
            let repository = RepositoryId::new(
                SecurityDomain::new("local").unwrap(),
                "github",
                "owner",
                name,
            )
            .unwrap();
            scope.contexts.push(crate::ContextPin {
                name: name.into(),
                repository: repository.clone(),
                commit: SnapshotId::new("c".repeat(40)).unwrap(),
                paths: PathScope::all(),
            });
            grants.push(Grant {
                principal: principal.clone(),
                repository,
                actions: BTreeSet::from([Action::Index]),
                paths: PathScope::all(),
            });
        }
        let first = OnboardingSession::new(
            "context".into(),
            principal.clone(),
            session.commit.clone(),
            scope.clone(),
            "scan-1".into(),
            &grants,
        )
        .unwrap();
        scope.contexts.reverse();
        let second = OnboardingSession::new(
            "context".into(),
            principal.clone(),
            session.commit.clone(),
            scope,
            "scan-1".into(),
            &grants,
        )
        .unwrap();
        assert_eq!(first.scope().contexts[0].name, "a-library");
        assert_eq!(first.input_fingerprint(), second.input_fingerprint());
        assert!(first.check_access(&principal, &grants[..1]).is_err());
    }

    #[cfg(feature = "server")]
    #[test]
    fn persisted_sessions_resume_and_concurrent_saves_preserve_history() {
        let (mut session, principal, grants) = fixture();
        let root = std::env::temp_dir().join(format!(
            "csb-onboarding-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut storage =
            crate::Storage::open(&root.join("db.sqlite"), &root.join("artifacts")).unwrap();
        assert!(
            storage
                .save_onboarding_session(&session, None, 1, &principal, &grants)
                .unwrap()
        );
        assert!(
            !storage
                .save_onboarding_session(&session, None, 2, &principal, &grants)
                .unwrap()
        );
        session
            .transition(0, OnboardingState::Scanning, &principal, &grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(0), 3, &principal, &grants)
            .unwrap();
        let mut stale = session.clone();
        session
            .set_proposal(
                1,
                vec![component("engine")],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(1), 4, &principal, &grants)
            .unwrap();
        stale
            .set_proposal(
                1,
                vec![component("different")],
                vec![],
                vec![],
                &principal,
                &grants,
            )
            .unwrap();
        assert!(
            storage
                .save_onboarding_session(&stale, Some(1), 5, &principal, &grants)
                .is_err()
        );
        let domain = SecurityDomain::new("local").unwrap();
        assert_eq!(
            storage
                .load_onboarding_session(&domain, session.id(), &principal, &grants)
                .unwrap(),
            session
        );
        assert!(
            storage
                .load_onboarding_session(&domain, session.id(), &principal, &[])
                .is_err()
        );
        let history: i64 = storage
            .connection
            .query_row("SELECT count(*) FROM onboarding_history", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(history, 3);
        storage.connection.execute_batch("CREATE TRIGGER fail_onboarding_history BEFORE INSERT ON onboarding_history BEGIN SELECT RAISE(ABORT,'simulated audit failure'); END;").unwrap();
        session
            .transition(
                2,
                OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        assert!(
            storage
                .save_onboarding_session(&session, Some(2), 6, &principal, &grants)
                .is_err()
        );
        assert_eq!(
            storage
                .load_onboarding_session(&domain, session.id(), &principal, &grants)
                .unwrap()
                .revision(),
            2
        );
        storage
            .connection
            .execute_batch("DROP TRIGGER fail_onboarding_history;")
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(2), 6, &principal, &grants)
            .unwrap();
        let mut proposal = component("engine");
        proposal.decision = ComponentDecision::Confirmed;
        session
            .decide_component(3, proposal, &principal, &grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(3), 7, &principal, &grants)
            .unwrap();
        let fingerprint = session.proposal_fingerprint();
        session
            .confirm(4, &fingerprint, false, 8, &principal, &grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(4), 8, &principal, &grants)
            .unwrap();
        drop(storage);
        let storage =
            crate::Storage::open(&root.join("db.sqlite"), &root.join("artifacts")).unwrap();
        let resumed = storage
            .load_onboarding_session(&domain, session.id(), &principal, &grants)
            .unwrap();
        assert_eq!(resumed.confirmation(), session.confirmation());
        assert_eq!(resumed.accepted_components().unwrap().len(), 1);
        drop(storage);
        std::fs::remove_dir_all(root).unwrap();
    }
}
