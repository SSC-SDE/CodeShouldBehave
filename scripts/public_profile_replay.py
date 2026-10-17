#!/usr/bin/env python3
"""Deterministic OFFLINE public-ci queue replay; never calls a provider or engine.

The real public_ci plan/finish_head state machine runs against synthetic provider
observations and injected outcomes. Durations are simulated values, not measured
GitHub latency, Rust analysis speed, browser performance or hosted capacity.
"""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timedelta, timezone
import hashlib
import heapq
import importlib.util
import json
from pathlib import Path
import random
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("csb_public_profile_control", ROOT / "scripts/public_ci.py")
ci = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ci
SPEC.loader.exec_module(ci)
SEEDS = (1729, 9648, 17567)
START = datetime(2026, 1, 1, tzinfo=timezone.utc)
ARRIVAL_SECONDS = 8 * 60 * 60
TICK_SECONDS = 60
ADMISSION_LIMIT = 3
SCHEMA = "csb-public-profile-offline-replay-v1"


class ReplayFailure(AssertionError):
    """An oracle/state mismatch blocks this scheduler evidence."""


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ReplayFailure(message)


def stamp(seconds):
    return (START + timedelta(seconds=seconds)).isoformat(timespec="seconds").replace("+00:00", "Z")


def configuration():
    """Strict example copy, deliberately unusable as live provider authority."""
    value = ci.read_json(ROOT / "config/public-ci.example.json", ci.MAX_CONFIG_BYTES)
    value["repository"] = {"id": 424242, "owner": "fixture", "name": "public-ci-replay"}
    value["site"] = {"origin": "https://fixture.github.io", "base_path": "/public-ci-replay/"}
    value["receipt"]["signer_workflow"] = "fixture/public-ci-replay/" + value["workflow_path"]
    value["budget"] = {"proof_mode": "unavailable", "account": "fixture",
                       "repository_byte_limit": ci.MAX_REPOSITORY_ARTIFACT_BYTES}
    for field in ("workflow_sha256", "policy_sha256", "context_sha256"):
        value[field] = digest(("SYNTHETIC-" + field).encode())
    value["engine"] = {"commit": digest(b"SYNTHETIC-engine")[:40],
                       "flowdiff_sha256": digest(b"SYNTHETIC-flowdiff"),
                       "exporter_sha256": digest(b"SYNTHETIC-exporter")}
    value["operations"] = {"analysis": False, "publication": False, "checks": False}
    return ci.load_config(value)


def comparison(config, observation):
    """Independent oracle implementation of the documented identity contract."""
    return digest(canonical({"repository": config["repository"], "pr_number": observation["number"],
                             "selected_baseline": observation["selected_baseline"],
                             "proposal": observation["proposal"], "policy_sha256": config["policy_sha256"],
                             "context_sha256": config["context_sha256"], "engine": config["engine"]}))


def corpus(seed, count=100, revisions=5, burst_count=20, arrival_seconds=ARRIVAL_SECONDS):
    require(1 <= count <= 100 and revisions >= 1 and 0 <= burst_count <= count, "Invalid synthetic corpus bounds")
    require(arrival_seconds >= revisions, "Arrival windows must be nonempty")
    randomizer = random.Random(seed)
    events = []
    for revision in range(1, revisions + 1):
        low = ((revision - 1) * arrival_seconds) // revisions
        high = (revision * arrival_seconds) // revisions
        for number in range(1, count + 1):
            seconds = randomizer.randrange(low, high)
            events.append({"at": seconds, "pr_number": number, "revision": revision, "phase": "arrival",
                           "proposal": digest(f"SYNTHETIC:{seed}:{number}:{revision}".encode())[:40],
                           "selected_baseline": digest(f"SYNTHETIC-base:{int(number % 9 == 0 and revision >= 3)}".encode())[:40],
                           "draft": (number + revision) % 5 == 0})
    for number in randomizer.sample(range(1, count + 1), burst_count):
        events.append({"at": arrival_seconds + TICK_SECONDS, "pr_number": number,
                       "revision": revisions + 1, "phase": "burst",
                       "proposal": digest(f"SYNTHETIC:{seed}:{number}:{revisions + 1}".encode())[:40],
                       "selected_baseline": digest(f"SYNTHETIC-base:{int(number % 9 == 0 and revisions + 1 >= 3)}".encode())[:40],
                       "draft": (number + revisions + 1) % 5 == 0})
    return sorted(events, key=lambda item: (item["at"], item["phase"], item["pr_number"]))


