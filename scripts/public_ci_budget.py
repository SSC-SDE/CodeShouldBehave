#!/usr/bin/env python3
"""Disjoint, durable application-REST tickets for one trusted controller pass.

This does not count GitHub Actions' internal requests, attestation verification,
Git transport, or Pages asset reads. It bounds public_ci.GitHub.api transports.
The pinned workflow runs one plan, one publisher, and at most three distinct
analysis jobs. Each job has a disjoint allocation, so they need no shared mutable
cross-runner file. Local counters cannot be reinitialized by another CLI process.
The runner filesystem and pinned workflow remain trusted; this is not a defense
against an operator editing both executable code and its private local ledger.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile

SCHEMA = "csb-public-api-budget-v1"
LEDGER_SCHEMA = "csb-public-api-role-ledger-v1"
MAX_CALLS = 100
LIMITS = {"plan": 25, "analyze-0": 4, "analyze-1": 4, "analyze-2": 4, "publish": 63}
HEX = re.compile(r"[0-9a-f]{64}\Z")
MAX_LEDGER_BYTES = 32 * 1024


class Refusal(ValueError):
    pass


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()


def sha(value):
    return hashlib.sha256(value).hexdigest()


def integer(value, low, high):
    return isinstance(value, int) and not isinstance(value, bool) and low <= value <= high


def strict_json(data):
    def pairs(values):
        result = {}
        for key, value in values:
            if key in result:
                raise Refusal("Duplicate REST budget JSON key")
            result[key] = value
        return result
    try:
        return json.loads(data, object_pairs_hook=pairs)
    except (ValueError, UnicodeError) as exc:
        raise Refusal("Invalid REST budget JSON") from exc


def pass_context(manifest):
    return {key: manifest[key] for key in ("schema_version", "repository_id", "run_id", "run_attempt",
                                           "config_sha256", "workflow_sha256", "limit", "allocations")}


def validate_manifest(value):
    fields = {"schema_version", "repository_id", "run_id", "run_attempt", "config_sha256",
              "workflow_sha256", "limit", "allocations", "analysis_requests", "pass_id"}
    if not isinstance(value, dict) or set(value) != fields:
        raise Refusal("Malformed REST pass budget")
    if value["schema_version"] != SCHEMA or not integer(value["limit"], MAX_CALLS, MAX_CALLS) or value["allocations"] != LIMITS or any(not integer(item, 1, MAX_CALLS) for item in value["allocations"].values()) or sum(LIMITS.values()) != MAX_CALLS:
        raise Refusal("REST allocations do not match the fixed one-pass contract")
    if not integer(value["repository_id"], 1, 2**63 - 1) or not integer(value["run_id"], 1, 2**63 - 1) or not integer(value["run_attempt"], 1, 10_000):
        raise Refusal("Invalid REST budget run/repository identity")
    for key in ("config_sha256", "workflow_sha256", "pass_id"):
        if not isinstance(value[key], str) or not HEX.fullmatch(value[key]):
            raise Refusal("Invalid REST budget identity digest")
    if value["pass_id"] != sha(canonical(pass_context(value))):
        raise Refusal("REST pass identity differs")
    requests = value["analysis_requests"]
    if not isinstance(requests, list) or len(requests) > 3 or any(not isinstance(item, str) or not HEX.fullmatch(item) for item in requests) or len(set(requests)) != len(requests):
        raise Refusal("Invalid or repeated REST analysis ticket assignment")
    return strict_json(canonical(value))


def make_manifest(config, run_id, run_attempt, request_ids=()):
    value = {"schema_version": SCHEMA, "repository_id": config["repository"]["id"],
             "run_id": run_id, "run_attempt": run_attempt, "config_sha256": sha(canonical(config)),
             "workflow_sha256": config["workflow_sha256"], "limit": MAX_CALLS,
             "allocations": dict(LIMITS), "analysis_requests": list(request_ids)}
    value["pass_id"] = sha(canonical(pass_context(value)))
    return validate_manifest(value)


def assign_requests(manifest, request_ids):
    result = validate_manifest(manifest)
    if result["analysis_requests"] and result["analysis_requests"] != list(request_ids):
        raise Refusal("REST analysis tickets are already assigned")
    result["analysis_requests"] = list(request_ids)
    return validate_manifest(result)


def analysis_role(manifest, request_id):
    value = validate_manifest(manifest)
    try:
        return "analyze-" + str(value["analysis_requests"].index(request_id))
    except ValueError as exc:
        raise Refusal("Request has no assigned REST ticket") from exc


def binding_for(manifest, role, request_id=None):
    value = validate_manifest(manifest)
    if role not in LIMITS:
        raise Refusal("Unknown REST stage allocation")
    if role.startswith("analyze-"):
        if analysis_role(value, request_id) != role:
            raise Refusal("REST analysis role/request differs")
    elif request_id is not None:
        raise Refusal("Non-analysis REST ticket cannot carry a request")
    return {"pass_id": value["pass_id"], "role": role, "limit": LIMITS[role], "request_id": request_id}


class Budget:
    """One role's counter, shared safely by threads and CLI process instances."""
    def __init__(self, directory, manifest, role, request_id=None):
        self.manifest = validate_manifest(manifest)
        self.binding = binding_for(self.manifest, role, request_id)
        if os.environ.get("GITHUB_ACTIONS") == "true":
            job = "analyze" if role.startswith("analyze-") else role
            if os.environ.get("GITHUB_RUN_ID") != str(self.manifest["run_id"]) or os.environ.get("GITHUB_RUN_ATTEMPT") != str(self.manifest["run_attempt"]) or os.environ.get("GITHUB_JOB") != job:
                raise Refusal("REST ticket is outside its actual workflow run/job")
        directory = Path(directory)
        if directory.is_symlink():
            raise Refusal("Linked REST budget directory")
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.directory = directory.resolve()
        directory_info = self.directory.stat()
        if directory_info.st_uid != os.getuid() or directory_info.st_mode & 0o022:
            raise Refusal("REST budget directory is not owned/protected")
        self.path = self.directory / (role + ".json")
        self.lock_path = self.directory / (role + ".lock")
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
        try:
            descriptor = os.open(self.lock_path, flags, 0o600)
            os.close(descriptor)
            created = True
        except FileExistsError:
            created = False
        with self._locked():
            if created:
                if self.path.exists() or self.path.is_symlink():
                    raise Refusal("REST ledger existed before its one-use claim")
                self._write({"schema_version": LEDGER_SCHEMA, "binding": self.binding, "used": 0, "reservations": []})
            self._read()  # A deleted/corrupt prior ledger is never a fresh budget.

    def _locked(self):
        class Lock:
            def __enter__(inner):
                try:
                    descriptor = os.open(self.lock_path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
                except OSError as exc:
                    raise Refusal("Missing or linked REST budget claim") from exc
                info = os.fstat(descriptor)
                if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o022:
                    os.close(descriptor)
                    raise Refusal("REST budget claim is not owned/protected")
                inner.file = os.fdopen(descriptor, "rb")
                fcntl.flock(inner.file, fcntl.LOCK_EX)
                return inner

            def __exit__(inner, *_):
                fcntl.flock(inner.file, fcntl.LOCK_UN)
                inner.file.close()
        return Lock()

    def _read(self):
        if self.path.is_symlink() or not self.path.is_file() or self.path.stat().st_size > MAX_LEDGER_BYTES:
            raise Refusal("Missing/linked/oversized REST ledger; recovery required")
        value = strict_json(self.path.read_bytes())
        if not isinstance(value, dict) or set(value) != {"schema_version", "binding", "used", "reservations", "ledger_sha256"}:
            raise Refusal("Malformed REST role ledger")
        proof = value.pop("ledger_sha256")
        if proof != sha(canonical(value)) or value["schema_version"] != LEDGER_SCHEMA or value["binding"] != self.binding:
            raise Refusal("REST ledger identity/integrity differs")
        records = value["reservations"]
        if not integer(value["used"], 0, self.binding["limit"]) or not isinstance(records, list) or len(records) != value["used"]:
            raise Refusal("REST counter was reset or exceeds its allocation")
        for ordinal, record in enumerate(records, 1):
            if not isinstance(record, dict) or set(record) != {"ordinal", "method", "path_sha256"} or record["ordinal"] != ordinal or record["method"] not in ("GET", "POST", "PATCH", "DELETE") or not isinstance(record["path_sha256"], str) or not HEX.fullmatch(record["path_sha256"]):
                raise Refusal("Invalid REST reservation history")
        return value

    def _write(self, value):
        output = dict(value, ledger_sha256=sha(canonical(value)))
        data = canonical(output)
        if len(data) > MAX_LEDGER_BYTES:
            raise Refusal("REST ledger byte budget exhausted")
        descriptor, name = tempfile.mkstemp(prefix=".rest-budget-", dir=self.directory)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(data); stream.flush(); os.fsync(stream.fileno())
            os.replace(name, self.path)
            directory_fd = os.open(self.directory, os.O_RDONLY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
        finally:
            Path(name).unlink(missing_ok=True)

    def reserve(self, method, path):
        if method not in ("GET", "POST", "PATCH", "DELETE") or not isinstance(path, str) or not 1 <= len(path.encode()) <= 4096:
            raise Refusal("Invalid REST transport reservation")
        with self._locked():
            value = self._read()
            if value["used"] >= self.binding["limit"]:
                raise Refusal("Application REST stage allocation exhausted; no transport permitted")
            value["used"] += 1
            value["reservations"].append({"ordinal": value["used"], "method": method, "path_sha256": sha(path.encode())})
            self._write(value)  # Reserve durably BEFORE the provider request.
            return value["used"]

    def snapshot(self):
        with self._locked():
            value = self._read()
        return dict(value, remaining=self.binding["limit"] - value["used"], ledger_sha256=sha(canonical(value)))


def aggregate(manifest, snapshots):
    value = validate_manifest(manifest)
    expected = {"plan", "publish"} | {"analyze-" + str(index) for index in range(len(value["analysis_requests"]))}
    seen, used = set(), 0
    for snapshot in snapshots:
        if not isinstance(snapshot, dict) or set(snapshot) != {"schema_version", "binding", "used", "reservations", "remaining", "ledger_sha256"} or snapshot["schema_version"] != LEDGER_SCHEMA or not isinstance(snapshot["binding"], dict):
            raise Refusal("Malformed REST stage snapshot")
        role = snapshot.get("binding", {}).get("role")
        if not isinstance(role, str) or role not in expected:
            raise Refusal("Unknown REST stage snapshot")
        request_id = value["analysis_requests"][int(role[-1])] if role in expected and role.startswith("analyze-") else None
        binding = binding_for(value, role, request_id)
        if role not in expected or role in seen or snapshot.get("binding") != binding or not integer(snapshot.get("used"), 0, binding["limit"]) or snapshot.get("remaining") != binding["limit"] - snapshot["used"]:
            raise Refusal("Repeated/missing/invalid REST stage snapshot")
        payload = {key: snapshot[key] for key in ("schema_version", "binding", "used", "reservations")}
        if snapshot.get("ledger_sha256") != sha(canonical(payload)) or not isinstance(payload["reservations"], list) or len(payload["reservations"]) != payload["used"]:
            raise Refusal("REST snapshot proof differs")
        for ordinal, record in enumerate(payload["reservations"], 1):
            if not isinstance(record, dict) or set(record) != {"ordinal", "method", "path_sha256"} or record["ordinal"] != ordinal or record["method"] not in ("GET", "POST", "PATCH", "DELETE") or not isinstance(record["path_sha256"], str) or not HEX.fullmatch(record["path_sha256"]):
                raise Refusal("REST snapshot reservation history differs")
        seen.add(role); used += snapshot["used"]
    if seen != expected or used > MAX_CALLS:
        raise Refusal("REST pass has missing stages or exceeds100 application calls")
    return {"schema_version": SCHEMA, "pass_id": value["pass_id"], "application_rest_calls": used,
            "limit": MAX_CALLS, "roles": sorted(seen), "complete_stage_evidence": True}
