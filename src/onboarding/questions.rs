//! Evidence-driven questions. Answers interpret observations and cannot add retrieval rights.
use super::*;
use std::collections::BTreeMap;

pub const QUESTION_VERSION: &str = "csb-onboarding-questions-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(rename_all = "snake_case"))]
pub enum IntegrationDecision {
    Pending,
    Include,
    Exclude,
    Defer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct IntegrationCandidate {
    pub id: String,
    pub component_id: String,
    pub kind: String,
    pub name: String,
    pub reason: String,
    pub evidence: Vec<OnboardingEvidence>,
    pub test_only: bool,
    pub decision: IntegrationDecision,
    pub answer: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
pub struct QuestionOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
pub struct OnboardingQuestion {
    pub id: String,
    pub fingerprint: String,
    pub category: String,
    pub component_id: String,
    pub provider_index: Option<usize>,
    pub integration_id: Option<String>,
    pub prompt: String,
    pub reason: String,
    pub evidence: Vec<OnboardingEvidence>,
    pub decision: String,
    pub options: Vec<QuestionOption>,
}

fn fingerprint(
    session: &OnboardingSession,
    identity: &str,
    evidence: &[OnboardingEvidence],
) -> String {
    let mut digest = Sha256::new();
    for field in [QUESTION_VERSION, &session.input_fingerprint(), identity] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    for item in evidence {
        digest.update((item.repository_index as u64).to_be_bytes());
        for field in [&item.path, &item.blob, &item.producer_version] {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field);
        }
        digest.update(item.start_line.to_be_bytes());
        digest.update(item.end_line.to_be_bytes());
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn integrations(session: &OnboardingSession) -> (Vec<IntegrationCandidate>, bool) {
    let mut candidates = Vec::new();
    let mut truncated = false;
    for component in &session.proposals {
        let mut grouped: BTreeMap<(&str, &str, bool), Vec<&ComponentSignal>> = BTreeMap::new();
        for signal in &component.signals {
            if matches!(
                signal.kind.as_str(),
                "database"
                    | "data client"
                    | "queue"
                    | "storage"
                    | "external contract"
                    | "internal contract"
                    | "interface contract"
                    | "schema artifact"
            ) {
                grouped
                    .entry((&signal.kind, &signal.name, signal.test_only))
                    .or_default()
                    .push(signal);
            }
        }
        for ((kind, name, test_only), signals) in grouped {
            if candidates.len() == 200 {
                truncated = true;
                continue;
            }
            let evidence = signals
                .iter()
                .map(|signal| component.evidence[signal.evidence_index].clone())
                .collect::<Vec<_>>();
            let identity = format!("{}:{kind}:{name}:{test_only}", component.id);
            candidates.push(IntegrationCandidate {
                id: format!("integration-{}", fingerprint(session, &identity, &evidence)),
                component_id: component.id.clone(),
                kind: kind.into(),
                name: name.into(),
                reason: signals[0].reason.clone(),
                evidence,
                test_only,
                decision: IntegrationDecision::Pending,
                answer: None,
            });
        }
    }
    (candidates, truncated)
}

fn component_decision(decision: ComponentDecision) -> &'static str {
    match decision {
        ComponentDecision::Pending => "pending",
        ComponentDecision::Confirmed => "confirmed",
        ComponentDecision::Corrected => "corrected",
        ComponentDecision::Rejected => "rejected",
        ComponentDecision::Deferred => "deferred",
    }
}
fn integration_decision(decision: IntegrationDecision) -> &'static str {
    match decision {
        IntegrationDecision::Pending => "pending",
        IntegrationDecision::Include => "include",
        IntegrationDecision::Exclude => "exclude",
        IntegrationDecision::Defer => "defer",
    }
}
fn options(values: &[(&str, &str)]) -> Vec<QuestionOption> {
    values
        .iter()
        .map(|(value, label)| QuestionOption {
            value: (*value).into(),
            label: (*label).into(),
        })
        .collect()
}
pub(super) fn plan(session: &OnboardingSession) -> Vec<OnboardingQuestion> {
    let mut questions = Vec::new();
    for component in &session.proposals {
        let identity = format!("component:{}", component.id);
        let fp = fingerprint(session, &identity, &component.evidence);
        questions.push(OnboardingQuestion {
            id: identity, fingerprint: fp, category: "component".into(), component_id: component.id.clone(), provider_index: None, integration_id: None,
            prompt: format!("Include {} as {} at the proposed roots?", component.name, component.role),
            reason: "This decision selects architecture boundaries and source paths; observed imports remain qualified evidence. No credentials or secret values are requested.".into(),
            evidence: component.evidence.clone(), decision: component_decision(component.decision).into(),
            options: options(&[("confirmed", "Confirm"), ("corrected", "Correct"), ("rejected", "Exclude"), ("deferred", "Defer")]),
        });
    }
    for (index, provider) in session.provider_mappings.iter().enumerate() {
        let identity = format!("provider:{}:{index}", provider.component_id);
        questions.push(OnboardingQuestion {
            id: identity.clone(), fingerprint: fingerprint(session, &identity, &provider.evidence), category: "provider".into(), component_id: provider.component_id.clone(), provider_index: Some(index), integration_id: None,
            prompt: format!("Does {} describe this component in {}?", provider.provider, provider.environment),
            reason: "Static hosting evidence is unverified. Confirm or correct the interpretation, exclude stale configuration, or defer; no provider is contacted.".into(),
            evidence: provider.evidence.clone(), decision: component_decision(provider.decision).into(),
            options: options(&[("confirmed", "Confirm label"), ("corrected", "Correct"), ("rejected", "Exclude"), ("deferred", "Defer")]),
        });
    }
    for integration in &session.integrations {
        questions.push(OnboardingQuestion {
            id: integration.id.clone(), fingerprint: fingerprint(session, &integration.id, &integration.evidence), category: "integration".into(), component_id: integration.component_id.clone(), provider_index: None, integration_id: Some(integration.id.clone()),
            prompt: format!("Include {} ({}) in the architecture context?", integration.name, integration.kind),
            reason: format!("{} Include retains only these approved source observations; external documentation and newly referenced repositories are not fetched.", integration.reason),
            evidence: integration.evidence.clone(), decision: integration_decision(integration.decision).into(),
            options: options(&[("include", "Include observed context"), ("exclude", "Exclude"), ("defer", "Defer")]),
        });
    }
    // 100 components + 200 mappings + 200 integration groups, validated at ingestion.
    questions
}
