# Decisions and Situations — HLD review workstream

Scope: the HLD review engine, report budget, and publication bookkeeping.
Source of truth: provider-confirmed merges and the verification commands listed
below. PR numbers refer to SSC-SDE/CodeShouldBehave.

## Decisions

### D1. Correctness before the reviewer surface
- W1.1 P0 defects were fixed before further HLD surface work: snapshot-wide
  completeness made every delta uncertain, relation certainty inherited a
  container's deferral, and class-level trimming made contract additions
  unprovable.
- Evidence: PRs #66 (`0450aa5`) and #67 (`ba60cec`); demo `a849afb→3bc3def`
  moved from 1 element/0 relations added to 72/73.
- Rejected alternative: polishing the Focus HLD while the engine still could not
  prove what the PR changed.

### D2. Demo PR disposition
- Closed #63, #53, #48, #24 as demo-only and not required for W1.1/W1.2.
- Merged #65 (`4c79364`) after rebasing, because it held unique
  provider-confirmed retirement evidence not present on `main`.
- Evidence: `gh pr close` results; #65 was conflicting only because `main`
  advanced after it was written.

### D3. W1.2 envelope: compact index, not a larger cap
- Keep each HLD version ≤ 900 KiB (921,600 B).
- Record trimmed elements and relations as 64-bit identity/fingerprint pairs in
  `DesignCoverage.deferred_facts`; resolve one-sided facts from that index.
- Matching fingerprint → unchanged; differing → modified; absent from display
  and index → `uncertain` only.
- Rejected: raising the version envelope to ~3 MiB. Measured full inventory is
  ~2.7–3 MB per snapshot (276 retained elements vs 650 deferred), and the
  published site was already at 62.88 MB of 64 MiB.

### D4. Capacity and retention
- Keep the 64 MiB expanded-byte site cap.
- Default: retire a report when its PR merges. Optional grace is configurable
  (validated 0–90 days; 14 days was the example).
- Authorization lives in `.codeshouldbehave/report-retention.json`; merge time
  comes from the provider (`merged_at`), never from a closed PR state.
- Evidence: retirement already existed as an explicit ledger
  (`.codeshouldbehave/report-retirements.json`); PR #30's 6.83 MB report was
  retired that way.

### D5. Dedupe scope
- Shipped: review artifacts omit rendered `markdown` and `mermaid` from both HLD
  versions; the public report and CLI export regenerate them.
- Measured saving: 442,150 B (216,043×2 + 5,032×2) on a 3,520,417 B artifact.
- Deferred: ID interning (~729 KB of 64-hex IDs), baseline+delta encoding, and
  compressed-byte site accounting. Each is larger schema/UI work and none is
  required by the decisions above.

### D6. Uncertainty presentation
- Keep per-item `uncertain` labels explicit, with deferred-path links, rather
  than hiding or aggregating them.

### D7. Merge and bookkeeping protocol
- Squash merges; one integration PR per workstream.
- Record the provider-confirmed merge commit in a docs-only follow-up PR; never
  commit directly to `main`.
- Evidence: #68 (`bc2e96c`), #69 (`c835908`), #71 (merge record for #70).

### D8. Generator profile versioning
- `csb-hld-rust-static-v6` → `v7` because the serialized coverage schema gained
  `deferred_facts` and comparison semantics changed.
- Comparisons across profiles remain an error by design.

## Situations faced

### S1. `origin/main` would not advance
- Symptom: `git fetch origin main` wrote `FETCH_HEAD` but
  `refs/remotes/origin/main` stayed at `d317058` inside the worktree.
- Cause: the worktree shares a gitdir whose fetch did not update the tracking
  ref.
- Fix: `git fetch origin refs/heads/main:refs/remotes/origin/main`, or an
  explicit `git update-ref`.
- Lesson: verify the tracking ref, not just `FETCH_HEAD`.

### S2. Conflicting evidence PR
- #65 conflicted after #66–#69 landed. Rebase kept both tracker sections and
  amended stale claims ("PR #63 remains unmerged") to "closed as demo-only".
- Lesson: rebase docs appends; never drop provider-confirmed evidence.

### S3. Rebase applied to the wrong branch
- A backgrounded shell ran `git switch` after it had already failed, so
  `git rebase` executed on the previously checked-out branch.
- Fix: verify `git branch --show-current` and the target ref before rebasing.
- Lesson: backgrounded git sequences are not atomic; check state before reuse.

### S4. Report size anatomy
- Measured 3,520,417 B JSON + 3,081,646 B HTML per report.
- Largest avoidable costs: 11,053 64-hex IDs (~729 KB), rendered markdown
  duplicated per version (~432 KB), two full snapshots instead of
  baseline+delta, and HTML pre-rendering the JSON content.
- Gzip: JSON 666,311 B; HTML 214,149 B (~7× smaller).
- Lesson: the inventory is not the size driver; representation duplication is.

### S5. Publication cap and consent
- Site was at 62,877,610 B of 64 MiB (8 reports, avg 7.86 MB).
- Retirement changes consent semantics; the repo requires explicit
  authorization. A base-owned policy file is that authorization.
- Provider projection lacked merge state; `project_pull`/`validate_pr` were
  extended with optional `merged_at`.

### S6. LLD parent equality vs artifact dedupe
- Omitting rendered fields broke the LLD check that compares the artifact parent
  to the immutable stored design.
- Fix: compare immutable facts with rendered fields cleared, then regenerate
  them for export.
- Lesson: an integrity check must compare the invariant, not derived payload.

### S7. Stale test and prebuilt fixture
- `tests/public_report.rs` called `generate_hld` without the focus argument.
- Python LLD fixtures execute `target/debug/csb-onboarding`, which must be
  rebuilt after Rust changes.
- Lesson: compile tests and rebuild bins before interpreting fixture failures.

## Verification

| Suite | Result |
|---|---|
| `cargo test --lib --features server` | 267 passed |
| `cargo test --features server --test public_report` | 18 passed |
| `python3 -m unittest discover -s tests -p "*test.py"` | 269 passed, 6 skipped |

## Open items

1. Regenerate the `a849afb→3bc3def` review with v7 and measure: uncertain count,
   `deferred_facts` size, artifact bytes, and retention effects. Unit tests
   prove the logic; the production artifact has not been regenerated.
2. W3.3: render explicit "trimmed inventory, not an observed change" labels with
   deferred-path links in the Focus HLD.
3. W2.2: pre-flight publication admission against the dynamic retained set.
4. Remaining dedupe: ID interning, baseline+delta, compressed-byte accounting.

## Boundaries

No deployment, release-pin change, retention-limit change, or consent-semantics
change beyond the explicit retention policy file. Merge authorization is never
inferred from a branch or PR being ready.
