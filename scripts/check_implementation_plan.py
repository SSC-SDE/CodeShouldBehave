"""Validate plan structure and prerequisite ordering; this is not a product test."""

import re
from pathlib import Path


def check():
    root = Path(__file__).resolve().parents[1]
    plan = (root / "docs/IMPLEMENTATION_PLAN.md").read_text()
    contracts = root / "docs/IMPLEMENTATION_CONTRACTS.md"
    assert contracts.is_file(), "Missing normative implementation contracts"

    phases = list(re.finditer(r"^## Phase ([1-8]) — (.+)$", plan, re.M))
    assert [int(m[1]) for m in phases] == list(range(1, 9)), "Phases out of order"
    expected_titles = ["foundation", "Reviewer", "onboarding", "High-Level", "Low-Level", "Observability", "AI integration", "Production hardening"]
    for phase, title in zip(phases, expected_titles):
        assert title in phase[2], f"Unexpected phase title: {phase[0]}"

    counts = {1: 17, 2: 7, 3: 11, 4: 7, 5: 7, 6: 8, 7: 8, 8: 6}
    steps = list(re.finditer(r"^### Step ([1-8])\.(\d+) — (.+)$", plan, re.M))
    identifiers = [(int(m[1]), int(m[2])) for m in steps]
    expected = [(phase, step) for phase, count in counts.items() for step in range(1, count + 1)]
    assert identifiers == expected, "Missing, duplicate, or out-of-order steps"

    for i, step in enumerate(steps):
        identity = identifiers[i]
        phase_start = phases[identity[0] - 1].start()
        phase_end = phases[identity[0]].start() if identity[0] < 8 else len(plan)
        assert phase_start < step.start() < phase_end, f"Step outside its phase: {identity}"
        end = steps[i + 1].start() if i + 1 < len(steps) else len(plan)
        block = plan[step.end():min(end, phase_end)]
        prerequisite = re.search(r"^\*\*Prerequisites:\*\* (.+)$", block, re.M)
        assert prerequisite, f"Missing prerequisites: {identity}"
        assert "**Gate:**" in block or "**Release gate:**" in block, f"Missing gate: {identity}"
        assert f"docs/evidence/P{identity[0]}.{identity[1]}/acceptance.md" in block, f"Missing evidence path: {identity}"
        assert re.search(r"^- \[[ x]\]", block, re.M), f"No actionable checklist: {identity}"
        if not prerequisite[1].startswith("None."):
            refs = re.findall(r"Step ([1-8])\.(\d+)", prerequisite[1])
            assert refs, f"No explicit prerequisite references: {identity}"
            for phase, number in refs:
                dependency = (int(phase), int(number))
                assert dependency in identifiers, f"Unknown dependency: {dependency}"
                assert identifiers.index(dependency) < i, f"Forward/cyclic prerequisite: {identity} -> {dependency}"

    for phase, number in re.findall(r"\bStep ([1-8])\.(\d+)\b", plan):
        assert (int(phase), int(number)) in identifiers, f"Broken step reference: {phase}.{number}"
    assert not re.search(r"\bStep \d+(?![\d.])\b", plan), "Unqualified legacy step reference"
    assert "IMPLEMENTATION_CONTRACTS.md" in plan
    assert "IMPLEMENTATION_CONTRACTS.md" in (root / "README.md").read_text()
    tracker = (root / "docs/implementation-progress.md").read_text()
    rows = [line for line in tracker.splitlines() if re.match(r"^\| [1-8] \|", line)]
    assert len(rows) == 8, "Tracker must contain exactly eight phase rows"
    statuses = {"not_started", "in_progress", "blocked", "draft_pr", "ready_for_review", "merged"}
    for phase, row in enumerate(rows, 1):
        fields = [field.strip().strip("`") for field in row.split("|")[1:-1]]
        assert len(fields) == 10, f"Unexpected tracker columns for phase {phase}"
        assert fields[0] == str(phase), "Tracker phases out of order"
        assert fields[2:4] == [f"phase-{phase}", "main"], f"Wrong branch or PR base: phase {phase}"
        assert fields[4] in statuses, f"Unknown phase status: {fields[4]}"
        if fields[4] == "merged":
            assert fields[6] not in {"", "—"}, f"Missing merged PR URL: phase {phase}"
            assert re.fullmatch(r"[0-9a-f]{7,40}", fields[9]), f"Missing resulting commit: phase {phase}"
    assert "implementation-progress.md" in plan
    assert "implementation-progress.md" in (root / "AGENTS.md").read_text()
    print(f"PASS: 8 ordered phases, {len(steps)} steps, backward prerequisites, acceptance gates, and evidence records.")
    print("PASS: eight tracked phase branches, main PR targets, and repository workflow instructions.")
    print("Documentation validation only; implementation, security and release gates remain unverified.")


if __name__ == "__main__":
    check()