def observation(config, event):
    return {"number": event["pr_number"], "repository": config["repository"],
            "selected_baseline": event["selected_baseline"], "proposal": event["proposal"],
            "state": "open", "draft": event["draft"], "public": True, "consent": True}


def recipe(seed, number, revision):
    code = seed + number * 11 + revision * 7
    return {"persistent_failure": code % 43 == 0, "transient_once": code % 17 == 0,
            "deferred_once": code % 19 == 0, "incomplete": code % 7 == 0}


def expected_terminal(seed, number, revision):
    outcome = recipe(seed, number, revision)
    return "failure" if outcome["persistent_failure"] else "unsupported" if outcome["incomplete"] else "published"


def assert_converged(config, current, state, seed, revisions_by_key):
    require(not state["pending"], "Current queue did not drain")
    actual = {record["pr_number"]: record for record in state["completed"]}
    require(set(actual) == set(current), "Missing/extra current terminal PRs")
    for number, item in current.items():
        key = comparison(config, item)
        require(actual[number]["comparison_key"] == key, "Completed cache holds a stale baseline/head")
        expected = expected_terminal(seed, number, revisions_by_key[key])
        require(actual[number]["outcome"] == expected, "Injected incomplete/failure became a clean current result")


def stale_completion_probe(config):
    first = {"at": 0, "pr_number": 1, "revision": 1, "phase": "probe", "proposal": "1" * 40,
             "selected_baseline": "a" * 40, "draft": False}
    state = ci.plan(config, ci.empty_state(config), [observation(config, first)], stamp(0))["state"]
    changed = dict(first, proposal="2" * 40)
    state = ci.plan(config, state, [observation(config, changed)], stamp(1))["state"]
    before = canonical(state)
    try:
        ci.finish_head(state, 1, first["proposal"], "published")
    except ci.Refusal:
        require(canonical(state) == before, "Rejected stale completion mutated state")
        return {"old_completion_rejected": True, "state_unchanged": True}
    raise ReplayFailure("An old head completion overwrote its replacement")


