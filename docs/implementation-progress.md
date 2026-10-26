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

## 2026-10-26 IST — corrected release prepared

- PR #13 merge verified: `3983c86730f275015a8eebf0a59cd2f55770213a`.
- Trusted build/attestation `40223827336` passed. Published signed release
  `canary-3983c86730f2`; archive, exact hosted signatures and production bootstrap
  verified. Extracted publisher bytes match the corrected source.
- New pin update changes engine identity only. Runtime budget/receipt/workflow
  identities and all scope/consent fields remain unchanged.
- Native sandbox preflight pending. Live test still requires reviewed main pins;
  no direct main write or self-merge performed.
- PR #14: https://github.com/SSC-SDE/CodeShouldBehave/pull/14.
- Native Linux sandbox preflight `40225424957` passed on exact pin commit
  `9808b790ce547c5ab8597b270d29060a5470a5cf`, including required networkless,
  credential-free sandbox startup and actual pinned ELF execution.
- Pin PR ready for review/merge. Remaining acceptance is the live PR-to-Pages
  HLD/LLD/source journey after main selects this verified package.

## 2026-10-26 IST — bounded diagnostics and explicit signed-intent recovery

- Maintenance branch `codex/pages-intent-recovery` isolates recovery from the
  phase-6 canary. PR #15 remains open, unchanged and unmerged at `7e09fc1`.
- Implemented category-only HTTP diagnostics and bounded post-creation status
  404 polling without repeat POSTs. The actual cause of the live creation/status
  404 remains unconfirmed until this diagnostic engine runs.
- Implemented exact-state, expiring, manual-dispatch-only recovery through the
  existing controller workflow. Workflow bytes, budget identities and receipt
  verifier are unchanged. Scope/origin validation and verified served-site readback
  precede a normal signed site_promotion receipt; no ledger reset or deletion.
- Prepared expiring authorization for existing receipt `7323471755`, artifact
  `12626065953`. Runtime cryptographic verification remains mandatory; API reads
  used to prepare this file are not treated as verified signature evidence.
- Public tests: 97 passed, four native-dependent skips. Tests cover plan/stage
  wiring, expiry, state mismatch, scope change, immutable replay, failed readback,
  concurrent receipts, 404 polling and redacted diagnostics.
- Deployment gates: review/merge recovery source; build/verify signed release;
  review/merge pins; manually recover; normal live PR #15 analysis; browser
  HLD/LLD/exact-source acceptance. No live recovery or end-to-end success claimed.

- Recovery source commit: `75b73765675d2d563b7447062a2dc30f1d8a5e77`.
- Maintenance PR #16: https://github.com/SSC-SDE/CodeShouldBehave/pull/16.
  Provider check `wake` passed (run `40269704766`); this is scheduling
  evidence only, not live recovery or dashboard acceptance.
- Provider reconfirmed PR #15 OPEN at the unchanged full head
  `7e09fc19477719b8d9fe2b9548bdc7e3514425fe`.

## 2026-10-26 — signed recovery engine and pin refresh

- PR #16 provider-confirmed merge: `56b23fc7edf29ad4521f3a1ea603d215c75965ef`.
- PR #15 remains OPEN at `7e09fc19477719b8d9fe2b9548bdc7e3514425fe`;
  its branch is unchanged. Maintenance branch `codex/recovery-release-pins`
  isolates engine pins from the phase-6 canary.
- Account UI rechecked at approximately 04:44 UTC: Actions and Packages USD 0,
  Usage blocked. Existing release evidence is valid until 06:28:25 UTC and
  passed the hosted build gate. Runtime budget identity remains unchanged.
- Trusted Linux build/public contract tests/attestation passed in run
  `40270731162`, exact merged source above. Both archive checksums passed;
  runtime signature verified with exact source/signer and hosted runners only.
- Published release: https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-56b23fc7edf2.
- Pins change engine identity only. Native startup preflight pending.
- Signed intent remains deployment `7323471755`; artifact `12626065953` is
  unexpired, with expiry 2026-10-27T03:34:09Z. Recovery authorization expires
  2026-10-27T03:32:33Z. No state reset, live recovery or dashboard success claimed.
- Remaining: reviewed pin merge, manual recovery, fresh PR analysis, and live
  check-link HLD/LLD/exact-source navigation acceptance.

- Production bootstrap passed against the published URLs; extracted
  `scripts/public_ci.py` byte-matches merged recovery source.
- Native sandbox preflight `40271067920` passed on pin commit
  `e3d0cdf9c59a3fdb2baaf0cf384a9890d134edf2`, including signed package
  verification and actual ELF startup inside the credential-free networkless sandbox.
- Pin PR #17: https://github.com/SSC-SDE/CodeShouldBehave/pull/17.
  Ready for owner merge; no main write or self-merge performed.

## 2026-10-26 — live recovery diagnostic and Pages version correction

