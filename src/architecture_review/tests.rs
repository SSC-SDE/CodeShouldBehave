use super::*;
use compatibility::{Compatibility, CompatibilityError, VersionedContract};
use decision::{BlockReason, required_checks, uncovered_paths};
use evidence::{CheckKind, CheckOutcome};
use intake::{IntakeError, IntakeLimits};
use model::{Change, ChangeError, ChangeKind, Risk};
use planning::Lane;

fn change(path: &str, kind: ChangeKind, risk: Risk) -> Change {
    Change::new(path.into(), kind, risk, 5, 3).unwrap()
}

fn inventory(changes: Vec<Change>) -> Inventory {
    Inventory::collect(changes, IntakeLimits { max_changes: 20, max_changed_lines: 1000 }).unwrap()
}

fn limits() -> PlanLimits {
    PlanLimits { max_tasks_per_batch: 2, max_weight_per_batch: 1000, max_batches: 20 }
}

fn policy() -> DecisionPolicy {
    DecisionPolicy { require_behavior: true, require_security_for_high_risk: true }
}

fn run<'a>(inventory: &'a Inventory, evidence: &'a [Evidence<'a>], contracts: &'a [VersionedContract<'a>]) -> ReviewResult<'a> {
    review(ReviewRequest { inventory, evidence, contracts, plan_limits: limits(), decision_policy: policy() }).unwrap()
}

#[test]
fn rejects_noncanonical_paths_and_empty_changes() {
    for path in ["", "/src/a.rs", "src//a.rs", "../a.rs", "./a.rs", "src/../a.rs", "src\\a.rs", "C:/a.rs", "src/a\n.rs"] {
        assert_eq!(Change::new(path.into(), ChangeKind::Implementation, Risk::Low, 1, 0), Err(ChangeError::InvalidPath));
    }
    assert_eq!(Change::new("src/a.rs".into(), ChangeKind::Implementation, Risk::Low, 0, 0), Err(ChangeError::EmptyChange));
    assert!(Change::new("src/日本.rs".into(), ChangeKind::Implementation, Risk::Low, 0, 1).is_ok());
}

#[test]
fn weights_boundaries_and_risk_without_u32_overflow() {
    let plain = change("src/a.rs", ChangeKind::Implementation, Risk::Low);
    let boundary = change("src/b.rs", ChangeKind::PublicContract, Risk::High);
    assert_eq!(plain.review_weight(), 8);
    assert_eq!(boundary.review_weight(), 64);
    assert!(!plain.is_public_boundary());
    assert!(boundary.is_public_boundary());
    let huge = Change::new("src/huge.rs".into(), ChangeKind::DataLayout, Risk::High, u32::MAX, u32::MAX).unwrap();
    assert_eq!(huge.review_weight(), u64::from(u32::MAX) * 16);
}

#[test]
fn intake_sorts_counts_and_rejects_duplicates() {
    let a = change("src/a.rs", ChangeKind::Implementation, Risk::Low);
    let b = change("src/b.rs", ChangeKind::Dependency, Risk::High);
    let result = inventory(vec![b, a.clone()]);
    assert_eq!(result.changes()[0].path(), "src/a.rs");
    assert_eq!(result.total_lines(), 16);
    assert_eq!(result.boundary_count(), 1);
    assert_eq!(result.risk_counts(), [1, 0, 1]);
    assert_eq!(Inventory::collect(vec![a.clone(), a], IntakeLimits { max_changes: 2, max_changed_lines: 100 }), Err(IntakeError::DuplicatePath("src/a.rs".into())));
}

