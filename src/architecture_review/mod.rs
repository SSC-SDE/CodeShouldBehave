//! Opt-in architecture review demonstration for a draft review, not a production gate.
//!
//! Changes flow through validated intake, weighted planning, evidence admission,
//! semantic-version compatibility, advisory decision, and deterministic audit modules.
//! Callers supply all inputs and limits. This API performs no I/O, launches no jobs,
//! changes no runtime policy, and never authorizes publishing, merging, or consent.
//! Public boundaries and the semver dependency are deliberate source-backed HLD changes.

pub mod audit;
pub mod compatibility;
pub mod decision;
pub mod evidence;
pub mod intake;
pub mod model;
pub mod planning;

use self::audit::{AuditRecord, fingerprint, summarize};
use self::compatibility::{Compatibility, CompatibilityError, ContractAssessment, VersionedContract, assess_contracts};
use self::decision::{DecisionPolicy, ReviewDecision, ReviewVerdict, evaluate};
use self::evidence::{Evidence, EvidenceError, EvidenceLedger};
use self::intake::Inventory;
use self::planning::{PlanError, PlanLimits, ReviewPlan};

pub struct ReviewRequest<'a> {
    pub inventory: &'a Inventory,
    pub evidence: &'a [Evidence<'a>],
    pub contracts: &'a [VersionedContract<'a>],
    pub plan_limits: PlanLimits,
    pub decision_policy: DecisionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewResult<'a> {
    pub plan: ReviewPlan<'a>,
    pub decision: ReviewDecision<'a>,
    pub contracts: Vec<ContractAssessment<'a>>,
    pub audit: AuditRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewError {
    Planning(PlanError),
    Evidence(EvidenceError),
    Compatibility(CompatibilityError),
}

/// Runs only the local, deterministic pipeline; supplied check outcomes remain assertions.
pub fn review(request: ReviewRequest<'_>) -> Result<ReviewResult<'_>, ReviewError> {
    let plan = ReviewPlan::build(request.inventory, request.plan_limits).map_err(ReviewError::Planning)?;
    let mut ledger = EvidenceLedger::for_inventory(request.inventory);
    for observation in request.evidence {
        ledger.record(observation).map_err(ReviewError::Evidence)?;
    }
    let contracts = assess_contracts(request.inventory, request.contracts).map_err(ReviewError::Compatibility)?;
    let mut decision = evaluate(&ledger, request.decision_policy);
    if contracts.iter().any(|assessment| assessment.outcome != Compatibility::Compatible) {
        decision.verdict = ReviewVerdict::Blocked;
    }
    let audit = AuditRecord {
        fingerprint: fingerprint(&plan, &ledger, request.decision_policy, &contracts, &decision),
        summary: summarize(&plan, &ledger),
        verdict: decision.verdict,
    };
    Ok(ReviewResult { plan, decision, contracts, audit })
}

#[cfg(test)]
mod tests;