- PR #17 merge verified: `96667f0e70da4e26e63b4c940bd940f46cb4543f`.
- Manual recovery run `40272099786` verified the signed intent and authorized
  plan, then failed at `pages_create: HTTP 404`. No promotion receipt or reset.
- Publisher token has Pages write permission; Pages workflow configuration is
  present; original artifact remains unexpired.
- Identified compatibility defect: synthetic `csb-run-attempt-generation` build
  versions instead of repository commit SHAs. GitHub's official deploy action
  uses GITHUB_SHA; its issue #383 documents matching 404s for other strings.
  https://github.com/actions/deploy-pages/issues/383
- New intents bind the actual controller SHA. Explicit legacy recovery maps
  only the exact synthetic identity to its verified original run SHA, requires
  that SHA in trusted expiring authorization, preserves artifact/digest/history,
  and verifies served bytes before a new signed promotion receipt.
- 100 public tests passed, four native-dependent skips; added exact mapping,
  unauthorized mapping refusal, original artifact and immutable history tests.
- PR #15 remains open/unmodified. Live HLD/LLD/source verification is blocked
  until this correction is merged, signed and selected by reviewed engine pins.

- Correction commit `d571502`; PR #18: https://github.com/SSC-SDE/CodeShouldBehave/pull/18.

## 2026-10-26 — corrected Pages version release refresh

- PR #18 merge verified: `54b841656901147f77ba405e37cac8e9cead6440`.
- PR #15 remains open at `7e09fc19477719b8d9fe2b9548bdc7e3514425fe`.
- USD 0 Actions/Packages Usage blocked controls rechecked at approximately
  05:07 UTC. Existing release observation remains valid until 06:28:25 UTC;
  runtime budget identity is unchanged.
- Bounded trusted build dispatched: `40272553688`, exact merged source above.
- Signed intent deployment `7323471755` is still latest. Original artifact
  `12626065953` remains unexpired. Explicit original-SHA recovery authorization
  expires 2026-10-27T03:32:33Z. No reset or replacement of signed history.
- Pin preparation uses `codex/pages-sha-release-pins`, isolated from PR #15.
  Release verification, native preflight and pin PR evidence follow below.

- Build/public tests/hosted attestation `40272553688` passed. Both archive
  checksums and exact-source hosted signature verified independently.
- Release: https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-54b841656901.
- Pin diff verified to change engine identity only; workflow, runtime budget,
  scope, consent and recovery authorization are unchanged.

- Native sandbox preflight `40272854755` passed on exact pin commit
  `6e1fe301511ff714c1a82803b8f80a6ad39a9041`: published bootstrap/signatures
  and actual ELF startup in the networkless credential-free sandbox.
- Pin PR #19: https://github.com/SSC-SDE/CodeShouldBehave/pull/19.
  Ready for owner merge; no main write or self-merge. Live recovery and
  HLD/LLD/exact-source acceptance remain pending the reviewed main pins.

## 2026-10-26 — Pages served; empty deployment marker readback correction

- PR #19 merge verified: `4d5bc0cd735e51bdbb4c94be249d35b1625a109f`.
- Recovery `40273022299` successfully created Pages at original commit
  `f64c45f5f64fdeeb33ef3446a5d045d5ece3ebc4`; the creation 404 is resolved.
  Both that run and readback retry `40273194314` stopped before signing a
  promotion receipt because the excluded root `.nojekyll` marker returns 404.
- Independently fetched all 257 manifest entries: only the zero-byte deployment
  marker is absent; all 256 served files match length and digest.
- Browser verified actual public PR #15 HLD component, LLD function catalog,
  process_payment flow and exact added source lines 11–24 at proposal 7e09fc19.
  Baseline f64c45f5 correctly shows absence for this newly added file.
- Fix reconstructs only root `.nojekyll` when its signed size is zero and digest
  equals SHA256(empty). Every report/asset still requires live byte/hash readback;
  complete local site verification and signed promotion remain mandatory.
- Tests: 105 passed, four native-dependent skips. Cases reject nonempty/wrong-hash
  markers, missing reports, other hidden files and corrupted report content.
- PR #15 remains unmerged. Main signed engine refresh/pins still gate runtime
  use of this helper; automatic check-link publication is not yet finalized.

- Corrected production `restore_public_site` passed against live Pages and
  original signed manifest digest
  `3430ffa6d922783f6e1fce9f9294d06b054315110f61d121944b2f1e350c2e7f`: 257
  entries, including full `public_site.verify_site`. This is real network
  readback evidence, not a mocked verification. Hosted signed promotion and
  PR check publication still await the merged, signed engine.

- Correction commit `d934ae9`; PR #20: https://github.com/SSC-SDE/CodeShouldBehave/pull/20.

## 2026-10-26 — marker readback signed release refresh