def run_seed(seed, *, count=100, revisions=5, burst_count=20, arrival_seconds=ARRIVAL_SECONDS):
    config = configuration()
    events = corpus(seed, count, revisions, burst_count, arrival_seconds)
    state = ci.empty_state(config)
    current, revisions_by_key, births, attempts, terminals = {}, {}, {}, Counter(), {}
    raw_passes, raw_actions = [], []
    pressures, intermediate = Counter(), Counter()
    event_position = 0
    max_pending = max_state_bytes = max_reservation = max_parallel_observed = 0
    tick = pass_number = 0
    last_arrival = events[-1]["at"]
    final_drain_passes = 0

    def observe_until(at):
        nonlocal event_position
        while event_position < len(events) and events[event_position]["at"] <= at:
            event = events[event_position]
            item = observation(config, event)
            current[item["number"]] = item
            key = comparison(config, item)
            revisions_by_key[key] = event["revision"]
            births[key] = event["at"]
            event_position += 1

    while True:
        require(pass_number < 2500, "Offline queue exceeded its bounded replay passes")
        observe_until(tick)
        if event_position == len(events) and not state["pending"]:
            actual = {item["pr_number"]: item["comparison_key"] for item in state["completed"]}
            if all(actual.get(number) == comparison(config, item) for number, item in current.items()):
                break
        pass_number += 1
        if tick >= last_arrival:
            final_drain_passes += 1
        rate_remaining, handoff_bytes, pressure = 5000, 0, "none"
        if pass_number % 13 == 4:
            rate_remaining, pressure = 0, "provider_rate_floor"
        elif pass_number % 17 == 8:
            handoff_bytes, pressure = 32 * ci.MIB, "handoff_full"
        elif pass_number % 19 == 5:
            handoff_bytes, pressure = 25 * ci.MIB, "handoff_reduced"
        pressures[pressure] += 1
        observations = [current[number] for number in sorted(current)]
        before = state
        result = ci.plan(config, before, observations, stamp(tick),
                         existing_handoff_bytes=handoff_bytes, rate_remaining=rate_remaining,
                         admission_limit=ADMISSION_LIMIT)
        state = result["state"]
        require(result["max_parallel"] == 2, "Unexpected model parallelism")
        require(len(result["admitted"]) <= ADMISSION_LIMIT and len(state["pending"]) <= 100, "Queue/admission bound exceeded")
        require(result["handoff_reservation_bytes"] + handoff_bytes <= 32 * ci.MIB, "Handoff reservation exceeded")
        if pressure in ("provider_rate_floor", "handoff_full"):
            require(not result["admitted"], "Admission ignored injected rate/storage exhaustion")
        old = {item["pr_number"]: item for item in before["pending"]}
        for pending in state["pending"]:
            if pending["pr_number"] in old:
                require(pending["queue_sequence"] == old[pending["pr_number"]]["queue_sequence"],
                        "A replacement head lost its existing fair queue position")
        selected = sorted((item for item in state["pending"] if item["status"] != "failed"), key=lambda item: item["queue_sequence"])
        require([item["pr_number"] for item in result["admitted"]] == [item["pr_number"] for item in selected[:len(result["admitted"])]],
                "Later work bypassed an earlier eligible queue position")
        require(canonical(before) != canonical(state), "Plan did not advance durable observations")
        for item in result["admitted"]:
            require(item["comparison_key"] == comparison(config, current[item["pr_number"]]), "Admission used wrong current input identity")
        max_pending = max(max_pending, len(state["pending"]))
        max_state_bytes = max(max_state_bytes, len(canonical(state)))
        max_reservation = max(max_reservation, result["handoff_reservation_bytes"])
        row = {"pass": pass_number, "at": tick, "pressure": pressure,
               "rate_remaining": rate_remaining, "existing_handoff_bytes": handoff_bytes,
               "reservation_bytes": result["handoff_reservation_bytes"], "pending": len(state["pending"]),
               "deferred": result["deferred"], "admitted": [item["pr_number"] for item in result["admitted"]],
               "queue": [{"pr": item["pr_number"], "sequence": item["queue_sequence"], "status": item["status"]} for item in state["pending"]]}
        raw_passes.append(row)
        slots = [(tick, 0), (tick, 1)]
        heapq.heapify(slots)
        completions = []
        for ordinal, item in enumerate(result["admitted"]):
            begin, slot = heapq.heappop(slots)
            revision = revisions_by_key[item["comparison_key"]]
            duration = 3 + (seed + item["pr_number"] * 3 + revision * 5) % 17
            end = begin + duration
            heapq.heappush(slots, (end, slot))
            completions.append((end, ordinal, begin, slot, item))
        active_points = []
        for end, _, begin, _, _ in completions:
            active_points.extend([(begin, 1), (end, -1)])
        active = 0
        for _, delta in sorted(active_points, key=lambda p: (p[0], p[1])):
            active += delta
            max_parallel_observed = max(max_parallel_observed, active)
            require(active <= 2, "Synthetic job assignment exceeded two slots")
        for end, _, begin, slot, item in sorted(completions):
            observe_until(end)
            number, key = item["pr_number"], item["comparison_key"]
            attempts[key] += 1
            rules = recipe(seed, number, revisions_by_key[key])
            if comparison(config, current[number]) != key:
                injected = "cancelled"
            elif rules["persistent_failure"] or (rules["transient_once"] and attempts[key] == 1):
                injected = "transient"
            elif rules["deferred_once"] and attempts[key] == 1:
                injected = "deferred"
            else:
                injected = "unsupported" if rules["incomplete"] else "published"
            state = ci.finish_head(state, number, item["proposal"], injected, "synthetic_offline_" + injected)
            intermediate[injected] += 1
            actual = next((record for record in state["completed"] if record["pr_number"] == number and record["comparison_key"] == key), None)
            terminal = actual["outcome"] if actual else None
            if terminal:
                require(key not in terminals, "A terminal comparison was executed twice")
                terminals[key] = terminal
            require(attempts[key] <= 3, "Retry/deferred attempts exceeded the model bound")
            raw_actions.append({"pr": number, "comparison_key": key, "proposal": item["proposal"],
                                "revision": revisions_by_key[key], "attempt": attempts[key], "slot": slot,
                                "simulated_start": begin, "simulated_finish": end,
                                "simulated_queue_delay": begin - births[key], "injected_outcome": injected,
                                "terminal_outcome": terminal})
        ci.validate_state(state, config)
        tick = max(tick + TICK_SECONDS, max((end for end, *_ in completions), default=tick) + 1)

    assert_converged(config, current, state, seed, revisions_by_key)
    require(max_state_bytes <= ci.MAX_STATE_BYTES, "Durable state exceeded configured bound")
    require(final_drain_passes <= 100, "Stable final heads did not converge within the finite fairness window")
    current_outcomes = Counter(item["outcome"] for item in state["completed"])
    superseded = len(events) - len(terminals)
    require(superseded >= 0, "Terminal count exceeds accepted source heads")
    final_keys = {comparison(config, value) for value in current.values()}
    require(final_keys <= terminals.keys(), "A latest source head disappeared without a terminal result")
    require(max_pending >= burst_count, "The separate burst did not exercise its simultaneous queue inventory")
    queue_delays = [item["simulated_queue_delay"] for item in raw_actions]
    job_durations = [item["simulated_finish"] - item["simulated_start"] for item in raw_actions]
    return {"seed": seed, "status": "passed_offline_queue_model", "synthetic_prs": count,
            "arrival_revisions": count * revisions, "burst_updates": burst_count,
            "source_heads": len(events), "source_events_sha256": digest(canonical(events)),
            "simulated_arrival_window_seconds": arrival_seconds, "simulated_controller_tick_seconds": TICK_SECONDS,
            "runtime_admission_limit": ADMISSION_LIMIT,
            "simulated_end_seconds": tick, "final_drain_passes": final_drain_passes,
            "passes": pass_number, "admissions": len(raw_actions), "terminal_history": dict(Counter(terminals.values())),
            "superseded_without_terminal": superseded, "latest_terminal_count": len(state["completed"]),
            "latest_terminal_outcomes": dict(current_outcomes), "injected_outcomes": dict(intermediate),
            "pressure_passes": dict(pressures), "max_pending": max_pending, "max_state_bytes": max_state_bytes,
            "max_handoff_reservation_bytes": max_reservation, "max_parallel_simulated": max_parallel_observed,
            "max_attempts_per_comparison": max(attempts.values(), default=0),
            "retry_attempts": sum(max(0, value - 1) for value in attempts.values()),
            "deferred_outcomes": intermediate["deferred"],
            "simulated_max_queue_delay_seconds": max(queue_delays, default=0),
            "simulated_job_duration_seconds": {"minimum": min(job_durations, default=0),
                                               "maximum": max(job_durations, default=0),
                                               "sum": sum(job_durations)},
            "undelivered_check_repairs": len(state.get("check_updates", [])),
            "stale_completion_probe": stale_completion_probe(config),
            "raw_events": events, "raw_passes": raw_passes, "raw_actions": raw_actions,
            "final_state_sha256": digest(canonical(state))}


