#!/usr/bin/env python3
"""Optional offline load/correctness replay with 25 overlapping fixture clients.

Prerequisites: the Phase 5 browser fixture and its actual native host running
on http://127.0.0.1:3202. Finish the separately measured browser replay first.
This driver performs GETs only; it never accepts designs or runs reviewed code.

    CSB_QA_NATIVE_PROFILE=release python3 scripts/phase5_lld_concurrent_replay.py \
        target/phase5-browser-v5/fixture.json \
        --output target/phase5-concurrent-release-final-v1

The local bearer is read from workspace HTML and kept only in process memory.
Reports contain timing, immutable IDs, bounded counts, and fixed failure codes;
they contain no bearer, response bodies, Mermaid, source, or prompt content.
There are 25 client loops, four synchronized rounds, and three fixed seeds.
Each round fetches fresh HLD, LLD catalog, function detail and both exact sources.
The fixture host serializes native work with context_lock. Overlapping clients
therefore test waiting and identity correctness, not parallel native workers.
This is an HTTP profile, not browser usable-render SLO or hosted capacity proof.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
from html.parser import HTMLParser
import http.client
import ipaddress
import json
import math
import os
from pathlib import Path
import random
import re
import socket
import subprocess
import threading
import time
from urllib.parse import urlencode, urlparse


PROJECT = Path(__file__).resolve().parents[1]
CLIENTS = 25
ROUNDS = 4
SEEDS = (1729, 9648, 17567)
MAX_RESPONSE_BYTES = 16 * 1024 * 1024
REQUEST_TIMEOUT_SECONDS = 90
ALLOWED_OUTCOMES = {
    "change_detected", "no_change_detected_within_scope", "incomplete", "unavailable"
}
FORBIDDEN_MARKERS = (
    b"PRIVATE_HLD_SENTINEL", b"PRIVATE_EXCLUDED_SENTINEL",
    b"CSB_LLD_REPLAY_NEVER_DISCLOSE", b"CSB_LLD_PRIVATE_NEVER_DISCLOSE",
)
NATIVE_PROFILE = os.environ.get("CSB_QA_NATIVE_PROFILE", "debug")
SOURCE_FILES = (
    "src/lld.rs", "src/lld/catalog.rs", "src/lld/diff.rs", "src/lld/flow.rs",
    "src/lld/render.rs", "src/bin/csb_onboarding/lld.rs",
    "scripts/github_mvp_server.py", "scripts/phase5_lld_concurrent_replay.py",
    "src/workspace.html", "src/workspace.js", "src/workspace.css",
    f"target/{NATIVE_PROFILE}/csb-onboarding", f"target/{NATIVE_PROFILE}/flowdiff",
)


class ReplayFailure(Exception):
    """Fixed failure codes prevent a response/credential entering a report."""


def require(condition, code):
    if not condition:
        raise ReplayFailure(code)


def sha256(value):
    return hashlib.sha256(value).hexdigest()


def inside(path, root, *, existing=True):
    resolved = path.resolve(strict=existing)
    require(resolved != root and root in resolved.parents, "path_outside_fixture_target")
    return resolved


def source_hashes():
    return {path: sha256((PROJECT / path).read_bytes()) for path in SOURCE_FILES}


def percentile(values, fraction=0.95):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


class SessionMeta(HTMLParser):
    def __init__(self):
        super().__init__()
        self.token = None

    def handle_starttag(self, tag, attributes):
        values = dict(attributes)
        if tag == "meta" and values.get("name") == "csb-local-session":
            require(self.token is None, "duplicate_local_session_meta")
            value = values.get("content", "")
            require(re.fullmatch(r"[A-Za-z0-9_-]{20,128}", value) is not None,
                    "invalid_local_session_meta")
            self.token = value


def fixture_inputs(args):
    address = urlparse(args.origin)
    try:
        port = address.port
    except ValueError:
        raise ReplayFailure("invalid_fixture_origin") from None
    require(address.scheme == "http" and address.hostname in
            ("127.0.0.1", "localhost", "::1") and port == 3202,
            "fixture_only_loopback_port_3202_required")
    require(not address.username and not address.password and not address.query
            and not address.fragment and address.path in ("", "/")
            and not address.params, "fixture_origin_has_extra_fields")
    addresses = socket.getaddrinfo(address.hostname, port, type=socket.SOCK_STREAM)
    require(addresses and all(ipaddress.ip_address(item[4][0]).is_loopback
                             for item in addresses), "fixture_dns_not_loopback")
    host = addresses[0][4][0]
    target = (PROJECT / "target").resolve(strict=True)
    fixture_path = inside(args.fixture, target)
    require(fixture_path.name == "fixture.json", "fixture_manifest_name_required")
    fixture = json.loads(fixture_path.read_text())
    require(re.fullmatch(r"fixture/[a-z0-9-]+", fixture.get("slug", "")) is not None,
            "synthetic_fixture_slug_required")
    root = inside(Path(fixture["root"]), target)
    require(fixture_path.parent == root, "fixture_manifest_root_mismatch")
    for key in ("repo", "output", "bin"):
        fixture[key] = str(inside(Path(fixture[key]), root))
    remote = subprocess.run(
        ["git", "-c", "core.hooksPath=" + os.devnull, "-C", fixture["repo"],
         "remote", "get-url", "origin"], capture_output=True, text=True,
        timeout=10, check=False,
        env={**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull},
    )
    require(remote.returncode == 0 and remote.stdout.strip() ==
            f"https://github.com/{fixture['slug']}.git", "non_fixture_git_remote")
    require(re.fullmatch(r"[0-9a-f]{40,64}", fixture.get("baseline", "")) is not None,
            "invalid_fixture_baseline")
    for number in (99, 100):
        pin = fixture.get("reviews", {}).get(str(number), {})
        require(re.fullmatch(rf"pr-{number}-[0-9a-f]{{64}}", pin.get("review_id", ""))
                is not None, "fixture_review_identity_missing")
        require(re.fullmatch(r"[0-9a-f]{40,64}", pin.get("head", "")) is not None,
                "fixture_proposal_identity_missing")
        artifact_path = inside(Path(fixture["output"]) / f"pr-{number}.json", root)
        artifact = json.loads(artifact_path.read_text())
        observed = artifact.get("hld", {}).get("proposal", {})
        identity = observed.get("repository", {})
        require(f"{identity.get('owner')}/{identity.get('name')}" == fixture["slug"]
                and observed.get("snapshot") == pin["head"]
                and observed.get("id") == pin["proposal_design"],
                "fixture_artifact_repository_pin_mismatch")
    output = inside(args.output, target, existing=False)
    require(not output.exists(), "refuse_existing_replay_output")
    require(output.parent.exists(), "replay_output_parent_missing")
    output.mkdir(mode=0o700)
    return fixture, fixture_path, host, port, output


class Transport:
    def __init__(self, host, port, token, seed, raw, origin):
        self.host, self.port, self.token = host, port, token
        self.seed, self.raw, self.origin = seed, raw, origin
        self.started = time.perf_counter()
        self.lock = threading.Lock()
        self.in_flight = 0
        self.peak = 0
        self.samples = []

    def request(self, client, round_number, kind, path):
        begin = time.perf_counter()
        with self.lock:
            self.in_flight += 1
            self.peak = max(self.peak, self.in_flight)
            at_start = self.in_flight
        connection = http.client.HTTPConnection(self.host, self.port,
                                               timeout=REQUEST_TIMEOUT_SECONDS)
        status = None
        failure = None
        response_bytes = 0
        try:
            connection.request("GET", path, headers={
                "Authorization": "Bearer " + self.token,
                "Accept": "application/json",
                "Origin": self.origin,
                "Connection": "close",
            })
            response = connection.getresponse()
            status = response.status
            body = response.read(MAX_RESPONSE_BYTES + 1)
            response_bytes = len(body)
            require(response_bytes <= MAX_RESPONSE_BYTES, "response_payload_limit")
            require(not any(marker in body for marker in FORBIDDEN_MARKERS),
                    "excluded_sentinel_in_response")
            require(status == 200, "unexpected_http_status")
            require(response.getheader("Content-Type", "").startswith("application/json"),
                    "non_json_api_response")
            value = json.loads(body)
            require(isinstance(value, dict), "non_object_api_response")
            return value
        except ReplayFailure as error:
            failure = str(error)
            raise
        except Exception:
            failure = "transport_or_json_failure"
            raise ReplayFailure(failure) from None
        finally:
            connection.close()
            end = time.perf_counter()
            record = {
                "seed": self.seed, "client": client, "round": round_number,
                "kind": kind, "started_ms": (begin - self.started) * 1000,
                "ended_ms": (end - self.started) * 1000,
                "http_json_duration_ms": (end - begin) * 1000,
                "http_status": status, "response_bytes": response_bytes,
                "client_requests_in_flight_at_start": at_start,
                "transport_failure_code": failure,
            }
            with self.lock:
                self.in_flight -= 1
                self.samples.append(record)
                self.raw.write(json.dumps(record, separators=(",", ":")) + "\n")
                self.raw.flush()


def local_token(host, port):
    connection = http.client.HTTPConnection(host, port, timeout=REQUEST_TIMEOUT_SECONDS)
    try:
        connection.request("GET", "/workspace", headers={"Connection": "close"})
        response = connection.getresponse()
        require(response.status == 200, "fixture_workspace_not_available")
        body = response.read(MAX_RESPONSE_BYTES + 1)
        require(len(body) <= MAX_RESPONSE_BYTES, "workspace_payload_limit")
        parser = SessionMeta()
        parser.feed(body.decode("utf-8"))
        require(parser.token is not None, "fixture_workspace_session_missing")
        without_session = re.sub(
            rb'<meta name="csb-local-session" content="[^"]*">', b"", body
        )
        require(sha256(without_session) == sha256((PROJECT / "src/workspace.html").read_bytes()),
                "served_workspace_does_not_match_frozen_source")
        return parser.token
    finally:
        connection.close()


def pinned_identity(value, fixture, number, parent, component, scope):
    require(value.get("review_id") == fixture["reviews"][str(number)]["review_id"],
            "crossed_review_identity")
    if parent is not None:
        require(value.get("parent_hld") == parent and value.get("component") == component,
                "crossed_hld_component_identity")
        require(value.get("scope_version") == scope, "crossed_context_scope_identity")


def select_evidence(detail, rng):
    candidates = [node for node in detail.get("graph", {}).get("nodes", [])
                  if node.get("operation") in ("condition", "return", "try_propagation",
                      "binding", "call", "assignment", "unsupported_region")]
    require(candidates, "selected_function_has_no_source_backed_flow_node")
    node = candidates[rng.randrange(len(candidates))]
    evidence = [item for item in detail.get("evidence", [])
                if item.get("id") == node.get("evidence_id")]
    require(len(evidence) == 1, "flow_node_exact_evidence_missing_or_ambiguous")
    span = evidence[0]
    require(span.get("path") == detail["entry"]["path"]
            and span.get("blob") == detail["entry"]["blob"]
            and span.get("start_line") == node.get("start_line")
            and span.get("end_line") == node.get("end_line"), "node_evidence_span_mismatch")
    return node, span


def reviewer(client, seed, fixture, transport, barrier):
    rng = random.Random(seed * 1000003 + client)
    rows = []
    try:
        for round_number in range(ROUNDS):
            number = 99 if rng.randrange(2) == 0 else 100
            name = "handle" if rng.randrange(2) == 0 else "private_helper"
            # A real barrier precedes every burst. Requests overlap, and the
            # counters include connect/read plus time waiting for context_lock.
            barrier.wait(timeout=REQUEST_TIMEOUT_SECONDS * 3)
            review_id = fixture["reviews"][str(number)]["review_id"]
            prefix = f"/v1/reviews/{review_id}"
            hld = transport.request(client, round_number, "hld", prefix + "/hld")
            pinned_identity(hld, fixture, number, None, None, None)
            proposal = hld.get("proposal", {})
            require(hld.get("baseline", {}).get("snapshot") == fixture["baseline"]
                    and proposal.get("snapshot") == fixture["reviews"][str(number)]["head"],
                    "crossed_hld_commit_identity")
            identity = proposal.get("repository", {})
            require(f"{identity.get('owner')}/{identity.get('name')}" == fixture["slug"],
                    "served_repository_is_not_synthetic_fixture")
            components = [item for item in proposal.get("elements", [])
                          if item.get("kind") == "component" and "api" in item.get("roots", [])]
            require(len(components) == 1, "fixture_api_component_not_unique")
            parent, component, scope = proposal["id"], components[0]["id"], proposal["scope_version"]
            query = {"parent_hld": parent, "component": component}
            catalog = transport.request(client, round_number, "catalog",
                                        prefix + "/lld/catalog?" + urlencode(query))
            pinned_identity(catalog, fixture, number, parent, component, scope)
            require(catalog.get("baseline", {}).get("snapshot") == fixture["baseline"]
                    and catalog.get("proposal", {}).get("snapshot") == proposal["snapshot"],
                    "crossed_catalog_commit_identity")
            require(catalog["baseline"]["hld_design_id"] == hld["baseline"]["id"]
                    and catalog["proposal"]["hld_design_id"] == parent,
                    "crossed_catalog_parent_design")
            choices = [row for row in catalog.get("entries", [])
                       if (row.get("proposal") or row.get("baseline") or {}).get("kind") == "function"
                       and (row.get("proposal") or row.get("baseline") or {}).get("name") == name]
            require(len(choices) == 1, "fixture_selected_function_not_unique")
            selected = choices[0]
            require(selected.get("baseline") and selected.get("proposal"),
                    "fixture_paired_function_missing")
            entry = selected["proposal"]["id"]
            query["entry"] = entry
            detail = transport.request(client, round_number, "detail",
                                       prefix + "/lld/detail?" + urlencode({**query, "side": "diff"}))
            pinned_identity(detail, fixture, number, parent, component, scope)
            require(detail.get("entry_id") == entry, "crossed_selected_entry")
            require(detail.get("delta", {}).get("outcome") in ALLOWED_OUTCOMES,
                    "missing_explicit_detail_outcome")
            if detail["delta"]["outcome"] == "no_change_detected_within_scope":
                require(all(detail.get(side, {}).get("graph", {}).get("complete") is True
                            and detail.get(side, {}).get("coverage", {}).get("structure_complete") is True
                            for side in ("baseline", "proposal")),
                        "false_clean_outcome_with_incomplete_local_detail")
            pins = []
            sides = ["baseline", "proposal"]
            rng.shuffle(sides)
            for side in sides:
                current = detail.get(side)
                require(isinstance(current, dict) and current.get("entry") == selected[side],
                        "crossed_detail_entry_occurrence")
                require(current.get("catalog_id") == catalog[side]["id"],
                        "crossed_detail_catalog_generation")
                graph, coverage = current.get("graph", {}), current.get("coverage", {})
                require(isinstance(graph.get("complete"), bool)
                        and (graph["complete"] or graph.get("limitations") or coverage.get("gaps")),
                        "silent_incomplete_local_flow")
                require(len(graph.get("nodes", [])) <= 200 and len(graph.get("edges", [])) <= 400,
                        "local_graph_bounds_exceeded")
                require(coverage.get("runtime_validation") == "unavailable",
                        "unexpected_runtime_validation_claim")
                node, evidence = select_evidence(current, rng)
                expected_commit = fixture["baseline"] if side == "baseline" else proposal["snapshot"]
                require(evidence.get("commit") == expected_commit,
                        "crossed_detail_evidence_commit")
                source = transport.request(client, round_number, "source_" + side,
                    prefix + "/lld/source?" + urlencode({**query, "evidence": evidence["id"]}))
                pinned_identity(source, fixture, number, parent, component, scope)
                require(source.get("evidence_id") == evidence["id"]
                        and source.get("commit") == expected_commit
                        and source.get("path") == evidence["path"]
                        and source.get("blob_id") == evidence["blob"],
                        "crossed_exact_source_identity")
                require(source.get("start_line") == evidence["start_line"]
                        and evidence["start_line"] <= source.get("end_line", 0) <= evidence["end_line"],
                        "crossed_exact_source_span")
                text = source.get("text")
                require(isinstance(text, str) and text and len(text.encode()) <= 65536
                        and len(text.split("\n")) <= 200,
                        "source_missing_or_preview_bounds_exceeded")
                expected_truncation = source["end_line"] < evidence["end_line"]
                require(source.get("truncated") is expected_truncation,
                        "silent_source_preview_truncation")
                pins.append({"side": side, "commit": expected_commit, "entry_id": current["entry"]["id"],
                    "detail_id": current["id"], "catalog_id": current["catalog_id"],
                    "node_id": node["id"], "evidence_id": evidence["id"],
                    "blob_id": evidence["blob"], "start_line": source["start_line"],
                    "end_line": source["end_line"], "flow_complete": graph["complete"],
                    "coverage_partial": not coverage.get("structure_complete", False)})
            rows.append({"seed": seed, "client": client, "round": round_number,
                "pr": number, "name": name, "review_id": review_id,
                "parent_hld": parent, "component": component, "scope_version": scope,
                "detail_outcome": detail["delta"]["outcome"], "identity_verified": True,
                "both_exact_source_sides_verified": True, "pins": pins})
        return {"client": client, "completed_rounds": rows, "failure_code": None}
    except Exception as error:
        barrier.abort()
        code = str(error) if isinstance(error, ReplayFailure) else "client_or_barrier_failure"
        return {"client": client, "completed_rounds": rows, "failure_code": code}


def seed_run(seed, fixture, host, port, token, output, origin):
    raw_path = output / f"seed-{seed}-requests.jsonl"
    with raw_path.open("x", encoding="utf-8") as raw:
        raw_path.chmod(0o600)
        transport = Transport(host, port, token, seed, raw, origin)
        barrier = threading.Barrier(CLIENTS)
        started = time.perf_counter()
        with ThreadPoolExecutor(max_workers=CLIENTS, thread_name_prefix="csb-fixture-reviewer") as executor:
            tasks = [executor.submit(reviewer, client, seed, fixture, transport, barrier)
                     for client in range(CLIENTS)]
            clients = [future.result() for future in as_completed(tasks)]
        elapsed = time.perf_counter() - started
        samples = transport.samples
        errors = [row for row in clients if row["failure_code"]]
        rows = [row for client in clients for row in client["completed_rounds"]]
        valid = (not errors and len(rows) == CLIENTS * ROUNDS
                 and len(samples) == CLIENTS * ROUNDS * 5 and transport.peak == CLIENTS
                 and transport.in_flight == 0
                 and all(row["http_status"] == 200 and row["transport_failure_code"] is None
                         for row in samples))
        grouped = {}
        for kind in ("hld", "catalog", "detail", "source_baseline", "source_proposal"):
            timings = [row["http_json_duration_ms"] for row in samples if row["kind"] == kind]
            grouped[kind] = {"requests": len(timings), "http_json_p95_ms": percentile(timings),
                             "http_json_max_ms": max(timings) if timings else None}
        report = {
            "seed": seed, "status": "passed" if valid else "failed",
            "simultaneously_active_client_loops": CLIENTS, "rounds_per_client": ROUNDS,
            "round_start_barrier": True, "actual_client_peak_requests_in_flight": transport.peak,
            "in_flight_definition": "Client connect/read requests including host context_lock waiting; not parallel native workers",
            "completed_identity_verified_rounds": len(rows), "http_requests": len(samples),
            "exact_source_pins_verified": sum(len(row["pins"]) for row in rows),
            "local_flow_complete_pins": sum(pin["flow_complete"] for row in rows for pin in row["pins"]),
            "qualified_context_partial_pins": sum(pin["coverage_partial"] for row in rows for pin in row["pins"]),
            "pr_distribution": {str(number): sum(row["pr"] == number for row in rows) for number in (99, 100)},
            "selected_function_distribution": {name: sum(row["name"] == name for row in rows) for name in ("handle", "private_helper")},
            "elapsed_seconds": elapsed, "http_request_throughput_per_second": len(samples) / elapsed,
            "paired_round_throughput_per_second": len(rows) / elapsed,
            "endpoint_timings": grouped, "http_status_counts": {
                str(status): sum(1 for row in samples if row["http_status"] == status)
                for status in sorted({row["http_status"] for row in samples}, key=str)},
            "client_failures": [{"client": row["client"], "failure_code": row["failure_code"]}
                                for row in errors],
            "client_rounds": sorted(rows, key=lambda row: (row["client"], row["round"])),
            "raw_requests_file": raw_path.name,
        }
        destination = output / f"seed-{seed}.json"
        destination.write_text(json.dumps(report, indent=2) + "\n")
        destination.chmod(0o600)
        return report


def run(args):
    fixture, fixture_path, host, port, output = fixture_inputs(args)
    require(NATIVE_PROFILE in ("debug", "release"), "invalid_native_profile")
    before_hashes = source_hashes()
    report = {
        "schema": "codeshouldbehave-phase5-concurrent-reviewers-v1",
        "status": "running", "fixture_path": str(fixture_path), "origin": args.origin,
        "native_profile": NATIVE_PROFILE,
        "synthetic_repository": fixture["slug"], "seeds": list(SEEDS),
        "profile": "25 overlapping client streams; fresh HLD/catalog/detail/exact paired source; single local native host with serialized context_lock",
        "transport": "Actual loopback Python host and built Rust, private immutable fixture artifacts/SQLite, synthetic gh stub; no provider network",
        "authentication": "One fixture actor; 25 client loops share a local-session bearer kept solely in memory; not 25 distinct accounts or tenants",
        "read_only_http": True, "source_and_credentials_in_reports": False,
        "presentation_timing": "HTTP/JSON completion only; no browser render or usable interaction timing",
        "concurrent_latency_slo": "None claimed; 300/500 ms usable interaction gates are measured separately in the sandboxed browser profile",
        "resource_scope": "No aggregate RSS/CPU or native worker parallelism inferred from client counters",
        "source_sha256_before": before_hashes, "runs": [],
    }
    try:
        token = local_token(host, port)
        with (output / "profile-check.jsonl").open("w") as raw:
            probe = Transport(host, port, token, 0, raw, args.origin)
            observation = probe.request(-1, -1, "native_profile_preflight", "/api/status")
            require(observation.get("native_profile") == NATIVE_PROFILE, "host_native_profile_mismatch")
        for seed in SEEDS:
            result = seed_run(seed, fixture, host, port, token, output, args.origin)
            report["runs"].append(result)
            print(json.dumps({"seed": seed, "status": result["status"],
                "requests": result["http_requests"],
                "peak_in_flight": result["actual_client_peak_requests_in_flight"],
                "elapsed_seconds": result["elapsed_seconds"]}))
            if result["status"] != "passed":
                raise ReplayFailure("concurrent_seed_correctness_failed")
        after_hashes = source_hashes()
        report["source_sha256_after"] = after_hashes
        require(before_hashes == after_hashes, "native_or_fixture_source_changed_during_replay")
        report["status"] = "passed_local_overlapping_request_correctness"
        report["checks"] = [
            "25 client requests measured in flight in each seeded barrier profile",
            "PR 99/100 exact baseline/proposal, parent HLD, component and scope stayed paired",
            "Catalog/function occurrence/detail/evidence/blob/source revision identities stayed paired",
            "Every response was HTTP 200 with complete or visibly qualified partial static evidence",
            "Both source sides were bounded and truncation explicit",
            "Excluded/secret fixture sentinel markers absent from every response",
            "Native and served workspace source hashes remained frozen",
        ]
        report["remaining_production_gaps"] = [
            "Real GitHub App/OAuth users, network/webhook ingestion, distinct actors and tenant isolation",
            "Hosted Vercel/backend capacity, production workload/debounce and quotas",
            "Parallel native workers and concurrent usable browser SLOs",
            "Unsupported Rust macros/cfg/dispatch/state/runtime semantics remain explicitly unresolved",
        ]
    except Exception as error:
        report["status"] = "failed"
        report["failure_code"] = str(error) if isinstance(error, ReplayFailure) else "replay_setup_or_io_failure"
    destination = output / "report.json"
    destination.write_text(json.dumps(report, indent=2) + "\n")
    destination.chmod(0o600)
    print(json.dumps({"status": report["status"], "report": str(destination)}))
    return 0 if report["status"].startswith("passed_") else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--origin", default="http://127.0.0.1:3202")
    args = parser.parse_args()
    try:
        raise SystemExit(run(args))
    except ReplayFailure as error:
        print(json.dumps({"status": "setup_refused", "failure_code": str(error)}))
        raise SystemExit(2) from None
    except (OSError, ValueError, KeyError, TypeError):
        print(json.dumps({"status": "setup_refused", "failure_code": "invalid_fixture_inputs"}))
        raise SystemExit(2) from None