- PR #20 merge verified: `c35deed322516d80c0b821645b40449dc475c96c`.
- PR #15 remains open at unchanged `7e09fc19477719b8d9fe2b9548bdc7e3514425fe`.
- Fresh browser reload verified account Actions/Packages USD 0 Usage blocked
  and GitHub Free. New release-only observation at 07:12:17Z expires 19:12:17Z.
  Prior expired observation was not reused; runtime budget identity unchanged.
- Trusted build dispatched `40284382933`, exact merged source above.
- Original signed recovery artifact remains unexpired until 2026-10-27T03:34:09Z;
  authorization expires two minutes earlier. Receipt history is unchanged.
- Isolated pin branch `codex/pages-marker-release-pins`; remaining validation
  is signed release verification, native sandbox preflight, reviewed pins and
  hosted recovery/check publication. Prior live HLD/LLD/source evidence remains
  valid for the published exact comparison, not proof of check publication.

- Build/public tests/hosted attestation `40284382933` passed. Archive checksums
  and exact-source hosted provenance passed independent verification.
- Release: https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-c35deed32251.
- Runtime configuration diff verified engine-only; signed budget identity,
  workflow, scope, consent and recovery authorization are unchanged.

- Published production bootstrap passed; extracted recovery helper byte-matches
  merged source. Native sandbox preflight `40284804053` passed on
  `a14c40d78ea67039b78ea88d4a9b7233eb9aa963`.
- Pin PR #21: https://github.com/SSC-SDE/CodeShouldBehave/pull/21.
  Ready for owner merge. Main remains untouched; PR #15 remains open.
- PR #15 is sufficient for first branch/source-navigation acceptance; a wider
  cross-component demo is separate from completing this verified check link.

## 2026-10-26 — signed recovery completed

- PR #21 merge verified: `3e4d2923c304c75a57681b13257e31af0099b696`.
- Manual signed recovery `40285079425` passed all jobs, including actual served
  file readback, hosted attestation and durable promotion receipt.
- New signed state deployment `7326240939`, kind site_promotion, sequence 30,
  promotion_intent null. Original ledger preserved; no reset/deletion.
- Normal PR analysis dispatched in `40285306791` on exact merged main.
- PR #15 remains open at `7e09fc19477719b8d9fe2b9548bdc7e3514425fe`.
  Check-link/browser acceptance evidence follows after actual completion.

- Normal analysis/publication `40285306791` passed all jobs. Signed state
  deployment `7326296300`, sequence 32, records report
  `462de65d977d9965908d1cc8ee87f960e1e35b1a82fc45372b22e63e3b1a3aee`.
- Architecture check `121448420436` exists on exact PR #15 head; conclusion
  neutral, title Incomplete · Review required (coverage gaps retained).
- GitHub replaces Actions-owned details_url with its own check page, despite
  the publisher supplying the correct dashboard URL. Manual correction via
  maintainer CLI refused HTTP 403 (GitHub App required); no bypass attempted.
- Added explicit exact-report Markdown link to check summaries and exact-link
  delivery acknowledgment. Missing/wrong links remain pending; app, head and
  external_id fences unchanged. 106 public tests passed, four skips.
- Browser verified supplied immutable report URL opens Focus automatically
  and loads the four-function LLD catalog. Same published report identity as
  prior verified HLD/LLD/exact-source journey. Check summary fallback still
  requires signed engine rollout; end-to-end PR link is not claimed finalized.

## 2026-10-26 — check summary-link signed release

- PR #22 merge verified: `bb0bfa67cb629a375f50459691b25c7a76f82e6e`.
- USD 0 Actions/Packages Usage blocked rechecked from freshly loaded billing
  page at approximately 07:40 UTC. Release observation remains valid until
  19:12:17Z; runtime budget identity unchanged.
- Trusted build `40287232950` dispatched on exact merged source.
- Pin branch `codex/check-link-release-pins` isolates this update from PR #15,
  which remains open at `7e09fc19477719b8d9fe2b9548bdc7e3514425fe`.
- Signed recovery and report publication already passed. This rollout delivers
  the explicit check-summary link because GitHub rewrites Actions Details URLs.
  Final browser click-through remains pending main pins and controller run.

- Build/public tests/hosted attestation `40287232950` passed; both archive
  checksums and exact-source hosted signature passed independent verification.
- Release: https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-bb0bfa67cb62.
- Pin diff verified engine-only; no workflow/scope/consent/runtime budget changes.

- Published production bootstrap passed; helper bytes match merged source.
- Native sandbox preflight `40287650116` passed on
  `7b9afa5d23795c38b008d418c3ab550a9b79cf6c`.
- Pin PR #23: https://github.com/SSC-SDE/CodeShouldBehave/pull/23.
  Ready for owner merge; no main write or self-merge. Final live check-summary
  delivery and browser click-through require a controller run after main pins.
