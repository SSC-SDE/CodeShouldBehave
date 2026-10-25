# Public implementation progress

This tracks the public Phase 6 continuation; private historical records are not
copied into this repository. Branch: `phase-6`; PR base: `main`.

## 2026-10-24 — context finalized and drift verified

- PR #3: https://github.com/SSC-SDE/CodeShouldBehave/pull/3 — merged
  `14eb601f117de04f7719ffc81143dec343e5c217`.
- PR #4: https://github.com/SSC-SDE/CodeShouldBehave/pull/4 — merged
  `c9a6f78dce6df5bb681a41eced9833df69710a2a`, provider verified.
- Owner accepted eight unsupported paths and unknown hosting; integrations deferred.
- Native context drift against that exact main: unchanged, reuse allowed. Evidence:
  `.codeshouldbehave/setup-proposal/context-drift.json`.
- Fixed exact manifest-file root recognition during drift; regression verifies
  existing manifest acceptance and missing manifest rejection. Three drift tests
  and Rust 1.87 strict all-target/all-feature Clippy passed.
- Next: merge correction, fresh budget observation, Linux release refresh and
  signature/native startup verification. Current engine pins remain unchanged.
- Activation OFF: export permission, review defaults, publication consent and
  analysis/publication/check operations remain false; consumer workflows staged only.

## 2026-10-24 — refreshed Linux release

- PR #5 merged: `39b89ac088beb8823da738e9587c6ab07c315676`.
- Current GitHub Free / USD 0 Actions and Packages hard stops verified by UI;
  fresh twelve-hour observation passed the hosted guard on build attempt 2.
- Release build and hosted attestation passed: run `40087980741`, attempt 2.
- Package hashes and exact-source hosted signatures verified. Canary tag:
  `canary-39b89ac088be`. Engine pins updated on `phase-6`, not main.
- Published bootstrap passed. Native sandbox startup passed in run `40088281103`
  on exact pin commit `e90ef4b6317451a4f6a0bd55155dc561109d6bcc`.
- Next checkpoint: review/merge the pin PR, then prepare the installation and
  consent diff for explicit owner approval. Do not activate on merge.
- Activation remains OFF; publication consent, export permission, defaults and
  runtime operations unchanged. No consumer workflows installed.

## 2026-10-24 — installation/consent review prepared

- PR #6 provider-confirmed merge: `bd5c06f6566c9523e48dddcefc10f85caf4c6236`.
- Prepared unapplied six-file installation patch, byte manifest and separate
  consent preview under `.codeshouldbehave/setup-proposal/installation-review/`.
- Pages read-only check: workflow source, expected project URL, status null.
- Export context regeneration, reviewed public budget proof, final hashes and
  complete activation diff remain pending explicit owner consent.
- Activation remains OFF. No installer execution or runtime workflow placement.

## 2026-10-25 — final activation prepared

PR #7 merge verified: `0e5403557bf36bb01aede19eb0d28062dbf93716`. Owner
requested activation. Fresh native export context retains the approved boundary,
gaps and deferred integrations. Verified-runtime installer prepared seven runtime
files including the public budget observation, policy and context. 82 public
tests passed, four skipped; config/hashes/budget and main-only Pages access pass.
See docs/ACTIVATION_STATUS.md. Pending final activation PR merge and actual live
PR-to-dashboard verification; no live completion claim.

## 2026-10-25 — live canary blocked, no success claim

PR #8 merged at `ad25c3feff864efb14c3187a08891e72bd588292`. Automatic
nudge and controller delivery verified with review-only PR #9. Budget evidence
was still fresh; no refresh was fabricated. Empty-queue run 40127428437 passed.
Canary run 40127500835 attempts 1 and 2 failed before producing a public report.

Read-only Linux probe 40127927031 identified tree_inventory/lstat racing native
temporary-file replacement. Active sampling now tolerates vanished entries only;
final inventories remain strict and links remain refused. Public tests: 83 passed,
four native-dependent tests skipped (Unix-socket suite requires local permissions).

