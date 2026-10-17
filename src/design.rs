//! Source-backed High-Level Design. User intent, static facts and runtime
//! unknowns remain separate. This module never executes reviewed code.
mod diff;
pub(crate) mod generate;
mod render;
pub(crate) mod wire;

use crate::{
    ComponentProposal, ComponentProviderMapping, IntegrationCandidate, RepositoryId, ScopeRecord,
    ScopeVersion, SnapshotId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use diff::{compare_hld, evaluate_hld_constraints};
pub use generate::generate_hld;
pub use render::{render_hld_markdown, render_hld_mermaid};

pub const HLD_GENERATOR_VERSION: &str = "csb-hld-rust-static-v3";
pub const HLD_PROFILE: &str = "Rust public declarations, explicit package dependencies, schema artifacts and static deployment configuration; runtime behavior unverified";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignLevel {
    #[default]
    Hld,
    Lld,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignReviewState {
    Draft,
    Partial,
    AwaitingConfirmation,
    Accepted,
    Superseded,
    Stale,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignView {
    Observed,
    AcceptedAsBuilt,
    Intended,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignResolution {
    ObservedStructure,
    ResolvedReference,
    NameOnlyCandidate,
    UserAssertion,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignElementKind {
    Component,
    Dependency,
    Contract,
    Schema,
    Deployment,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignFact {
    pub kind: String,
    pub value: String,
    pub resolution: DesignResolution,
    pub evidence_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignElement {
    pub id: String,
    pub kind: DesignElementKind,
    pub name: String,
    pub component_id: Option<String>,
    pub roots: Vec<String>,
    pub responsibility: Option<String>,
    pub user_assertions: Vec<String>,
    pub facts: Vec<DesignFact>,
    pub evidence_ids: Vec<String>,
    pub unknowns: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignRelation {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub direction: String,
    pub protocol: Option<String>,
    pub synchrony: String,
    pub resolution: DesignResolution,
    pub evidence_ids: Vec<String>,
    pub unknowns: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignEvidence {
    pub id: String,
    #[serde(with = "wire::repository")]
    pub repository: RepositoryId,
    #[serde(with = "wire::snapshot")]
    pub commit: SnapshotId,
    pub path: String,
    pub blob: String,
    pub start_line: usize,
    pub end_line: usize,
    pub producer_version: String,
    pub relationship_type: String,
    pub resolution: DesignResolution,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignSection {
    pub id: String,
    pub title: String,
    pub element_ids: Vec<String>,
    pub relation_ids: Vec<String>,
    pub unknowns: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignCoverage {
    pub profile: String,
    pub enumeration_complete: bool,
    pub indexed: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub deferred: usize,
    pub complete: bool,
    pub gaps: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignConstraint {
    pub id: String,
    pub kind: String,
    pub from_component_id: String,
    pub to_component_id: Option<String>,
    pub value: String,
    pub user_assertion: String,
    pub evidence_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignCorrespondence {
    pub kind: String,
    pub from_ids: Vec<String>,
    pub to_ids: Vec<String>,
    pub resolution: DesignResolution,
}
/// A source-qualified document available for explicit reviewer selection. Its
/// contents are untrusted text, never instructions or verified code behavior.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignDocumentCandidate {
    pub id: String,
    pub title: String,
    pub evidence_id: String,
}
/// Selected source intent plus a reviewer assertion. Observed code facts are
/// preserved independently; this is not automatic document/code verification.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignDecisionRecord {
    pub id: String,
    pub component_id: String,
    pub document_candidate_id: String,
    pub evidence_ids: Vec<String>,
    pub intent: String,
    pub resolution: DesignResolution,
    pub verification: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignVersion {
    #[serde(default, skip_serializing_if = "is_hld")]
    pub level: DesignLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lld: Option<crate::lld::LldDesignData>,
    pub id: String,
    pub generator_version: String,
    #[serde(with = "wire::repository")]
    pub repository: RepositoryId,
    #[serde(with = "wire::snapshot")]
    pub snapshot: SnapshotId,
    #[serde(with = "wire::scope_version")]
    pub scope_version: ScopeVersion,
    pub scope_fingerprint: String,
    pub onboarding_session_id: String,
    pub onboarding_revision: u64,
    pub onboarding_input_fingerprint: String,
    pub onboarding_proposal_fingerprint: String,
    pub review_state: DesignReviewState,
    pub view: DesignView,
    pub elements: Vec<DesignElement>,
    pub relations: Vec<DesignRelation>,
    pub evidence: Vec<DesignEvidence>,
    pub sections: Vec<DesignSection>,
    pub coverage: DesignCoverage,
    pub constraints: Vec<DesignConstraint>,
    pub correspondence: Vec<DesignCorrespondence>,
    #[serde(default)]
    pub document_candidates: Vec<DesignDocumentCandidate>,
    #[serde(default)]
    pub decision_records: Vec<DesignDecisionRecord>,
    pub mermaid: String,
    pub markdown: String,
}
fn is_hld(level: &DesignLevel) -> bool {
    *level == DesignLevel::Hld
}

/// Trusted confirmed-context handoff, not an authorization token. Hosted
/// callers must independently check fresh repository/path grants.
#[derive(Clone, Debug)]
pub struct ConfirmedDesignContext {
    pub accepted_commit: SnapshotId,
    pub session_id: String,
    pub session_revision: u64,
    pub input_fingerprint: String,
    pub proposal_fingerprint: String,
    pub components: Vec<ComponentProposal>,
    pub providers: Vec<ComponentProviderMapping>,
    pub integrations: Vec<IntegrationCandidate>,
    pub gaps: Vec<String>,
}

pub(crate) fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn design_scope_fingerprint(scope: &ScopeRecord) -> String {
    let mut hash = Sha256::new();
    hash.update(b"csb-hld-scope-v1");
    hash.update(scope.canonical_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub(crate) fn under(path: &str, root: &str) -> bool {
    root == "."
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
pub(crate) fn safe_path(path: &str, scope: &ScopeRecord) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.chars().any(char::is_control)
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|part| matches!(part, ".." | ".git") || part.is_empty())
        && scope.primary_paths.allows(path)
        && !crate::sensitive_path(path)
}

impl ConfirmedDesignContext {
    pub fn from_accepted(value: &serde_json::Value, scope: &ScopeRecord) -> Result<Self, String> {
        let invalid =
            || "HLD requires a bounded, usable confirmed primary-repository context".to_string();
        let field = |name: &str| value[name].as_str().ok_or_else(invalid);
        let repository = wire::repository_from_value(&value["repository"])?;
        if value["version"] != "csb-accepted-context-v1"
            || value["usable"] != true
            || value["sensitive_paths_excluded"] != true
            || repository != scope.primary_repository
            || value["confirmation"].is_null()
            || !scope.contexts.is_empty()
            || value["contexts"]
                .as_array()
                .is_none_or(|items| !items.is_empty())
        {
            return Err(invalid());
        }
        let components: Vec<ComponentProposal> =
            serde_json::from_value(value["components"].clone()).map_err(|_| invalid())?;
        let providers: Vec<ComponentProviderMapping> =
            serde_json::from_value(value["providers"].clone()).map_err(|_| invalid())?;
        let integrations: Vec<IntegrationCandidate> =
            serde_json::from_value(value["integrations"].clone()).map_err(|_| invalid())?;
        let gaps: Vec<String> =
            serde_json::from_value(value["gaps"].clone()).map_err(|_| invalid())?;
        let context = Self {
            accepted_commit: SnapshotId::new(field("commit")?.into())?,
            session_id: field("session_id")?.into(),
            session_revision: value["session_revision"].as_u64().ok_or_else(invalid)?,
            input_fingerprint: field("input_fingerprint")?.into(),
            proposal_fingerprint: field("proposal_fingerprint")?.into(),
            components,
            providers,
            integrations,
            gaps,
        };
        context.validate(scope)?;
        if value["confirmation"]["input_fingerprint"] != context.input_fingerprint
            || value["confirmation"]["proposal_fingerprint"] != context.proposal_fingerprint
        {
            return Err(invalid());
        }
        Ok(context)
    }
    pub fn validate(&self, scope: &ScopeRecord) -> Result<(), String> {
        use crate::{ComponentDecision, IntegrationDecision};
        let valid_label = |text: &str| {
            !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
        };
        let valid_hash =
            |text: &str| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !scope.contexts.is_empty()
            || !valid_label(&self.session_id)
            || self.session_revision == 0
            || !valid_hash(&self.input_fingerprint)
            || !valid_hash(&self.proposal_fingerprint)
            || self.components.is_empty()
            || self.components.len() > 100
            || self.providers.len() > 200
            || self.integrations.len() > 200
            || self.gaps.len() > 100
            || self
                .gaps
                .iter()
                .any(|gap| gap.len() > 1024 || gap.chars().any(char::is_control))
        {
            return Err(
                "HLD confirmed context exceeds the supported primary-repository profile".into(),
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        for component in &self.components {
            if component.repository_index != 0
                || !valid_label(&component.id)
                || !ids.insert(&component.id)
                || !valid_label(&component.name)
                || !valid_label(&component.role)
                || component.roots.is_empty()
                || component.roots.len() > 100
                || !matches!(
                    component.decision,
                    ComponentDecision::Confirmed | ComponentDecision::Corrected
                )
                || component.roots.iter().any(|root| !safe_path(root, scope))
            {
                return Err(
                    "HLD component interpretation is invalid or outside approved scope".into(),
                );
            }
            for evidence in &component.evidence {
                validate_onboarding_evidence(evidence, scope)?;
            }
        }
        for provider in &self.providers {
            if provider.repository_index != 0
                || !ids.contains(&provider.component_id)
                || !valid_label(&provider.provider)
                || !safe_path(&provider.source_root, scope)
                || provider.evidence.is_empty()
                || !matches!(
                    provider.decision,
                    ComponentDecision::Confirmed | ComponentDecision::Corrected
                )
            {
                return Err(
                    "HLD deployment interpretation is invalid or outside approved scope".into(),
                );
            }
            for evidence in &provider.evidence {
                validate_onboarding_evidence(evidence, scope)?;
            }
        }
        for integration in &self.integrations {
            if !ids.contains(&integration.component_id)
                || !valid_label(&integration.name)
                || integration.decision != IntegrationDecision::Include
                || integration.evidence.is_empty()
            {
                return Err("HLD integration interpretation is invalid".into());
            }
            for evidence in &integration.evidence {
                validate_onboarding_evidence(evidence, scope)?;
            }
        }
        Ok(())
    }
}
fn validate_onboarding_evidence(
    evidence: &crate::OnboardingEvidence,
    scope: &ScopeRecord,
) -> Result<(), String> {
    if evidence.repository_index != 0
        || !safe_path(&evidence.path, scope)
        || evidence.start_line == 0
        || evidence.end_line < evidence.start_line
        || SnapshotId::new(evidence.blob.clone()).is_err()
    {
        return Err("HLD onboarding evidence is outside approved scope".into());
    }
    Ok(())
}

pub fn validate_design_scope(version: &DesignVersion, scope: &ScopeRecord) -> Result<(), String> {
    use std::collections::BTreeSet;
    let label = |value: &str, max: usize| {
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
    };
    if version.repository != scope.primary_repository
        || version.scope_version != scope.version
        || version.scope_fingerprint != design_scope_fingerprint(scope)
        || version.elements.len() > 2000
        || version.relations.len() > 4000
        || version.evidence.len() > 8000
        || (version.level == DesignLevel::Hld && version.sections.len() != 9)
        || version.constraints.len() > 100
        || version.correspondence.len() > 100
        || version.document_candidates.len() > 100
        || version.decision_records.len() > 100
        || !label(&version.id, 128)
        || !label(&version.generator_version, 128)
        || !label(&version.onboarding_session_id, 128)
        || version.coverage.gaps.len() > 100
        || version.coverage.gaps.iter().any(|gap| !label(gap, 1024))
    {
        return Err("Design identity, section inventory or resource boundary is invalid".into());
    }
    if version.level == DesignLevel::Lld {
        return crate::lld::validate_lld_design(version, scope);
    }
    if version.lld.is_some() {
        return Err("HLD design cannot contain typed LLD detail".into());
    }
    let elements: BTreeSet<_> = version
        .elements
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    let evidence: BTreeSet<_> = version
        .evidence
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    if elements.len() != version.elements.len() || evidence.len() != version.evidence.len() {
        return Err("Duplicate design identity".into());
    }
    let valid_ids = |ids: &[String]| ids.iter().all(|id| evidence.contains(id.as_str()));
    let mut document_ids = BTreeSet::new();
    for candidate in &version.document_candidates {
        let source = version
            .evidence
            .iter()
            .find(|evidence| evidence.id == candidate.evidence_id)
            .ok_or("Document candidate lacks exact scoped evidence")?;
        if !label(&candidate.id, 128)
            || !document_ids.insert(candidate.id.as_str())
            || !label(&candidate.title, 256)
            || source.relationship_type != "documentation_candidate"
            || ![".md", ".mdx", ".txt"]
                .iter()
                .any(|extension| source.path.to_ascii_lowercase().ends_with(extension))
        {
            return Err("Invalid design document candidate".into());
        }
    }
    let mut decision_ids = BTreeSet::new();
    for decision in &version.decision_records {
        let candidate = version
            .document_candidates
            .iter()
            .find(|candidate| candidate.id == decision.document_candidate_id)
            .ok_or("Decision record document is unavailable")?;
        if !label(&decision.id, 128)
            || !decision_ids.insert(decision.id.as_str())
            || !label(&decision.intent, 2048)
            || !version.elements.iter().any(|element| {
                element.id == decision.component_id && element.kind == DesignElementKind::Component
            })
            || decision.evidence_ids != vec![candidate.evidence_id.clone()]
            || decision.resolution != DesignResolution::UserAssertion
            || decision.verification != "requires_reviewer_verification"
        {
            return Err(
                "Decision records must remain scoped, explicitly selected reviewer intent".into(),
            );
        }
    }
    for item in &version.evidence {
        if item.repository != version.repository
            || item.commit != version.snapshot
            || !safe_path(&item.path, scope)
            || SnapshotId::new(item.blob.clone()).is_err()
            || item.start_line == 0
            || item.end_line < item.start_line
            || item.end_line > 1_000_000
            || !label(&item.id, 128)
            || !label(&item.producer_version, 128)
            || !label(&item.relationship_type, 128)
        {
            return Err("Design evidence is outside its immutable source scope".into());
        }
    }
    for item in &version.elements {
        if item.roots.iter().any(|root| !safe_path(root, scope))
            || !valid_ids(&item.evidence_ids)
            || !label(&item.id, 128)
            || !label(&item.name, 256)
            || item.roots.len() > 100
            || item.facts.len() > 2000
            || item
                .responsibility
                .as_ref()
                .is_some_and(|value| !label(value, 4096))
            || item.user_assertions.len() > 200
            || item.user_assertions.iter().any(|value| !label(value, 4096))
            || item.unknowns.len() > 100
            || item.unknowns.iter().any(|value| !label(value, 1024))
            || item.facts.iter().any(|fact| {
                !valid_ids(&fact.evidence_ids)
                    || !label(&fact.kind, 128)
                    || fact.value.len() > 4096
                    || fact.value.chars().any(char::is_control)
                    || (matches!(
                        fact.resolution,
                        DesignResolution::ObservedStructure
                            | DesignResolution::ResolvedReference
                            | DesignResolution::NameOnlyCandidate
                    ) && fact.evidence_ids.is_empty())
            })
        {
            return Err("Design element is outside its immutable evidence scope".into());
        }
    }
    for item in &version.relations {
        if !elements.contains(item.from.as_str())
            || !elements.contains(item.to.as_str())
            || (item.evidence_ids.is_empty() && item.resolution != DesignResolution::UserAssertion)
            || !label(&item.id, 128)
            || !label(&item.kind, 128)
            || !label(&item.direction, 128)
            || !label(&item.synchrony, 128)
            || item
                .protocol
                .as_ref()
                .is_some_and(|value| !label(value, 256))
            || item.unknowns.len() > 100
            || item.unknowns.iter().any(|value| !label(value, 1024))
            || !valid_ids(&item.evidence_ids)
        {
            return Err("Design relationship lacks scoped supporting evidence".into());
        }
    }
    let sections: BTreeSet<_> = version
        .sections
        .iter()
        .map(|section| section.id.as_str())
        .collect();
    let required = BTreeSet::from([
        "identity",
        "system_context",
        "components",
        "interactions",
        "data_ownership",
        "deployment",
        "constraints",
        "major_flows",
        "review_state",
    ]);
    if sections != required {
        return Err("Design section inventory is incomplete".into());
    }
    let relations = version
        .relations
        .iter()
        .map(|relation| relation.id.as_str())
        .collect::<BTreeSet<_>>();
    if relations.len() != version.relations.len() {
        return Err("Duplicate relationship identity".into());
    }
    for section in &version.sections {
        if !label(&section.title, 256)
            || section
                .element_ids
                .iter()
                .any(|id| !elements.contains(id.as_str()))
            || section
                .relation_ids
                .iter()
                .any(|id| !relations.contains(id.as_str()))
        {
            return Err("Design section references unscoped elements".into());
        }
    }
    for constraint in &version.constraints {
        if !label(&constraint.id, 128)
            || !label(&constraint.kind, 128)
            || !elements.contains(constraint.from_component_id.as_str())
            || constraint
                .to_component_id
                .as_ref()
                .is_some_and(|id| !elements.contains(id.as_str()))
            || !valid_ids(&constraint.evidence_ids)
            || constraint.value.len() > 4096
            || !label(&constraint.user_assertion, 4096)
        {
            return Err("Design constraint is outside the selected interpretation".into());
        }
    }
    for correspondence in &version.correspondence {
        if !matches!(
            correspondence.kind.as_str(),
            "rename" | "move" | "split" | "merge"
        ) || correspondence.resolution != DesignResolution::UserAssertion
            || correspondence.from_ids.is_empty()
            || correspondence.to_ids.is_empty()
            || correspondence.from_ids.len() > 100
            || correspondence.to_ids.len() > 100
            || correspondence.from_ids.iter().any(|id| !label(id, 128))
            || correspondence
                .to_ids
                .iter()
                .any(|id| !elements.contains(id.as_str()))
        {
            return Err("Design correspondence requires explicit bounded identities and selected target elements".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesignOutcome {
    ChangeDetected,
    NoChangeDetectedWithinScope,
    Incomplete,
    Unavailable,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignChange {
    pub id: String,
    pub kind: String,
    pub element_id: Option<String>,
    pub relation_id: Option<String>,
    pub before_name: Option<String>,
    pub after_name: Option<String>,
    pub evidence_ids: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignPossibleImpact {
    pub component_id: String,
    pub via_relation_ids: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignFinding {
    pub constraint_id: String,
    pub resolution: DesignResolution,
    pub message: String,
    pub evidence_ids: Vec<String>,
}
/// Source intent is never a proved architectural rule. Changed or unavailable
/// referenced documents and every document/code mismatch require verification.
pub fn evaluate_hld_decisions(
    proposal: &DesignVersion,
    accepted: &DesignVersion,
) -> Vec<DesignFinding> {
    accepted.decision_records.iter().take(100).map(|decision| {
        let old = decision.evidence_ids.first().and_then(|id|accepted.evidence.iter().find(|evidence|evidence.id == *id));
        let current = old.and_then(|old|proposal.document_candidates.iter()
            .filter_map(|candidate|proposal.evidence.iter().find(|evidence|evidence.id == candidate.evidence_id))
            .find(|evidence|evidence.path == old.path));
        let message = match (old,current) {
            (Some(old),Some(current)) if old.blob != current.blob => "The explicitly selected design document changed at this proposed commit. Its accepted intent may be stale; compare it with observed code before reusing the decision.",
            (_,None) => "The selected design document is not available in this bounded proposal inventory. Document availability and its intent/code correspondence require reviewer verification.",
            _ => "This document intent was explicitly supplied by a reviewer. Static analysis does not prove the code implements it; inspect the selected document and observed structure together.",
        };
        DesignFinding { constraint_id:decision.id.clone(),resolution:DesignResolution::Unknown,message:message.into(),
            evidence_ids:current.map(|evidence|vec![evidence.id.clone()]).unwrap_or_default() }
    }).collect()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesignDelta {
    pub id: String,
    pub baseline_design_id: String,
    pub proposal_design_id: String,
    pub baseline_commit: String,
    pub proposal_commit: String,
    pub outcome: DesignOutcome,
    pub changes: Vec<DesignChange>,
    pub possible_impacts: Vec<DesignPossibleImpact>,
    pub questions: Vec<String>,
    pub findings: Vec<DesignFinding>,
    pub coverage: DesignCoverage,
    pub mermaid: String,
}

#[cfg(test)]
mod tests;
