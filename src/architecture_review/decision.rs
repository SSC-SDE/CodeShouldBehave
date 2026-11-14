//! Advisory completeness decisions. Nothing here grants consent or permits merging.

use super::evidence::{CheckKind, CheckOutcome, EvidenceLedger};
use super::model::{Change, ChangeKind, Risk};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecisionPolicy {
    pub require_behavior: bool,
    pub require_security_for_high_risk: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewVerdict {
    ReadyForHumanReview,
    NeedsEvidence,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockReason {
    Missing,
    Inconclusive,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewIssue<'a> {
    pub path: &'a str,
    pub check: CheckKind,
    pub reason: BlockReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewDecision<'a> {
    pub verdict: ReviewVerdict,
    pub issues: Vec<ReviewIssue<'a>>,
}

pub fn required_checks(change: &Change, policy: DecisionPolicy) -> impl Iterator<Item = CheckKind> {
    [
        Some(CheckKind::Syntax),
        policy.require_behavior.then_some(CheckKind::Behavior),
        change.is_public_boundary().then_some(CheckKind::Compatibility),
        (change.kind == ChangeKind::Dependency
            || (change.risk == Risk::High && policy.require_security_for_high_risk))
            .then_some(CheckKind::Security),
        (change.kind == ChangeKind::DataLayout).then_some(CheckKind::Migration),
    ].into_iter().flatten()
}

pub fn evaluate<'a>(ledger: &EvidenceLedger<'a>, policy: DecisionPolicy) -> ReviewDecision<'a> {
    let mut issues = Vec::new();
    for change in ledger.inventory().changes() {
        for check in required_checks(change, policy) {
            let reason = match ledger.get(change.path(), check).map(|item| item.outcome) {
                Some(CheckOutcome::Passed) => continue,
                Some(CheckOutcome::Failed) => BlockReason::Failed,
                Some(CheckOutcome::Inconclusive) => BlockReason::Inconclusive,
                None => BlockReason::Missing,
            };
            issues.push(ReviewIssue { path: change.path(), check, reason });
        }
        // Optional failures cannot disappear just because policy doesn't require that check.
        for check in [CheckKind::Syntax, CheckKind::Behavior, CheckKind::Compatibility, CheckKind::Security, CheckKind::Migration] {
            if !required_checks(change, policy).any(|required| required == check)
                && ledger.get(change.path(), check).is_some_and(|item| item.outcome == CheckOutcome::Failed)
            {
                issues.push(ReviewIssue { path: change.path(), check, reason: BlockReason::Failed });
            }
        }
    }
    let verdict = if issues.iter().any(|issue| issue.reason == BlockReason::Failed) {
        ReviewVerdict::Blocked
    } else if issues.is_empty() {
        ReviewVerdict::ReadyForHumanReview
    } else {
        ReviewVerdict::NeedsEvidence
    };
    ReviewDecision { verdict, issues }
}

pub fn uncovered_paths<'a>(ledger: &EvidenceLedger<'a>, policy: DecisionPolicy) -> Vec<&'a str> {
    ledger.inventory().changes().iter().filter(|change| {
        required_checks(change, policy).any(|check| ledger.get(change.path(), check).is_none())
    }).map(|change| change.path()).collect()
}