#[test]
fn intake_stops_unbounded_sources_and_enforces_line_budget() {
    let a = change("src/a.rs", ChangeKind::Implementation, Risk::Low);
    assert_eq!(Inventory::collect(std::iter::repeat(a.clone()), IntakeLimits { max_changes: 2, max_changed_lines: 100 }), Err(IntakeError::TooManyChanges));
    assert_eq!(Inventory::collect(vec![a.clone()], IntakeLimits { max_changes: 2, max_changed_lines: 7 }), Err(IntakeError::TooManyLines));
    assert_eq!(Inventory::collect(Vec::new(), IntakeLimits { max_changes: 2, max_changed_lines: 100 }), Err(IntakeError::EmptyInventory));
    let mut empty = a;
    empty.added_lines = 0;
    empty.removed_lines = 0;
    assert_eq!(Inventory::collect(vec![empty], IntakeLimits { max_changes: 2, max_changed_lines: 100 }), Err(IntakeError::EmptyChange("src/a.rs".into())));
}

#[test]
fn planning_separates_lanes_and_respects_both_batch_limits() {
    let inventory = inventory(vec![
        change("src/c.rs", ChangeKind::PublicContract, Risk::Medium),
        change("src/b.rs", ChangeKind::Implementation, Risk::Low),
        change("src/a.rs", ChangeKind::Implementation, Risk::Low),
    ]);
    let plan = ReviewPlan::build(&inventory, limits()).unwrap();
    assert_eq!(plan.task_count(), 3);
    assert_eq!(plan.total_weight(), 48);
    assert_eq!(plan.batches().len(), 2);
    assert_eq!(plan.batches()[0].lane, Lane::Implementation);
    assert_eq!(plan.batches()[0].tasks.len(), 2);
    assert_eq!(plan.batches()[1].lane, Lane::Contract);
    let split = ReviewPlan::build(&inventory, PlanLimits { max_tasks_per_batch: 1, ..limits() }).unwrap();
    assert_eq!(split.batches().len(), 3);
    assert_eq!(ReviewPlan::build(&inventory, PlanLimits { max_weight_per_batch: 7, ..limits() }), Err(PlanError::ChangeTooHeavy("src/a.rs".into())));
    assert_eq!(ReviewPlan::build(&inventory, PlanLimits { max_batches: 1, ..limits() }), Err(PlanError::TooManyBatches));
    assert_eq!(ReviewPlan::build(&inventory, PlanLimits { max_tasks_per_batch: 0, ..limits() }), Err(PlanError::ZeroLimit));
}

#[test]
fn weight_limit_splits_same_lane_without_dropping_tasks() {
    let inventory = inventory(vec![change("a", ChangeKind::Implementation, Risk::Low), change("b", ChangeKind::Implementation, Risk::Low)]);
    let plan = ReviewPlan::build(&inventory, PlanLimits { max_weight_per_batch: 8, ..limits() }).unwrap();
    assert_eq!(plan.batches().len(), 2);
    assert_eq!(plan.task_count(), 2);
    assert!(plan.batches().iter().all(|batch| batch.weight == 8));
}

#[test]
fn ledger_rejects_unknown_duplicate_and_unusable_evidence() {
    let inventory = inventory(vec![change("src/a.rs", ChangeKind::Implementation, Risk::Low)]);
    let good = Evidence { path: "src/a.rs", check: CheckKind::Syntax, outcome: CheckOutcome::Passed, source: "local compiler output" };
    let unknown = Evidence { path: "src/missing.rs", ..good.clone() };
    let invalid = Evidence { source: "\n", ..good.clone() };
    let mut ledger = EvidenceLedger::for_inventory(&inventory);
    assert_eq!(ledger.record(&unknown), Err(EvidenceError::UnknownPath));
    assert_eq!(ledger.record(&invalid), Err(EvidenceError::InvalidSource));
    ledger.record(&good).unwrap();
    assert_eq!(ledger.record(&good), Err(EvidenceError::DuplicateCheck));
    assert_eq!(ledger.coverage(), evidence::EvidenceCoverage { recorded_checks: 1, checked_paths: 1, total_paths: 1 });
}