Probe 40128351722 classified a second failure as exporter/limit. Local exact
comparison succeeded with 22 flow edits; local public export reproduced
Public projection file/byte limit, including with max_details=8. No budget was
raised, no report was published, and no HLD/LLD check link has passed acceptance.
PR #9 remains DO NOT MERGE: it includes synthetic unlinked code and diagnostic
workflow changes. A bounded public projection fix and fresh verified release are
required before the live journey can pass. Raw native stderr was not published;
diagnostics emitted only fixed categories and code locations.

## 2026-10-25 — exporter budget correction

PR #9 is repurposed as the worker/exporter fix: synthetic src/review_canary.rs
and diagnostic workflow changes are removed from its final diff. Previous
DO NOT MERGE warnings refer to the earlier canary contents.

Reserve a 512 KiB incremental detail/source allowance after structural projection,
and bound duplicate Markdown to a labeled summary when over 256 KiB. Original
8 MiB bundle and strict final validation stay unchanged. Changed-file entries
are prioritized among retained declarations. The exact original canary artifact
now exports and verifies locally at 7,679,364 bytes, with HLD and LLD catalog plus
64 detail aliases and explicitly incomplete coverage. Fifteen exporter tests pass,
including a large declaration inventory; strict all-target/all-feature Clippy passes.

Important remaining limitation: the original native catalog omitted the canary
file under its one-MiB declaration budget. Export cannot recover omitted native
entries. Changed-file prioritization must also happen during native indexing
before the actual canary LLD journey can be accepted. No live successful report
is claimed; deployed release still predates these fixes.

## 2026-10-25 — PR #9 native changed-path indexing and browser verification

- Prioritize authorized changed paths, including rename old paths, before native
  LLD input/declaration bounds and one-MiB catalog trimming. Existing unprioritized
  API remains available; priorities cannot expand scope. Catalog identity binds
  retained entries and coverage. Limits remain unchanged.
- Prioritize changed paths in the exported catalog before browser pagination.
  Browser testing found that prioritizing detail allocation alone left Focus's
  first page empty; the published order now fixes that additional blocker.
- Exact historical canary replay: baseline `ad25c3feff864efb14c3187a08891e72bd588292`
  to proposal `e3590e9740124053ccf6b7e4d1185884fa795e06`. Synthetic code remains
  absent from the final PR diff. All four changed functions survive export.
- Verified public bundle `359f424a626e8fae540b53fa98e3fd805bee07a3405cc9220bce381d1b70b36f`:
  7,488,060 bytes, below the unchanged eight-MiB bound. HLD evidence and LLD
  details validate. `process_payment` contains 33 nodes/35 edges before Focus
  value contraction; source routes resolve to exact proposal evidence.
- Local published-site browser journey passed: Focus HLD -> Zoom to LLD -> four
  changed functions -> process_payment Mermaid -> denial node -> exact source
  lines 11–24 and selected line 13. HLD/LLD continue to report incomplete scope,
  unresolved calls and unverified runtime effects; this is not a clean verdict.
- Validation: 21 LLD tests, 16 public-report tests, 83 public Python tests passed
  (four native-dependent Python tests skipped on macOS); Rust 1.87 strict Clippy
  all-target/all-feature and no-default-feature profiles passed.
- Deployment gate remains: merge reviewed PR #9, refresh the signed Linux release
  and pins under fresh USD 0 evidence, then rerun the live PR-to-Pages check.
  The deployed old engine cannot load these branch fixes. Local browser success
  is not recorded as live production acceptance. No merge performed.

## 2026-10-25 UTC — PR #9 merge and signed Linux refresh

- Provider verified PR #9 merged at `6b2734e73ee14257858394ad67b672d0487b4d2f`.
- Fresh GitHub Free and account-wide USD 0 Actions/Packages hard stops observed;
  renewed twelve-hour release proof passed the hosted guard.
- Run `40221410115` passed build, public contracts and hosted attestation.
- Published `canary-6b2734e73ee1` after archive and exact source/signer verification.
- Updated both active engine configurations and runtime budget proof on phase-6.
  Installed input/config/budget validation passed. No scope/consent expansion.
- Next: native sandbox preflight; review/merge pin PR; bounded live canary check
  and Focus HLD/LLD/source journey. Live acceptance remains pending main pins.

