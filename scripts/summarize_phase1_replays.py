#!/usr/bin/env python3
import json
import platform
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise SystemExit(f"FAIL: {message}")


if len(sys.argv) != 3:
    fail("usage: summarize_phase1_replays.py REPLAY_ROOT BROWSER_REPORT")

root = Path(sys.argv[1])
browser_path = Path(sys.argv[2])
runs = []
for index in range(1, 4):
    path = root / f"run-{index}" / "report.json"
    if not path.is_file():
        fail(f"missing {path}")
    runs.append(json.loads(path.read_text()))
browser = json.loads(browser_path.read_text())

required_true = [
    "duplicate_generation_rejected",
    "delayed_generation_rejected",
    "expired_lease_recovered",
    "provider_throttle_observed",
    "provider_credential_revocation_observed",
    "access_revocation_denied",
    "scope_change_fenced_old_job",
    "clean_rebuild_match",
]
budgets = {
    "event_to_ready_p95_ms": 300_000,
    "graph_query_p95_us": 300_000,
    "cold_analysis_ms": 30_000,
    "peak_sampled_rss_bytes": 512 * 1024 * 1024,
    "cpu_time_ms": 60_000,
    "disk_bytes_before_report": 1024 * 1024 * 1024,
    "sqlite_write_p95_us": 5_000_000,
    "total_elapsed_ms": 300_000,
}
for number, run in enumerate(runs, 1):
    if run.get("schema") != "codeshouldbehave-phase1-load-replay-v5":
        fail(f"run {number} has an unexpected schema")
    if run.get("event_to_ready_samples") != 100 or run.get("completed_current_jobs") != 100:
        fail(f"run {number} did not publish every current job")
    if any(run.get(field) is not True for field in required_true):
        fail(f"run {number} failed a correctness assertion")
    if any(run.get(field) != 0 for field in ("final_queued", "final_running", "final_failed")):
        fail(f"run {number} has unfinished or failed current work")
    for field, maximum in budgets.items():
        if not isinstance(run.get(field), (int, float)) or run[field] > maximum:
            fail(f"run {number} exceeds {field} budget {maximum}")

if browser.get("schema") != "codeshouldbehave-phase1-browser-replay-v1":
    fail("browser report has an unexpected schema")
if browser.get("status") != "complete" or browser.get("samples") != 100:
    fail("browser replay did not complete 100 samples")
if browser.get("sessions") != 25 or browser.get("rendered_spans") != 25:
    fail("browser replay did not render all 25 sessions")
if browser.get("p95_ms", 501) > 500:
    fail("browser source-preview p95 exceeds 500 ms")

summary = {
    "schema": "codeshouldbehave-phase1-acceptance-summary-v1",
    "status": "passed",
    "platform": platform.platform(),
    "python": platform.python_version(),
    "runs": 3,
    "budgets": budgets,
    "max_observed": {field: max(run[field] for run in runs) for field in budgets},
    "browser": browser,
}
output = root / "acceptance-summary.json"
output.write_text(json.dumps(summary, indent=2) + "\n")
print(f"PASS: wrote {output}")