def replay():
    before = {"harness_sha256": digest(Path(__file__).read_bytes()),
              "public_ci_sha256": digest((ROOT / "scripts/public_ci.py").read_bytes()),
              "example_config_sha256": digest((ROOT / "config/public-ci.example.json").read_bytes())}
    started = time.monotonic()
    runs = [run_seed(seed) for seed in SEEDS]
    after = {"harness_sha256": digest(Path(__file__).read_bytes()),
             "public_ci_sha256": digest((ROOT / "scripts/public_ci.py").read_bytes()),
             "example_config_sha256": digest((ROOT / "config/public-ci.example.json").read_bytes())}
    require(before == after, "Source/config changed during replay")
    return {"schema_version": SCHEMA, "status": "passed_offline_queue_model", "source": before,
            "effective_synthetic_config_sha256": digest(canonical(configuration())),
            "seeds": list(SEEDS), "runs": runs, "actual_harness_elapsed_seconds": round(time.monotonic() - started, 6),
            "qualification": ["No GitHub, Pages, Git, Rust analyzer or provider write is invoked.",
                              "Published/unsupported/failure are injected queue-state tokens, not real reports or semantic verdicts.",
                              "Job and queue durations are simulated; no live latency, percentile, browser or hosted-capacity SLO is proved.",
                              "Check repair delivery, authorization, report integrity, billing eligibility and publication remain separate gates."]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New local JSON evidence file; never overwrite")
    args = parser.parse_args(argv)
    try:
        result = replay()
        ci.write_new_json(args.output, result, 32 * ci.MIB)
    except (ReplayFailure, ci.Refusal, ValueError, OSError) as error:
        parser.exit(1, "Offline queue replay failed: " + str(error) + "\n")
    print(json.dumps({"status": result["status"], "seeds": result["seeds"],
                      "latest_terminal_counts": [run["latest_terminal_count"] for run in result["runs"]],
                      "output": str(args.output)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