- PR #10: https://github.com/SSC-SDE/CodeShouldBehave/pull/10 (phase-6 -> main).
- Native Linux preflight `40221747369` passed on pin commit
  `84ab8516e63d779dcd52936b7e02e5431437b469`, including real binary execution
  inside the required credential-free/networkless sandbox.
- Pin PR ready for owner review/merge. Runtime proof expires `2026-10-26T06:28:25.632679Z`;
  refresh from a new real observation if the merge/live run occurs later.
- Live verification blocked on the pin merge by exact-main controller trust;
  no bypass, direct main push or merge performed.

## 2026-10-26 IST — live trigger verified; activation continuity repair

- PR #10 provider-confirmed merge: `3912a85ddbcb9d91d18dc0638f0955cdba35fafc`.
- Current USD 0 evidence was freshly checked during the preceding release work;
  no account controls changed. Opened bounded live canary PR #11 at `b7bcc72`.
- Nudge `40222017944` passed. Controller `40222027727` refused before analysis:
  `budget policy changed; recent activation is required`.
- Root cause: the PR #10 runtime observation refresh changed the budget identity
  bound to existing signed state. Release proof freshness and runtime activation
  continuity are separate; the earlier pin handoff incorrectly treated them alike.
- Restore the exact original runtime observation and budget block from `6b2734e`,
  preserving the refreshed signed engine pins, all hard limits and signed state.
  Existing regression explicitly tests that an attested activation survives proof
  expiry only for an unchanged policy; fresh initial activation still rejects expiry.
- PR #11 repurposed as this minimal repair; synthetic fixture removed from final
  diff. No state reset, guard bypass or main merge performed. Live HLD/LLD/source
  verification remains pending repair merge and a new bounded canary.

## 2026-10-26 IST — live native export passed; artifact layout correction

- PR #11 merged at `ded7e3119a9243cec84b8692b58c85a8c184f9fc`.
- PR #12 fixture head `9653cd8a031568367d019231dbcffb3b572b6409` triggered
  nudge `40222407518`; controller `40222380852` passed plan and native analysis.
- Native worker generated verified neutral/partial report
  `b702295aa536bf031ee1c9c2d99d7b1eef0019e4f4edcb5b40fa0e43f3dadf41`.
- Publication failed: pinned download-artifact flattens a single matched artifact
  into handoffs/ despite merge-multiple:false. Publisher expected a named child;
  it recorded missing analysis, and REST audit then refused missing worker evidence.
- Normalize only the single flat case using the exact trusted plan request; reject
  mismatched identity, extra files and symlinks. Existing multiple-artifact folders
  remain unchanged. Update installed workflow, template and pinned workflow hash.
  Engine pins, budgets, signed activation and digest validation remain unchanged.
- Four workflow regression tests pass; public suite 87 pass/four native skips.
- PR #12 now contains the correction; synthetic fixture removed from final diff.
  A new engine release is not required for this workflow-side correction. Main
  merge is required before the live controller can run it. No dashboard link,
  HLD/LLD browser or source navigation success is claimed for this live attempt.

## 2026-10-26 IST — preserve receipt workflow identity

- PR #12 merge verified: `0bf14b0c197fe7f58e222b2c4263b04b0ae77dfb`.
- Fresh fixture PR #13 at `f44845d` triggered the controller. Run `40223282098`
  stopped restoring receipts; no live dashboard was published.
- The prior workflow-side fix changed the hash to which existing signed receipts
  are bound. Restore the exact original controller/template bytes and config hash;
  move singleton handoff normalization into the signed engine's publication stage.
  This keeps historic receipt validation strict instead of introducing trust aliases
  or deleting/resetting ledger state. The live workflow fix was therefore incomplete.
- 87 public tests pass/four native skips, including singleton/multiple/invalid
  handoffs and the original workflow identity. Synthetic fixture removed from the
  final PR #13 diff. No scope, activation, budget or receipt-guard relaxation.
- Required next sequence: merge reviewed source PR #13; build/attest a new engine;
  update pins through review; then rerun live acceptance. Current release lacks
  engine-side normalization. HLD/LLD/source live verification remains unconfirmed.