#[test]
fn missing_and_inconclusive_checks_never_become_ready() {
    let inventory = inventory(vec![change("src/a.rs", ChangeKind::Implementation, Risk::Low)]);
    let empty = run(&inventory, &[], &[]);
    assert_eq!(empty.decision.verdict, ReviewVerdict::NeedsEvidence);
    assert_eq!(empty.decision.issues.len(), 2);
    let observation = Evidence { path: "src/a.rs", check: CheckKind::Syntax, outcome: CheckOutcome::Inconclusive, source: "compiler interrupted" };
    let mut ledger = EvidenceLedger::for_inventory(&inventory);
    ledger.record(&observation).unwrap();
    assert_eq!(uncovered_paths(&ledger, policy()), vec!["src/a.rs"]);
    let result = evaluate(&ledger, policy());
    assert!(result.issues.iter().any(|issue| issue.reason == BlockReason::Inconclusive));
    assert_eq!(result.verdict, ReviewVerdict::NeedsEvidence);
}

#[test]
fn optional_failed_check_still_blocks() {
    let inventory = inventory(vec![change("src/a.rs", ChangeKind::Implementation, Risk::Low)]);
    let evidence = [
        Evidence { path: "src/a.rs", check: CheckKind::Syntax, outcome: CheckOutcome::Passed, source: "syntax report" },
        Evidence { path: "src/a.rs", check: CheckKind::Behavior, outcome: CheckOutcome::Passed, source: "behavior report" },
        Evidence { path: "src/a.rs", check: CheckKind::Security, outcome: CheckOutcome::Failed, source: "security report" },
    ];
    let result = run(&inventory, &evidence, &[]);
    assert_eq!(result.decision.verdict, ReviewVerdict::Blocked);
    assert_eq!(result.decision.issues.len(), 1);
    assert_eq!(result.decision.issues[0].check, CheckKind::Security);
}

#[test]
fn required_checks_follow_architectural_responsibility() {
    let data = change("src/schema.rs", ChangeKind::DataLayout, Risk::High);
    assert_eq!(required_checks(&data, policy()).collect::<Vec<_>>(), vec![CheckKind::Syntax, CheckKind::Behavior, CheckKind::Compatibility, CheckKind::Security, CheckKind::Migration]);
    let dependency = change("Cargo.toml", ChangeKind::Dependency, Risk::Low);
    let relaxed = DecisionPolicy { require_behavior: false, require_security_for_high_risk: false };
    assert_eq!(required_checks(&dependency, relaxed).collect::<Vec<_>>(), vec![CheckKind::Syntax, CheckKind::Compatibility, CheckKind::Security]);
}

#[test]
fn version_review_handles_breaking_downgrade_requirements_and_prereleases() {
    for (baseline, candidate, consumers, expected) in [
        ("1.2.0", "1.3.0", "^1", Compatibility::Compatible),
        ("1.2.0", "1.1.9", "*", Compatibility::Downgrade),
        ("1.2.0", "2.0.0", "^1", Compatibility::OutsideConsumerRequirement),
        ("1.2.0", "2.0.0", ">=1", Compatibility::BreakingRelease),
        ("0.2.0", "0.3.0", ">=0", Compatibility::BreakingRelease),
        ("0.0.1", "0.0.2", ">=0", Compatibility::BreakingRelease),
        ("1.2.0", "1.3.0-alpha.1", "^1", Compatibility::OutsideConsumerRequirement),
        ("1.2.0+z", "1.2.0+a", "^1", Compatibility::Compatible),
    ] {
        assert_eq!(VersionedContract::parse("src/api.rs", baseline, candidate, consumers).unwrap().assess(), expected);
    }
    assert!(VersionedContract::parse("src/api.rs", "nonsense", "1.0.0", "*").is_err());
    assert!(VersionedContract::parse("src/api.rs", "1.0.0", "1.0.1", "not a requirement").is_err());
}

