#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "Usage: scripts/phase1_operator_rehearsal.sh OPERATOR_ID NEW_OUTPUT_FILE" >&2
    exit 2
fi

operator=$1
output=$2
if [ -z "$operator" ] || [ "$operator" = "codex" ] || [ "$operator" = "implementer" ]; then
    echo "A distinct operator identity is required" >&2
    exit 2
fi
if [ -e "$output" ]; then
    echo "Refusing to overwrite existing output: $output" >&2
    exit 2
fi

cargo test --all-features operations::tests::reconciliation_capacity_disk_and_drain_gate_admission
cargo test --all-features scheduler::tests::duplicate_enqueue_expired_lease_and_restart_are_safe
cargo test --all-features storage::tests::isolated_restore_preserves_historical_review_scope_and_commits
cargo test --all-features service::tests::scope_change_persists_new_identity_and_supersedes_old_work
cargo test --all-features rollout::tests

{
    echo "schema=codeshouldbehave-phase1-operator-rehearsal-v1"
    echo "status=passed"
    echo "operator=$operator"
    echo "commit=$(git rev-parse HEAD)"
    echo "performed_at_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "procedures=startup-readiness,stuck-job,recover-worker,restore-backup,narrow-scope,disable-ingestion,rollback"
} > "$output"

echo "PASS: operator rehearsal written to $output"
