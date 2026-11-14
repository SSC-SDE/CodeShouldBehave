//! Content-addressed local summaries; a digest is not a signature or authorization.

use super::compatibility::ContractAssessment;
use super::decision::{DecisionPolicy, ReviewDecision, ReviewVerdict};
use super::evidence::EvidenceLedger;
use super::planning::ReviewPlan;
use sha2::{Digest, Sha256};
use std::fmt::{self, Write};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditSummary {
    pub changed_paths: usize,
    pub changed_lines: u64,
    pub boundary_changes: usize,
    pub risk_counts: [usize; 3],
    pub batches: usize,
    pub review_weight: u64,
    pub recorded_checks: usize,
    pub checked_paths: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    pub fingerprint: [u8; 32],
    pub summary: AuditSummary,
    pub verdict: ReviewVerdict,
}

pub fn summarize(plan: &ReviewPlan<'_>, ledger: &EvidenceLedger<'_>) -> AuditSummary {
    let inventory = ledger.inventory();
    let coverage = ledger.coverage();
    AuditSummary {
        changed_paths: inventory.changes().len(),
        changed_lines: inventory.total_lines(),
        boundary_changes: inventory.boundary_count(),
        risk_counts: inventory.risk_counts(),
        batches: plan.batches().len(),
        review_weight: plan.total_weight(),
        recorded_checks: coverage.recorded_checks,
        checked_paths: coverage.checked_paths,
    }
}

/// Versioned, length-delimited encoding includes inputs, supplied evidence and decisions.
/// The pipeline provides path-sorted contracts and observations for stable replay.
pub fn fingerprint(
    plan: &ReviewPlan<'_>,
    ledger: &EvidenceLedger<'_>,
    policy: DecisionPolicy,
    contracts: &[ContractAssessment<'_>],
    decision: &ReviewDecision<'_>,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    field(&mut hash, b"architecture-review-demo-v1");
    hash.update([policy.require_behavior as u8, policy.require_security_for_high_risk as u8]);
    hash.update((ledger.inventory().changes().len() as u64).to_le_bytes());
    for change in ledger.inventory().changes() {
        field(&mut hash, change.path().as_bytes());
        hash.update([change.kind as u8, change.risk as u8]);
        hash.update(change.added_lines.to_le_bytes());
        hash.update(change.removed_lines.to_le_bytes());
    }
    hash.update((plan.batches().len() as u64).to_le_bytes());
    for batch in plan.batches() {
        hash.update([batch.lane as u8]);
        hash.update((batch.tasks.len() as u64).to_le_bytes());
        for task in &batch.tasks {
            field(&mut hash, task.change.path().as_bytes());
            hash.update(task.weight.to_le_bytes());
        }
    }
    hash.update((ledger.coverage().recorded_checks as u64).to_le_bytes());
    for observation in ledger.observations() {
        field(&mut hash, observation.path.as_bytes());
        hash.update([observation.check as u8, observation.outcome as u8]);
        field(&mut hash, observation.source.as_bytes());
    }
    hash.update((contracts.len() as u64).to_le_bytes());
    for assessment in contracts {
        field(&mut hash, assessment.contract.path.as_bytes());
        // Each display value is separately hashed to keep framing unambiguous without strings.
        for value in [&assessment.contract.baseline as &dyn fmt::Display, &assessment.contract.candidate, &assessment.contract.consumers] {
            let mut encoded = HashWriter(Sha256::new());
            write!(encoded, "{value}").expect("hash writer cannot fail");
            hash.update(encoded.0.finalize());
        }
        hash.update([assessment.outcome as u8]);
    }
    hash.update([decision.verdict as u8]);
    hash.update((decision.issues.len() as u64).to_le_bytes());
    for issue in &decision.issues {
        field(&mut hash, issue.path.as_bytes());
        hash.update([issue.check as u8, issue.reason as u8]);
    }
    hash.finalize().into()
}

pub fn render_text(record: &AuditRecord) -> String {
    let mut text = String::new();
    writeln!(text, "Architecture review demo: {:?}", record.verdict).expect("string writer cannot fail");
    writeln!(text, "{} paths; {} changed lines; {} public boundaries", record.summary.changed_paths, record.summary.changed_lines, record.summary.boundary_changes).expect("string writer cannot fail");
    writeln!(text, "{} batches; {} weight; {} checks across {} paths", record.summary.batches, record.summary.review_weight, record.summary.recorded_checks, record.summary.checked_paths).expect("string writer cannot fail");
    text.push_str("SHA-256: ");
    for byte in record.fingerprint {
        write!(text, "{byte:02x}").expect("string writer cannot fail");
    }
    text.push_str("\nAdvisory only: no merge, publication, or consent is authorized.\n");
    text
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

struct HashWriter(Sha256);

impl Write for HashWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.0.update(value.as_bytes());
        Ok(())
    }
}