#[test]
fn contracts_must_bind_unique_known_public_boundaries() {
    let inventory = inventory(vec![change("src/api.rs", ChangeKind::PublicContract, Risk::Low), change("src/body.rs", ChangeKind::Implementation, Risk::Low)]);
    let contract = VersionedContract::parse("src/api.rs", "1.0.0", "1.0.1", "^1").unwrap();
    assert_eq!(compatibility::assess_contracts(&inventory, &[contract.clone(), contract.clone()]), Err(CompatibilityError::DuplicatePath));
    assert_eq!(compatibility::assess_contracts(&inventory, &[VersionedContract { path: "src/body.rs", ..contract.clone() }]), Err(CompatibilityError::NotPublicBoundary));
    assert_eq!(compatibility::assess_contracts(&inventory, &[VersionedContract { path: "src/missing.rs", ..contract }]), Err(CompatibilityError::UnknownPath));
}

#[test]
fn end_to_end_ready_is_only_advisory_and_audit_is_order_independent() {
    let inventory = inventory(vec![change("src/api.rs", ChangeKind::PublicContract, Risk::Low)]);
    let mut evidence = [
        Evidence { path: "src/api.rs", check: CheckKind::Syntax, outcome: CheckOutcome::Passed, source: "syntax report" },
        Evidence { path: "src/api.rs", check: CheckKind::Behavior, outcome: CheckOutcome::Passed, source: "behavior report" },
        Evidence { path: "src/api.rs", check: CheckKind::Compatibility, outcome: CheckOutcome::Passed, source: "contract report" },
    ];
    let contracts = [VersionedContract::parse("src/api.rs", "1.0.0", "1.1.0", "^1").unwrap()];
    let first = run(&inventory, &evidence, &contracts);
    assert_eq!(first.decision.verdict, ReviewVerdict::ReadyForHumanReview);
    assert_eq!(first.audit.summary.boundary_changes, 1);
    assert_eq!(first.audit.summary.recorded_checks, 3);
    let fingerprint = first.audit.fingerprint;
    let text = audit::render_text(&first.audit);
    assert!(text.contains("Advisory only"));
    assert!(text.contains("1 paths; 8 changed lines; 1 public boundaries"));
    evidence.reverse();
    assert_eq!(run(&inventory, &evidence, &contracts).audit.fingerprint, fingerprint);
    evidence[0].source = "different contract report";
    assert_ne!(run(&inventory, &evidence, &contracts).audit.fingerprint, fingerprint);
}

#[test]
fn incompatible_versions_block_even_with_passing_check_assertions() {
    let inventory = inventory(vec![change("src/api.rs", ChangeKind::PublicContract, Risk::Low)]);
    let evidence = [CheckKind::Syntax, CheckKind::Behavior, CheckKind::Compatibility].map(|check| Evidence { path: "src/api.rs", check, outcome: CheckOutcome::Passed, source: "supplied report" });
    let contracts = [VersionedContract::parse("src/api.rs", "1.0.0", "2.0.0", ">=1").unwrap()];
    let result = run(&inventory, &evidence, &contracts);
    assert_eq!(result.decision.verdict, ReviewVerdict::Blocked);
    assert_eq!(result.audit.verdict, ReviewVerdict::Blocked);
    assert_eq!(result.contracts[0].outcome, Compatibility::BreakingRelease);
}

#[test]
fn audit_changes_with_policy_and_batch_boundaries() {
    let inventory = inventory(vec![change("src/a.rs", ChangeKind::Implementation, Risk::Low), change("src/b.rs", ChangeKind::Implementation, Risk::Low)]);
    let original = run(&inventory, &[], &[]).audit.fingerprint;
    let policy_changed = review(ReviewRequest { inventory: &inventory, evidence: &[], contracts: &[], plan_limits: limits(), decision_policy: DecisionPolicy { require_behavior: false, ..policy() } }).unwrap();
    let batches_changed = review(ReviewRequest { inventory: &inventory, evidence: &[], contracts: &[], plan_limits: PlanLimits { max_tasks_per_batch: 1, ..limits() }, decision_policy: policy() }).unwrap();
    assert_ne!(policy_changed.audit.fingerprint, original);
    assert_ne!(batches_changed.audit.fingerprint, original);
}
