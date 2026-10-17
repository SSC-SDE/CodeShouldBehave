#!/usr/bin/env python3
"""Validate a sanitized Phase 2 human pilot observation."""

from __future__ import annotations

import argparse
import hashlib
import json
import stat
import sys
from pathlib import Path

SCHEMA = "codeshouldbehave-phase2-reviewer-observation-v1"
TASKS = {
    "triage",
    "identity",
    "evidence",
    "drilldown",
    "revision",
    "fallback",
    "summary",
}
ROLES = {
    "first-time contributor",
    "service owner",
    "cross-repository reviewer",
    "repository maintainer",
}
ALLOWED_KEYS = {
    "schema",
    "recorded_at_utc",
    "role",
    "experience",
    "authorized_example",
    "tasks",
    "elapsed_seconds",
    "first_relevant_evidence_seconds",
    "backtracks",
    "wrong_interpretation",
    "qualitative_feedback",
    "decision",
    "claims",
    "contains_credentials_or_source",
}


def fail(message: str) -> None:
    raise ValueError(message)


def bounded_text(value: object, name: str, *, required: bool = False) -> str:
    if not isinstance(value, str):
        fail(f"{name} must be text")
    value = value.strip()
    if required and not value:
        fail(f"{name} is required")
    if len(value) > 4000 or "\x00" in value:
        fail(f"{name} exceeds its safe bound")
    return value


def bounded_integer(value: object, name: str, minimum: int, maximum: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{name} must be an integer")
    if value < minimum or value > maximum:
        fail(f"{name} must be between {minimum} and {maximum}")
    return value


def validate(path: Path, owner_role: str) -> dict[str, object]:
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
        fail("observation must be a regular, non-symlink file")
    if metadata.st_size > 65536:
        fail("observation exceeds 64 KiB")
    raw = path.read_bytes()
    try:
        record = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"observation is not valid UTF-8 JSON: {error}")
    if not isinstance(record, dict):
        fail("observation root must be an object")
    unknown = set(record) - ALLOWED_KEYS
    if unknown:
        fail(f"unexpected fields: {', '.join(sorted(unknown))}")
    if record.get("schema") != SCHEMA:
        fail("unsupported observation schema")
    role = bounded_text(record.get("role"), "role", required=True)
    if role not in ROLES:
        fail("role is not recognized")
    if role == owner_role:
        fail("participant role must differ from the repository owner's role")
    bounded_text(record.get("experience"), "experience", required=True)
    bounded_text(record.get("authorized_example"), "authorized_example", required=True)
    bounded_text(record.get("recorded_at_utc"), "recorded_at_utc", required=True)
    bounded_text(record.get("wrong_interpretation"), "wrong_interpretation")
    bounded_text(record.get("qualitative_feedback"), "qualitative_feedback", required=True)
    tasks = record.get("tasks")
    if not isinstance(tasks, dict) or set(tasks) != TASKS or not all(value is True for value in tasks.values()):
        fail("all seven fixed tasks must be present and completed")
    elapsed = bounded_integer(record.get("elapsed_seconds"), "elapsed_seconds", 1, 86400)
    first = bounded_integer(
        record.get("first_relevant_evidence_seconds"),
        "first_relevant_evidence_seconds",
        0,
        elapsed,
    )
    bounded_integer(record.get("backtracks"), "backtracks", 0, 999)
    if record.get("decision") != "pass":
        fail("observation decision must be pass")
    if record.get("contains_credentials_or_source") is not False:
        fail("privacy declaration must confirm no credentials or source")
    if record.get("claims") != {"statistical_improvement": False, "merge_approval": False}:
        fail("claims must explicitly deny statistical improvement and merge approval")
    return {
        "schema": SCHEMA,
        "status": "passed",
        "role": role,
        "experience": record["experience"],
        "authorized_example": record["authorized_example"],
        "elapsed_seconds": elapsed,
        "first_relevant_evidence_seconds": first,
        "backtracks": record["backtracks"],
        "sha256": hashlib.sha256(raw).hexdigest(),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("observation", type=Path)
    parser.add_argument("--owner-role", choices=sorted(ROLES), default="repository maintainer")
    args = parser.parse_args()
    try:
        summary = validate(args.observation, args.owner_role)
    except (OSError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
