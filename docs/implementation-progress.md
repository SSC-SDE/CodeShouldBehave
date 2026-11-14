# Public implementation progress

This tracks the public Phase 6 continuation; private historical records are not
copied into this repository. Branch: `phase-6`; PR base: `main`.
The standalone landing-page publication below uses `codex/public-landing`;
it does not advance, merge or activate the Phase 6 App implementation.

## 2026-11-14 — hosted review desk release cutover

Owner authorized fixing the hosted/local UI mismatch while preserving all approved
design decisions. Release source is merged main `60b46619aefac3a18e0e7cfcd02a21f80b9ddfe5`.
The Standard Character desk/Ring and Focus review desk/Moria artwork ship unchanged;
native LLD and PR-check Focus entry remain intact. Draft PR #53 stays unmerged.

Fresh authenticated billing UI observation confirmed GitHub Free and account-wide
Actions/Packages USD 0 budgets with Usage blocked. No billing controls were changed.
Only the independent release-build proof was refreshed; the activated runtime
budget identity, scope, consent and controller authority remain unchanged.
Trusted Linux build: https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/43605356067.
Build and hosted attestation passed. Downloaded source/runtime checksums and
offline provenance passed with exact source/signer SHA and hosted runners required.
Release: https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-60b46619aefa.
Both engine pin documents now select that verified package. Native preflight and
live publication evidence remain pending.

## 2026-11-14 - Standard Character desk implementation

Approved reference: Character A+B in the local Ring/Standard prototype. Work is
isolated to `ui/standard-review-desk`; the unrelated App/backend checkout is not
part of this UI change. The owner subsequently authorized publication and merge
of the verified changes. Deployment, activation and release-pin changes remain out of scope.

Implemented the Standard-only review desk with a persistent authorized PR queue,
native sans typography, paginated DTO-backed change summaries, bounded structural
reference maps, exact-side HLD-to-LLD mapping, source identity validation and an
evidence/scope disclosure. Per-PR positions remain local to page memory; evidence
is reauthorized on navigation. The existing Focus and native LLD renderers remain
the authority for their respective views. No synthetic author-claim text, sample
review questions, invented runtime findings or example source enters the new desk.

HTML loads the Standard and Ring assets before workspace preferences mount.
Ring visibility uses the native registry; the pause button is outside its inert
decorative host. Reduced-motion, forced-color and unavailable-artwork behavior
remain presentation-only.

Added focused JavaScript regression cases for uncertain and relation-only deltas,
bounded-map reachability, exact-side LLD ownership, immutable source identity,
per-review positions and stale/current authorization failures. Verification:
- JavaScript: 58 Standard/Focus/workspace tests and 15 preference tests passed.
- Python 3.14: 2 workspace asset tests and 129 public-host/package tests passed.
- Native `--features github-app`: 6 preference, 1 exact asset-route and 3 report-host tests passed. Initial commands without feature flags selected no tests and are not gate evidence.
- Actual source UI served at localhost:3224 over the existing published fixture: Changes → HLD → native LLD → exact baseline source, Focus round-trip, Ring pause/off/cancel and reduced-motion static mode verified. At 390px viewport, document width was 390px.
- Browser reported no JavaScript exceptions; one evidence request was aborted during navigation/reload. The fixture contains one PR, so multi-PR position isolation was covered by regression tests rather than a multi-PR browser session.
- Standard queue hides redundant timestamps, status and commit metadata; the exact comparison remains available under Evidence & scope.
- Integration PR: https://github.com/SSC-SDE/CodeShouldBehave/pull/51; implementation commit `7d55ea69835cb66995b5752df13744d7f0d8f5d6`, base `7ea5246992372db6d438364356e20ff3dcded5c6`.
- GitHub reported mergeable/CLEAN with successful `wake` nudge check. No branch protection/rulesets or required approvals are configured. The nudge is not test CI; the local test and browser evidence above are the verification gates.
- Owner-authorized PR #51 was confirmed `MERGED` by GitHub at `2026-11-14T13:51:15Z`, squash merge commit `9ee885293f079ecc9d267d5a31f731c6cfbcde00`. This documentation-only follow-up records that provider identity; no direct push to main, deployment or release-pin change.

## 2026-11-08 — public canary activation preparation

Owner completed the local HLD/LLD review and authorized continuation of the
bounded public canary. The current GitHub account budget page was observed with
Actions and Packages at `$0.00 / $0.00`, `Usage blocked`, and Free plan status.
The observation was refreshed in `.codeshouldbehave/budget-observation.json`;
runtime config validation passed. The existing pinned controller/nudge files are
already present on `main`; no beta launch is claimed.


## 2026-11-08 — canary budget identity preserved

The first controller run after PR #41 correctly refused before analysis with
`budget policy changed; recent activation is required` (run
[42488843241](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42488843241)).
The signed controller state binds the original budget policy and evidence
identity, so the refreshed observation is not used to rewrite that identity.
This correction restores the exact previously activated budget configuration;
no state reset or guard bypass is performed. A subsequent controller run is
required before claiming live canary acceptance.

The trusted Linux build then passed as run
[42489528360](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42489528360)
for merged source `958a764ec19bd492039b568b8d31387842adfd4e`. Release
`canary-958a764ec19b` contains the source/runtime archives, attestation,
manifest and checksums. The follow-up config pin update is required before the
controller can use the redesigned workspace; no Pages update is claimed yet.



## 2026-11-06 - public landing page

User requested pushing and deploying the landing page. Its implementation was
isolated from `5df2e62d128c2507ab65da76c729839c723db825` onto current public
`main`, `c52b5527d8d480c5b9e62863fe3ff4e94cf4bab5`, in a separate worktree.
Branch: `codex/public-landing`. The four local App/backend continuation commits
and the existing [draft PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32)
remain unchanged. This is a standalone marketing-site release, not a new phase
or an alternative Phase 6 integration PR.

Audit: deployed `/CodeShouldBehave/` is only a workspace redirect, with no
description or social metadata. Preserve `/workspace/`, immutable report links,
existing branding and the dark reviewer UI. Landing work uses native CSS,
self-hosted JetBrains Mono, graphite/blue tokens and system light/dark preference.
No framework or design-system migration, invented customer proof, App install
CTA, deployment, activation, credentials, release pins or budget changes.

Implemented: static product introduction, actual published PR24 HLD/LLD/source
captures, existing `[!]` brand favicon, system light/dark and reduced-motion
preferences, self-hosted type, accessible keyboard entry and explicit setup
availability. Zero landing scripts or trackers. Site/source/runtime inventories
include only the declared landing assets; unapproved neighboring files stay out
of the published site. Workspace/report paths and release controls are unchanged.

Original UI evidence at `5df2e62` (retained in `CodeShouldBehave-public/target/`):

- 121 public Python regressions pass, no skips. Existing HTTPError cleanup
  ResourceWarnings remain visible. New tests cover unapproved neighboring
  assets and missing/symlinked/oversized landing inputs, not page wording.
- Actual Chrome: widths 320, 390, 768, 1024 and 1440; both themes at desktop
  and mobile. No horizontal overflow; desktop headline two lines, subtext at
  most four, hero/CTA visible. All three image decodes observed. Keyboard skip,
  section anchor, reduced-motion behavior, documentation/source/notices links
  and landing-to-workspace navigation exercised. No page exceptions.
- CTA also opened a locally assembled real published PR32 report, bundle
  `a1f293a0cfc5bc0ab2e4dc3ef5d0186f6e419edc9291165af7ad5f65fb6cf7e3`.
  All 235 files matched the public manifest and the native semantic verifier
  accepted the complete bundle before assembly. Selecting its card retained
  exact review/bundle identities. Provider freshness is intentionally unknown
  because this preview did not invent a new GitHub observation.
- Lighthouse 12.8.2 against local Python HTTP: desktop performance 100,
  mobile 99, accessibility 100 and best practices 100 in both runs. LCP:
  desktop 0.48s, simulated mobile 2.25s; CLS 0 and TBT 0. SEO 92 because the
  preview origin has no valid root robots.txt. No origin-wide crawler policy,
  cache or compression changes were made. These are local lab results, not
  deployed field Core Web Vitals or an INP measurement.
- Hero body contrast: 7.49:1 light, 9.06:1 dark. CTA contrast: 6.42:1 light,
  8.92:1 dark. Final desktop/mobile captures and Lighthouse JSON are retained
  under ignored `target/landing-final-*` and `target/landing-lighthouse-*-final.json`.

Landing-only publication evidence:

- Pushed implementation commit `914254fc727980eb345ef756524e1858d13dd476`
  to `codex/public-landing`; GitHub's ref API confirmed that exact identity.
  No App continuation commits were pushed and PR #32 remains unchanged.
- Re-ran all 121 public Python tests on the isolated main-based tree: pass,
  no skips. Existing HTTPError cleanup ResourceWarnings remain visible.
- A clean-source Rust 1.87 macOS release build produced the existing three
  public-profile binaries, with 108 locked license packages and zero findings
  under the declared credential-pattern rules (not exhaustive detection).
  Local source archive SHA-256:
  `d42bd32c8e578cc21c105d64c0750904460fb56bb518f956f794cc0369b4d656`;
  runtime archive SHA-256:
  `5ce59502034480ee147417c14fd5889592f2cd520a81bdc355de97cff2b66f0c`.
- The extracted landing-only runtime's verifier and site assembler accepted the
  real 235-file published report and generated an integrity-verified
  11,403,352-byte site. Actual Chrome loaded all images and the self-hosted
  font, then opened its PR32 report through the primary CTA and selected card,
  retaining exact review/bundle identities with no page exceptions.
- These are local macOS artifacts, not uploaded releases or signed Linux
  qualification. No remote CI, PR, deployment or merge was performed.

Image provenance and preview commands:
[public usage](PUBLIC_MVP_USAGE.md#public-landing-page).

## 2026-11-06 - landing release deployment

- User authorized landing push and deployment. Landing-only
  [PR #33](https://github.com/SSC-SDE/CodeShouldBehave/pull/33) merged at
  provider-confirmed `fbe4b73340778311965d6bbf24919beb969c1c71`.
  Draft App PR #32 and its unpublished continuation remain separate.
- Authenticated GitHub UI confirmed GitHub Free and account-wide Actions and
  Packages USD 0 budgets with Usage blocked. Existing controls were not changed.
  Fresh release proof observed at `2026-11-06T20:29:16.428Z`, expires
  `2026-11-07T08:29:16.428Z`; the actual release guard accepted its exact bytes.
  Only the existing release proof secret/digest and release source variable
  were refreshed. Runtime budget identity and its historical observation remain
  unchanged because they belong to the existing signed activation ledger.
- Trusted Linux build and hosted attestation
  [42288835561](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42288835561)
  passed from the exact merge source. Both archive checksums and offline
  signature verification passed with exact source/signer commits and hosted
  runners required.
- [Canary release](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-fbe4b7334077)
  published. Linux runtime SHA-256:
  `d780fc5cc5a44bf43fe1f7a76c04df95ffc88d5a5768e33526d9fdb7dc1b5870`;
  offline proof SHA-256:
  `691c557c7b9344da489c9fcd4f11a3e752079025605b4a9de3fb0b909c5028c9`.
- Only engine identity changes in public-ci/engine-pins. Production bootstrap
  from the published URLs passed signature/hash/ELF checks and extraction.
  All 121 public Python regressions passed, no skips; existing ResourceWarnings
  remain visible.
- Native Linux sandbox [preflight 42289513084](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42289513084)
  passed on exact pin commit `40f12bcd99db2e7270333cf47c709c1438f42290`:
  signed Ubuntu Bubblewrap, empty credential-free/networkless sandbox, pinned
  release verification and actual ELF execution inside that sandbox.
- [Pin PR #34](https://github.com/SSC-SDE/CodeShouldBehave/pull/34) merged at
  provider-confirmed `134c895ef4baba5f50427906d432fc66a2bca462`.
  Manual controller 42289665798 was cancelled as the normal shared concurrency
  slot selected automatic run
  [42289687915](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42289687915)
  on the same exact merge. Staging failed before deployment; local replay of
  those exact inputs confirmed `Site cap exceeded; preserve previous generation`.
  Earlier source-fbe4b73 controller refused after main advanced; its current
  workflow/head checks were retained, not bypassed.
- No App execution grant, policy, source scope, consent, workflow, runtime
  budget identity, billing control or existing report has been changed.

## 2026-11-06 - explicitly approved obsolete report retirement

The user selected retirement rather than increasing capacity, then explicitly
approved these two redundant versions:

- PR24 older-scope bundle
  `7519517202cfdf944deb441c07cfece712f3e8908ca4c23700b70ee80f07a02c`.
- PR31 superseded-commit bundle
  `aa78ab56631a941bcd4b78ec3e7058615adf2fd5ae5af92637a578651906b46b`.

The current PR24 demo `4b0d25a5…` and current PR31 `3d77c185…` stay available.
Authorization is a bounded, repository-bound, base-owned exact-ID document,
not PR input or direct deletion. The publisher commits the existing monotonic
withdrawal ledger in its signed intent before deployment, emits expiry records,
and cancels any attempted successful check for a retired bundle. No source
scope, budget identity, receipt validation or 64 MiB site bound was relaxed.

Local verification: all 125 public Python regressions pass, no skips, including
actual site-byte removal, unselected report preservation, repeated retirement,
invalid/foreign/linked approval, signed intent retention and dead-link refusal.
The former mocked scope-withdrawal forwarding test now exercises real site output.

Exact failed-run replay used the digest-pinned previous Pages artifact from
run 42179183640, both real handoffs from 42289687915 and the actual macOS native
semantic verifier. The signed empty `.nojekyll` marker was reconstructed under
the existing restore rule; no missing source file was fabricated. Without
retirement: the exact capacity failure reproduced. With only the approved two
IDs: native verification and site integrity pass, six other existing reports
remain and the seven-report site occupies 55,894,576 bytes. Local snapshot hash:
`5db5d3da2e11e3e45180c48479865e97719112279636712ffcfcbe9d8b9b98c4`.
No live report was removed by this replay.

[Retirement PR #35](https://github.com/SSC-SDE/CodeShouldBehave/pull/35)
merged at provider-confirmed `5fb135fb7f3f550cb4373df8830679927cd0ced1`.
Trusted Linux [build and attestation 42292639339](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42292639339)
passed, including all 125 publication regressions on the actual builder.
[Signed canary](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-5fb135fb7f3f)
published after both archive hashes and offline exact-source/signer verification.
Runtime SHA-256:
`6c950ebc099075508b76471fe1e6515340f5848a21b71bd767a97591695564fd`;
offline proof SHA-256:
`0864e8909be527495349f9a63e12e1f11aa28ada645e66f603e642ea9f127e17`.
Production bootstrap from the published URLs passed.

## Landing page deployed and approved history retired

- Replacement native preflight
  [42295408429](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42295408429)
  passed on exact pin commit `ab523127b725dadca5b42e6ca9022f8b0d22b79f`.
  The local watcher timed out; direct GitHub job verification confirmed success.
- [Pin PR #36](https://github.com/SSC-SDE/CodeShouldBehave/pull/36) merged at
  provider-confirmed `0c7da4cb8cf1b97e6c061490cce9915b642431dc`.
- Controller 42307917844 exhausted its REST ticket during pre-deployment staging.
  No request limit was increased or counter reset. The next automatic pass,
  [42309306519](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42309306519),
  succeeded on the same source/config, including complete live readback,
  durable receipt commit, check delivery and the fixed REST audit.
- Signed intent record `7612878142` persisted both withdrawals at sequence 61.
  Verified promotion record `7612888556` is sequence 62, site generation 12,
  Pages deployment version `0c7da4cb8cf1b97e6c061490cce9915b642431dc`.
  The production cryptographic verifier accepted exact repository, source,
  workflow, main ref, hosted runner and invocation identity.
- Live site manifest SHA-256
  `8db9c0186d3ba946ac185c65c0b336862431b44a790a0776e80ee6a3283951d4`
  matches that signed receipt. Inventory: 1,737 files, 55,894,647 total bytes,
  seven reports and exactly the two approved expiry records.
- HTTP verification: all six other original report manifests return 200 and
  match their previous SHA-256 identities. Both retired report manifests return
  404 and their source-free expiry records say `withdrawn`. No other report
  was retired. The 64 MiB site, 172 MiB artifact and 100-call pass limits remain.
- Actual deployed Chrome smoke: desktop 1440 light and mobile 390 dark render
  all three images and self-hosted font, no overflow, visible hero/CTA and zero
  landing scripts. Primary CTA opens the seven-report workspace. Screenshots
  retained under ignored `target/live-landing-*` in the acceptance worktree.
- The retained exact PR24 link opens Focus/HLD, Zoom to LLD and `retry_decision`.
  Selecting function entry opens source with exact proposal
  `b312bbf3c3f01299d04745b73ca3b81d24582a40` and blob
  `621ad2ae5ebfb34e6f72aaf455575a61901f3f89`. A first source wait was placed
  before selecting a node and lost its browser tab; the correct interaction
  was then exercised successfully. No source data or guard was changed.
- App PR #32 remains separate and activation remains off. This is landing/Pages
  deployment acceptance, not hosted App or broader beta qualification.



## Current continuation — local, not pushed

PR #32 was inspected at `6efe23e26118bcd7e5351b3946b7f7fdd49f9faf`.
Implementation is locally committed on `phase-6` at
`14012713ee3a505dd1f1d45bb7b33a049126183c`, followed by App runtime distribution
`6edfc70c1129b4f23b6c384fad3d91e571a2cd3b` and progress-quota fairness correction
`f041715cb9a2498b603fcac412d026dedd511cf9`. None has been pushed; remote CI,
review and merge are not inferred.

Discovery, operator activation validation, check lifecycle UX and exact offline
recovery have passed local checks. Real App activation remains OFF. Remaining:
qualified USD 0 Linux host/HTTPS/key stores, freshly signed worker qualification,
independently fresh recovery authority/fencing, remote PR gates and live
fresh-repository acceptance. See the final dated entry and
[execution contract](APP_EXECUTION_DEVELOPMENT.md).


## 2026-11-06 - public landing page

User selected the public landing page after invoking the frontend design skill.
Resuming existing public `phase-6` / [draft PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32)
from local documentation commit `b3d3020`. Remote PR head remains
`6efe23e26118bcd7e5351b3946b7f7fdd49f9faf`; no new branch or PR is needed.

Audit: deployed `/CodeShouldBehave/` is only a workspace redirect, with no
description or social metadata. Preserve `/workspace/`, immutable report links,
existing branding and the dark reviewer UI. Landing work uses native CSS,
self-hosted JetBrains Mono, graphite/blue tokens and system light/dark preference.
No framework or design-system migration, invented customer proof, App install
CTA, deployment, activation, credentials, release pins or budget changes.

Implemented: static product introduction, actual published PR24 HLD/LLD/source
captures, existing `[!]` brand favicon, system light/dark and reduced-motion
preferences, self-hosted type, accessible keyboard entry and explicit setup
availability. Zero landing scripts or trackers. Site/source/runtime inventories
include only the declared landing assets; unapproved neighboring files stay out
of the published site. Workspace/report paths and release controls are unchanged.

Local evidence:

- 121 public Python regressions pass, no skips. Existing HTTPError cleanup
  ResourceWarnings remain visible. New tests cover unapproved neighboring
  assets and missing/symlinked/oversized landing inputs, not page wording.
- Actual Chrome: widths 320, 390, 768, 1024 and 1440; both themes at desktop
  and mobile. No horizontal overflow; desktop headline two lines, subtext at
  most four, hero/CTA visible. All three image decodes observed. Keyboard skip,
  section anchor, reduced-motion behavior, documentation/source/notices links
  and landing-to-workspace navigation exercised. No page exceptions.
- CTA also opened a locally assembled real published PR32 report, bundle
  `a1f293a0cfc5bc0ab2e4dc3ef5d0186f6e419edc9291165af7ad5f65fb6cf7e3`.
  All 235 files matched the public manifest and the native semantic verifier
  accepted the complete bundle before assembly. Selecting its card retained
  exact review/bundle identities. Provider freshness is intentionally unknown
  because this preview did not invent a new GitHub observation.
- Lighthouse 12.8.2 against local Python HTTP: desktop performance 100,
  mobile 99, accessibility 100 and best practices 100 in both runs. LCP:
  desktop 0.48s, simulated mobile 2.25s; CLS 0 and TBT 0. SEO 92 because the
  preview origin has no valid root robots.txt. No origin-wide crawler policy,
  cache or compression changes were made. These are local lab results, not
  deployed field Core Web Vitals or an INP measurement.
- Hero body contrast: 7.49:1 light, 9.06:1 dark. CTA contrast: 6.42:1 light,
  8.92:1 dark. Final desktop/mobile captures and Lighthouse JSON are retained
  under ignored `target/landing-final-*` and `target/landing-lighthouse-*-final.json`.

Implementation and native package smoke passed at exact local commit
`5df2e62d128c2507ab65da76c729839c723db825`. The isolated Rust 1.87 release build
produced all eight binaries for `aarch64-apple-darwin`. Source archive SHA-256:
`0ca1182a8ab47779f136f21366a3f6c779b33a2d1f13afe92db2cc9cd69b1c47`;
runtime archive SHA-256:
`0f97b63a15eaf97ed99326122116455d231120a4e6a9aba58c6dc27b3cc4ef47`.
Archives and manifests are retained in ignored `target/landing-runtime-5df2e62/`.
The package inventory retains 108 locked license packages and reports zero
findings under its declared credential-pattern rules, not exhaustive detection.

The extracted runtime's own native verifier and Python assembler accepted the
real 235-file public bundle and generated an integrity-verified 11,403,352-byte
site. Actual Chrome loaded all three images and the font from that packaged
site, then followed the primary CTA and selected the exact PR32 review/bundle
successfully, with no page exceptions. This is local macOS packaging proof,
not signed Linux, hosted App, deployment or beta qualification. Temporary raw
capture files were removed. Image provenance and preview commands are in
[public usage](PUBLIC_MVP_USAGE.md#public-landing-page).
No push, deployment, App activation, merge or launch is authorized by this work.

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

- User requests a single deployment PR and deployment now. PR #23 also updates
  explicit recovery authorization to signed state deployment `7326385331`,
  sequence 33, artifact `12633562944`, from run `40286001442`. Pages currently
  serves its exact manifest `27e2724beb927014573bfd1dc2d7e7eee4d34cfe62b819d607e6ebb82c8d818f`;
  original run checked too early and safely refused stale readback.
- Authorization names exact state/intent hashes with twelve-hour expiry; it
  neither replaces history nor grants arbitrary artifact recovery. Current
  SHA-based intent uses resume_exact_intent, without legacy commit remapping.

## 2026-10-27 — meaningful scheduler demo and evidence retention

- PR #23 merge verified at `ed06b6c8d36ce382c55abbb697145981ac124b21`.
- Demo PR #24: https://github.com/SSC-SDE/CodeShouldBehave/pull/24,
  head `b312bbf3c3f01299d04745b73ca3b81d24582a40`. Adds a local retry-policy
  package and changes the existing scheduler failure path: bounded Retry-After,
  deterministic jitter, attempt limit and explicit overflow failure. It preserves
  lease validation and transactional job updates. PR #15 remains unmerged.
- The wider comparison exposed HLD byte pruning that discarded dependency
  topology before the contract inventory. Generator v4 defers contracts first,
  retaining topology and source evidence within the existing bounded storage.
- Public export now bounds duplicated Markdown prose while retaining exact JSON
  evidence. Catalog and source allocation prioritize modified public functions
  with both counterparts so added declarations cannot starve the useful diff.
- Pages finalization may wait for the exact previously signed manifest only:
  at most twelve reads with five-second intervals. Unknown bytes fail immediately;
  desired manifest and every served file still require exact verification.
- Run `40454018716` analyzed PR #24 but refused stale Pages readback. Signed state
  deployment `7350943683`, sequence 40, remains a pending intent. Its hosted
  signature and current served manifest were independently verified. Prepared
  exact-intent recovery authorization expires at 2026-10-27T17:19:38.924970Z;
  no recovery dispatch or engine deployment is claimed here.
- Fresh Actions and Packages USD 0 / Usage blocked controls checked in billing
  UI at approximately 05:07 UTC. Runtime budget identity, workflow, installation,
  scope, consent and permissions remain unchanged.
- Validation: 221 Rust library tests, 18 public-export integration tests,
  114 Python publication tests (four skips), 85 dashboard tests, strict Clippy
  with all features and no default features, formatting and diff checks passed.
- Local browser acceptance on the exact demo head: Focus HLD split shows the
  added csb-retry-policy dependency; LLD opens Scheduler::fail first and shows
  both flows. Clicking proposal retry_decision shows baseline scheduler.rs
  lines 402–445 at ed06b6c8 and proposal lines 407–453 at b312bbf3, with the
  selected call at line 434. Partial coverage remains explicit.
- Local public bundle: `932756172a88be745bf6b1edc502f393e6fe14d434ac3a16b4d5a325d5157200`.
  This is local acceptance, not successful hosted PR-check acceptance. Remaining:
  owner-authorized engine merge, signed release/pins, signed recovery, fresh PR
  analysis and browser verification from the actual GitHub check-summary link.
- Consolidated engine fix PR #25:
  https://github.com/SSC-SDE/CodeShouldBehave/pull/25. Source commit
  `ed37223` contains the fixes and local validation above. Explicit owner merge
  approval requested before any main-branch merge or signed engine rollout.

## 2026-11-05 — resume demo deployment

- Owner requested completion of the remaining demo work after the explicit
  PR #25 merge/release checkpoint. Continuing the tested engine rollout.
- GitHub now confirms PR #15 was merged externally on November 1 at
  `adf5c771ad155dd561531c5d82e521768b8b3e83`. This task does not rewrite that
  history. PR #24 remains the unmerged scheduler demo at `b312bbf3`.
- Updated the existing #25 branch with current main; no conflicting code changes.
  Fresh validation: Rust library and public export tests, 114 Python publication
  tests (four skips), 85 dashboard tests and formatting passed.
- Fresh browser observation: GitHub Free; SSC-SDE account-wide Actions and
  Packages each USD 0, Usage blocked. New release-only proof created from these
  observed values. Runtime budget identity remains unchanged.
- Signed state deployment 7350943683, sequence 40, cryptographically verified
  again against its original hosted run; live Pages manifest matches its intent.
  Exact recovery authorization refreshed to 2026-11-06T04:53:51Z.
- Public controller currently refuses scheduled recovery because manual dispatch
  is required. Recovery and final PR #24 browser acceptance follow signed rollout.

## 2026-11-05 — signed demo engine rollout

- PR #25 merged at `00055eaead6173451e73bb63d8dc4a7a80d710e8`, provider verified.
- Trusted build/tests/attestation `42069590687` passed. Signed release
  https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-00055eaead61.
  Archive checksums and hosted exact-source signature independently passed.
- Pin branch `codex/demo-evidence-release-pins` starts from that merged source.
  Only engine identities change; runtime budget and authorization scope remain.
- Manual exact-intent recovery run `42069928076` dispatched on verified main.
  Native preflight and final hosted PR #24 acceptance remain to be recorded.
- Production bootstrap passed against published release URLs; extracted
  public_ci.py byte-matches merged source. Native Linux preflight is next.
- Native Linux sandbox startup `42070465549` passed on pin commit
  `669c390faa6263d92bfaf73bcd28ecf0fcc8ff7c`: verified release download,
  hosted signature and actual binary startup inside the networkless sandbox.
- Manual signed recovery `42069928076` passed all jobs. The old pending Pages
  deployment is recovered; no reset or replacement of signed history occurred.
- Pin PR #26: https://github.com/SSC-SDE/CodeShouldBehave/pull/26.
  Ready for explicit owner merge approval. Live #24 acceptance requires this
  engine selection on main and a fresh controller run.

## 2026-11-05 — live demo deployed; final receipt recovery

- PR #26 merge verified at `7f3c193beadd88069ccaf9aa19cffe501ade61ee`.
- Automatic PR controller `42070853667` analyzed #24 successfully; publication
  refused an HTTPError before state mutation. All 727 signed existing files then
  independently passed bounded parallel readback. One normal retry was dispatched.
- Retry `42072060623` analyzed, staged and deployed the new dashboard; final
  sequential HTTP readback refused an HTTPError before creating the check.
- Signed deployment 7580235538 / sequence 42 records exact intent generation 6,
  manifest `915139824ee08e7f3f92be09538cc2f7641e7750ba05ee2d47932629aba9c4b4`.
  Its signature and live manifest were verified. All 997 manifest entries passed
  independent size/hash readback (signed empty .nojekyll reconstructed).
- Live browser acceptance passed directly on report
  `4b0d25a5a53b9d0229a147a07a784f88ff3055a8b6003c518fd8dd6036abba1f`:
  Focus default; HLD Unified and Split show the new package; Zoom to LLD opens
  changed functions; fail shows both flows; retry_decision opens exact baseline
  scheduler.rs:402–445 at ed06b6c8 and proposal :407–453 at b312bbf3, including
  changed line 434. This is a hosted report, not a localhost preview.
- Refreshed exact-intent approval expires 2026-11-06T05:17:52Z. Recovery must use
  the existing signed deployment; no ledger reset, scope change or engine rebuild.
  Actual GitHub check-summary link delivery remains pending recovery.

## 2026-11-05 — bounded Pages readback repair

- PR #27 merged at `eb6b37e34d6c22c4b18f9ded9c361939fa30bc5c` after explicit
  owner approval. Recovery `42074803971` refused the 120-second site restore
  deadline while reading the 997-file site sequentially. Signed intent sequence
  42 remains intact; dashboard/source browser acceptance remains valid.
- Readback now prevalidates the full signed inventory and aggregate byte cap,
  then uses six workers with at most six submitted reads. Only the coordinator
  writes files; every length/digest and final site validation remain required.
  The same 120-second deadline and 64 MiB site cap remain enforced.
- One retry is allowed only for transient HTTP 429/500/502/503/504 responses.
  Missing files, redirects, corrupted bytes and unsupported markers still fail.
- 119 publication tests passed (four skips), including real concurrency bounds,
  bounded transient retries, whole-inventory byte rejection and deadline cleanup.
- The repaired production restore function independently verified the ACTUAL
  signed 997-file deployed snapshot within its original deadline, including final
  public_site verification. No locally weakened limits or skipped files were used.
- Rollout requires merging this helper repair and selecting its signed package.
  Demo PR #24 remains open and contains the verified direct live report link.

## 2026-11-05 — concurrent readback signed release

- PR #28 merged with explicit owner approval at
  `c46c28a509b649802ab68da2bee7594c2ff15fdd`.
- Trusted Linux build/tests/hosted attestation
  [42082187739](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42082187739)
  passed. [Signed release canary-c46c28a509b6](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-c46c28a509b6)
  is published. All archive checksums and exact-source hosted provenance passed.
- Production bootstrap from published assets passed; the extracted public_ci.py
  byte-matches the merged repair. Only engine pins change in runtime config.
- Fresh USD 0 release observation remains valid until 2026-11-06T04:53:50Z;
  runtime budget identity, workflow, scope and consent are unchanged.
- Pending signed intent sequence 42 and its exact recovery authorization remain
  intact. Deployment requires the pin PR merge; final check delivery is pending.
  Demo PR #24 stays unmerged.
- Native Linux sandbox preflight [42083297591](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42083297591)
  passed on pin commit `451fbccd1cc19fead155dab94b00c1adadab73ea`, including
  published package verification and actual networkless ELF startup.
- Pin PR #29: https://github.com/SSC-SDE/CodeShouldBehave/pull/29.
  Ready for explicit owner merge/deployment approval; no live pin change yet.

## 2026-11-05 — live PR demo acceptance complete

- PR #29 merged with explicit owner approval at
  `a1ca6d35749b5ee66163fff9fc6811455e6af93d`. Signed recovery
  [42083630233](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42083630233)
  passed using the existing exact intent and the released concurrent verifier.
- Automatic controller [42083656278](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42083656278)
  passed analysis, publication, live readback, receipt commit and check delivery.
  Final deployment record: `7581743834`; REST audit: 71/100 calls, complete
  evidence for plan, analyzer and publisher. USD 0 controls remain unchanged.
- [PR #24 Architecture check](https://github.com/SSC-SDE/CodeShouldBehave/runs/127446113035)
  is completed/neutral with title **Ready · Changes to review**. Its summary
  includes the actual public report link. Neutral denotes review is required,
  not a failed analysis and not merge approval.
- Browser acceptance followed that exact check link: Focus loads automatically;
  HLD Unified/Split shows the added csb-retry-policy dependency; Zoom to LLD
  opens seven changed declarations; fail shows baseline/proposal control flows;
  proposal Call retry_decision opens both source diffs and exact line 434.
- Source identities verified: baseline `ed06b6c8d36ce382c55abbb697145981ac124b21`
  scheduler.rs:402–445; proposal `b312bbf3c3f01299d04745b73ca3b81d24582a40`
  :407–453, selected line 434, blob `122707f0a511026cb9450112441b1b20580a98d0`.
- Hosted bundle: `4b0d25a5a53b9d0229a147a07a784f88ff3055a8b6003c518fd8dd6036abba1f`.
  Partial static coverage and unresolved effects remain explicitly visible.
- PR #24 stays open/unmerged; its description now includes the verified check
  entry and a three-minute presenter walkthrough. This completes the requested
  live demo checkpoint, not broader beta readiness or universal runtime proof.

## 2026-11-06 IST — simplified App onboarding planned

- PR #30 provider-confirmed merged at `c877af8ae2d25c2cf2773c70a0c6fa8499ba3d84`.
  PR #24 remains the open live demo; no merge or migration is authorized here.
- Added GITHUB_APP_ONBOARDING_PLAN.md: install/select/scope confirmation target,
  proposed Rust/SQLite service and bounded workers, least-privilege permissions,
  tenant/consent boundaries, exact check links, recovery and APP-0 through APP-6
  checkpoints. Every App checkpoint is pending. Hosted USD 0 feasibility is not
  assumed; no permissions, infrastructure or runtime behavior changed.
- Updated public README, public usage and template/browser READMEs to distinguish
  verified own-repository Actions/Pages delivery from planned App provisioning.
  Replaced obsolete release/status claims and corrected the actual check-summary
  entry instructions; historical activation evidence is explicitly labeled.
- Planning was prepared locally before implementation approval; no publication or
  deployment occurred at that checkpoint. The approved implementation continues
  below on the required `phase-6` branch.


## 2026-11-06 IST — App onboarding implementation started, activation off

- Owner approved the plan and local implementation; explicitly retained an
  activation approval gate and required demo PR #24 to stay unmerged.
- Inspected public branch/PR state: `main` is PR #30 merge
  `c877af8ae2d25c2cf2773c70a0c6fa8499ba3d84`; remote `phase-6` at
  `7e09fc19477719b8d9fe2b9548bdc7e3514425fe` is an ancestor, with no unique
  work or open integration PR. Resumed `phase-6` from current main. Private
  working-tree changes are untouched.
- APP-0 research: documented current free-host constraints. Owner has no existing
  Linux host or Oracle account; no host is qualified, provisioned or activated.
  Static Pages, Render Free, Workers and Vercel Hobby are not drop-in hosts for
  this approved durable SQLite/native-worker profile. Oracle is only a candidate.
- APP-1 local implementation: browser-bound single-use state and PKCE, secure
  session cookies, fresh numeric GitHub identity/admin/installation validation,
  bounded optional HTTPS adapter, same-origin writes, logout/expiry fencing and
  unconfigured repository selection in migration 15. Existing workspace exposes
  only disabled capability/login routes; no scope/worker/check activation exists.
- New network dependencies are confined to the optional `github-app` feature.
  CLI and existing `server` builds do not include its TLS client. Locked license
  inventory validated for 108 packages and 200 original notice files.
- Full APP-1 acceptance remains pending browser setup UX and real OAuth on an
  approved host. APP-2 scope/consent, webhook jobs, App receipts/check publication
  and second-repository end-to-end acceptance are not complete.
- Test commands/results and deployment boundaries are recorded in
  [APP-1 evidence](evidence/P6.app1/acceptance.md). No App setting, release pin,
  workflow, live consent or hosted report changed. PR #24 verified OPEN at
  `b312bbf3c3f01299d04745b73ca3b81d24582a40`.

- Implementation commit: `39e658145c944202e3e96459f936f43b027f472b`.
  Draft integration [PR #31](https://github.com/SSC-SDE/CodeShouldBehave/pull/31)
  targets `main` from `phase-6`; no merge or deployment is authorized.
- Verification complete for this local slice: 292 all-feature Rust tests, 119
  no-default-feature Rust tests, 119 publication tests, strict Clippy in both
  configurations, 30 documentation links, and a 349-file committed distribution
  boundary/secret-pattern scan. No live OAuth or browser setup acceptance claimed.


## 2026-11-06 IST — browser picker and inactive scope confirmation

- Continued PR #31 on `phase-6` from `36100505837e2eafceb7779db849d7226ed068ca`.
  Provider check confirmed it is open and now non-draft; no state change or merge
  was performed. PR #24 remains open/unmerged.
- Added `/github-app/setup` with OAuth return/error handling, searchable installed
  repository picker, exact-commit path inventory and a step-by-step scope review.
  Factory host remains unavailable; no source scan or automatic check can start.
- Metadata uses repository, Git ref, Git commit and Git tree endpoints only.
  Blob/Contents/patch endpoints remain blocked. Reuses `PathScope` and the existing
  sensitive-path policy; symlinks/submodules are excluded. Unsupported files and
  absence of source/architecture analysis remain explicit.
- Migration 16 records expiring actor-bound previews and immutable inactive scope
  revisions. Fresh authorization, exact inventory fingerprint and transactional
  revision comparison reject stale commits, expired/replaced previews, access
  revocation, cross-user confirmation and conflicting administrator edits.
- Browser setup shows one active step; editing invalidates confirmation, reload
  restores the saved revision, and logout/denial clears displayed context.
  No credentials or setup state enter browser storage. Publication consent and
  architecture acceptance remain false in every saved intent.
- Tests and resource limits are recorded in
  [APP-2 local evidence](evidence/P6.app2/acceptance.md). This completes the requested
  browser picker/inactive path confirmation slice, not APP-2 source/architecture
  acceptance or live GitHub onboarding. No host, account, App setting, workflow,
  release, deployment or activation changed.

- Final local checks: 302 all-feature Rust tests, 119 core-only tests, 119 public
  pipeline tests and ten offline browser scenarios passed. Strict Clippy in both
  configurations, formatting and whitespace checks passed. The legacy private-plan
  checker is unavailable in the public snapshot; see APP-2 evidence for that limit.
- Implementation commit: `49a0b80cc18afec57341bd69945f4387052315ef`. Updated the existing
  [PR #31](https://github.com/SSC-SDE/CodeShouldBehave/pull/31); provider-confirmed
  state is open/non-draft, with no merge or activation performed. Committed source
  distribution scan: 355 files, zero secret-pattern findings; 27 local docs links
  verified. Dependency/license inventory is unchanged from APP-1.

## 2026-11-06 IST — App source preview and architecture review

- GitHub confirmed PR #31 merged at `c52b5527d8d480c5b9e62863fe3ff4e94cf4bab5`.
  Continued the existing clean `phase-6` branch by fast-forward to merged main.
- Starting explicit selected-source consent, isolated preview worker, architecture
  review and durable inactive acceptance. Hosting/live OAuth and activation remain
  gated; PR #24 is open/unmerged. No production settings changed.
- Added opt-in exact-source preview, existing detector reuse through a new bounded
  native worker protocol, and a digest-pinned Linux sandbox launcher. Browser users
  review/correct component boundaries and explicitly accept gaps/defer connections;
  accepted architecture remains inactive and non-exportable.
- Added additive migrations 17/18, optional inactive loopback host composition,
  authenticated durable webhook notifications and sticky removal/uninstall blocks.
  Notification claiming does not enqueue analysis. Existing Actions/Pages ownership,
  workflow settings, viewer, releases and PR #24 remain unchanged.
- Local acceptance: 316 all-feature Rust tests, 119 core-only tests, 119 publication
  regressions, four sandbox policy tests and 13 offline browser scenarios passed.
  Native executable protocol passed synthetic success/hash-mismatch checks;
  non-loopback host startup was refused. Strict Clippy in both configurations passed.
- Evidence: [source preview](evidence/P6.app2/source-preview.md) and
  [webhook inbox](evidence/P6.app3/inbox.md). Full APP-2/APP-3/APP-4 rollout remains
  incomplete: no qualified host/live OAuth, installation-token reconciler, full PR
  worker scheduler or App signed-publication/check writer. No activation claimed.
- Implementation commit: `7373248ea3120861431aa86539a61021124edcdf`. Opened draft continuation
  [PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32) from `phase-6` to
  `main`; full automatic App check delivery remains pending. Committed distribution
  boundary scan passed for 364 files with zero secret-pattern findings. No release
  package was uploaded and no deployment/activation occurred.


## 2026-11-06 IST — PR #32 execution and signed delivery primitives

- Resumed existing draft [PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32)
  on `phase-6` at `6349fb5479200e7b562d61d82fe41f4e8e3d1bb1`. PR #24 remains
  open/unmerged; activation, App settings and existing deployments are unchanged.
- Added repository-limited installation tokens with RS256 JWTs, fresh public
  same-repository PR/merge-base resolution, durable notification-to-job transactions,
  revision/consent/lease fencing, bounded claims and delayed retry limits.
- Added migration 19 with no enabled grants, distinct Ed25519 App signed intents,
  exact readback validation and completed-check outbox delivery. Unknown create
  outcomes reconcile rather than duplicating checks. No production consent writer,
  PR analyzer adapter, host drain loop or report publisher is mounted.
- Contract, local commands and remaining APP-3/APP-4 gates:
  [App execution development](APP_EXECUTION_DEVELOPMENT.md). Live host qualification,
  activation and hosted onboarding acceptance remain incomplete.
- Validation: 329 all-feature Rust tests, 119 core-only tests and 119 publication
  regression tests passed. Strict Clippy passed for all targets with all features
  and with default features disabled; formatting/whitespace and 18 local docs links
  passed. The real HLD/LLD fixture retained exact baseline/proposal source bytes
  through signed intent, readback and a captured check payload. No live GitHub
  installation token, check write, public upload or browser E2E run is claimed.
- Implementation commit: `cd0f8154fe1f01cb1973a849f65391ee6c48ba04`.
  Committed source-distribution scan: 367 files, zero secret-pattern findings.
  Dependency versions/license inventory are unchanged; `ring` is now an optional
  direct dependency using its existing locked version. PR #32 remains draft;
  this checkpoint does not claim full APP-3/APP-4 acceptance.

## 2026-11-06 IST — PR #32 PR worker, dispatch and report hosting

- Resumed draft [PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32) on
  `phase-6` from `9dfb47744645020d8152fe2565ffd9cbb07ad829`. Provider confirmed
  PR #24 remains open/unmerged. No main commit, merge, release or activation.
- Added bounded installation-token Git object acquisition with exact hash checks
  and authorized blob-only reads. New credential-free PR worker reuses the native
  engine for HLD/LLD and exact source export. Linux launcher is pinned/networkless;
  offline policy tests pass, actual Linux qualification remains pending.
- Connected inbox → queue → worker → signed intent → report publication/readback
  → App completed-check delivery. Host dispatch defaults paused; absent consent
  performs no GitHub requests. Migration 20 creates only an empty report catalog.
- Added separate cookie-free report listener using the approved workspace, atomic
  immutable file staging, signed catalog recovery, seven-day expiry, irreversible
  withdrawal and current-consent checks on each report read. Readback verifies all
  chunks, exact catalog identity and viewer assets before check delivery.
- Verified local notification-to-check composition with real native diffs and mock
  GitHub writes; failing readback cannot post a completed check, retry posts once,
  and consent revoked during analysis cannot stage a signed report. Actual loopback
  HTTP readback passes. Recovery cannot undo withdrawal; tampered source is refused.
- Validation: 338 all-feature Rust tests, 119 core-only Rust tests, 119 public Python
  regressions (Python 3.13, no skips), eight sandbox policy tests, both strict Clippy
  configurations and formatting passed. Existing browser UI was not changed; no
  live App/HTTPS/browser or Linux sandbox acceptance is claimed by these tests.
- Explicit limitations: 64 unique authorized blobs across the comparison,
  reconstructable exact commits only, no fork adapter, pending final authenticated
  publication consent, durable lifetime API/delivery budgets, fair scheduling,
  missed-event/rerequest repair, queued/failure checks, cleanup and backup restore.
  Details, resource caps and operator options: [execution checkpoint](APP_EXECUTION_DEVELOPMENT.md).
- No protected credentials, production origin, host account, App setting, workflow,
  existing Actions/Pages deployment, engine pin or paid resource changed. PR #32
  remains draft pending the documented implementation and live qualification gates.
- Implementation commit: `426547b896bdc4386c701ac3152c67f428a1ecae`.
  Committed distribution scan: 377 files, zero secret-pattern findings; 25 local
  documentation links resolve. Locked dependency versions and license inventory
  are unchanged. Continued the same draft PR #32; no additional PR or release.

## 2026-11-06 IST — PR #32 final consent and durable safeguards

- Continued existing draft `phase-6` PR #32 from
  `ecf79b97ff34677ea6577cd661a8df43c5b58edd`; provider confirmed PR #24 open/unmerged.
- Added migration 21 and authenticated publication preview/confirm/status/revoke.
  The browser displays exact scope, destination, engine and retention, with three
  explicit permissions. Decisions bind numeric actor, scope/context, operator pins,
  expected revision and expiry. Consent never enables a grant. Replacement/revocation
  disables previous grants, cancels unfinished work and withdraws reports atomically.
- Added atomic non-refundable REST reservations before dispatch IO, with actual
  broker call enforcement. Bounds: 384 tickets/job, 50/notification identity,
  2,000/installation/day and 10,000/App/day. Failed/crashed passes retain reservations.
- Added persistent tenant turns, analysis/delivery alternation, three delivery
  attempts and bounded delayed retry. Idle passes reconcile one tracked PR with a
  durable ten-minute checkpoint; only previously delivered App checks can map
  rerequests to a PR. Neither path resets exhausted attempts or duplicates checks.
- Added per-App signed receipt-chain audit at store opening/maintenance, expiry and
  revocation cleanup, and bounded old orphan collection. Withdrawal precedes deletion;
  symlinks/special files are refused. Signed intents and active analysis leases keep
  recoverable staging. Whole-database rollback still requires an independent trusted
  backup anchor and remains an explicit operational gate.
- Validation: 346 all-feature Rust tests and 119 core-only Rust tests passed; the
  final orphan-cleanup increment additionally passed all 77 App tests. Publication
  regression suite: 119 passed, no skips. Offline browser suite: 17 scenarios,
  including final consent, reload, revocation and narrow screens. No live GitHub,
  HTTPS host or Linux sandbox acceptance is claimed. Tests use isolated fixtures.
- Remaining gates: approved operator activation transaction, qualified USD 0 host /
  signed Linux worker, initial PR discovery, queued/failure check UX, exhausted-job
  recovery decision, trusted backup/restore and live fresh-repository acceptance.
  Updated [execution contract](APP_EXECUTION_DEVELOPMENT.md) and README accordingly.
- Activation remains OFF. No credentials, App settings, release pins, deployments,
  billing resources, existing Actions/Pages ownership or PR #24 merge changed.
- Implementation commit: `3f8824e9c40fcec41250b38ee8e658fd3dbb0fc2`.
  Committed source distribution: 380 files, zero secret-pattern findings. Strict
  Clippy passes for all targets in all-feature and core-only configurations;
  formatting/whitespace and 25 local documentation links pass. No dependency
  versions or license inventory changed. Continued the existing draft PR #32.

## 2026-11-07 — bounded App completion, local verification and distribution

- Continued the public `phase-6` checkout from inspected PR #32 head
  `6efe23e26118bcd7e5351b3946b7f7fdd49f9faf`, separately from the preserved
  private-repository working tree. Implementation commit:
  `14012713ee3a505dd1f1d45bb7b33a049126183c`; packaging commit:
  `6edfc70c1129b4f23b6c384fad3d91e571a2cd3b`; final progress-quota correction:
  `f041715cb9a2498b603fcac412d026dedd511cf9`. No remote update or merge.
- Migrations 22–25 add durable initial discovery, explicit signed activation audit,
  terminal recovery decisions/monotonic lifetime history and check progress.
  Discovery admits one page or pending PR per tick, including already-open PRs
  with no webhook. Leases, pending capacity, failed-page retention, rotating turns
  and hourly repeated walks survive restart. Known exact-authority jobs stay with
  tracked repair. Pagination is not a snapshot-completeness claim; forks remain
  unsupported by the worker.
- The same immutable App-owned Architecture check transitions through queued,
  running/publishing and failure. Only validated full readback can produce a
  completed result/Focus link. A lost queued POST is reconciled from its original
  durable payload; PATCH checks exact App/head/job ownership. Progress writes have
  bounded retries and consume existing nonrefundable lifetime/daily budgets.
  A lifetime-budget or credential refusal before provider admission also persists
  a ten-minute delay, so a blocked first check cannot starve other pending checks.
  The regression verifies selection and unchanged spent tickets across restart.
- Added `csb-app-operator` preview/approve/commit/offline-evidence workflow with
  three independent Ed25519 roots, exact consent/scope/context/pins, fresh public
  admin/installation access, ten-minute approvals and atomic revision fences.
  Runtime grants bind canonical admin consent, not a synthetic policy hash.
  Browser consent never activates; the setup screen distinguishes recorded
  operator authority from unknown runtime availability. Logout is not revocation.
- Added `csb-app-recovery` exact offline backup/verify/new-copy tooling and explicit
  exhausted/unknown-job reconciliation/abandonment APIs. They cannot POST a
  replacement check, reset attempts/reservations or restore withdrawn reports.
  Host import verifies before migrations/listeners, requires a latest-committed/
  quiesced authority acknowledgement, boots paused and disables imported grants.
  Independent external freshness remains a deployment qualification gate: an old
  periodic backup cannot prove no later revocations or spent budgets existed.
- Initial integration checks exposed digest-format incompatibility, an immutable
  test-fixture mutation, a mismatched consented test origin and an oversized API
  parameter list. Actual CLI smoke also exposed standard PKCS#8 v1 rejection;
  v1/v2 now derive and match the independently configured operator public root.
  These failures were corrected before the implementation commit; they are not
  represented as successful earlier runs.
- Verified on macOS arm64 with Rust 1.87 and Python 3.14:
  `cargo +1.87.0 test --locked --all-features` — **387 passed**;
  `cargo +1.87.0 test --locked --no-default-features` — **119 passed**;
  strict all-target Clippy in both configurations passed.
  `/opt/homebrew/bin/python3 -m unittest discover -s tests -p 'public*test.py'`
  — **119 passed, no skips**; the corresponding `app_*sandbox_test.py` suite
  — **8 passed**. Sandbox policy checks do not qualify a Linux host.
- The no-first-webhook dispatcher regression runs inventory → current comparison
  → real native HLD/LLD → signed delivery with fixture GitHub writes. The actual
  recovery executable passed backup/anchor/verify/new-copy and foreign-App/
  no-overwrite smoke. Actual operator commands verified ephemeral fixture
  signatures, signed a standard PKCS#8 v1 approval, rejected tampering and left
  database/artifact paths absent. No production credentials or network authority
  were supplied.
- Actual Chrome rendered the operator-authorized setup state with synthetic App
  API responses, then exercised revocation and logout. Desktop and 390px pages
  displayed unknown runtime availability, no horizontal overflow, no page errors
  and no browser credential storage. Local visual evidence:
  `target/operator-status-desktop.png`, `target/operator-status-mobile.png`.
  This is local browser evidence, not live OAuth/App acceptance.
- `scripts/build_public_release.py --output target/app-continuation-runtime-f041715
  --revision f041715cb9a2498b603fcac412d026dedd511cf9 --build-runtime`
  built exact committed source in an isolated offline tree. Local
  `aarch64-apple-darwin` package contains all eight binaries, including the App
  host, preview/PR workers and operator/recovery tools. Clean extraction passed
  recovery self-test, operator help and invalid-worker-input rejection.
  Manifest: 449 files including notices, 108 locked registry-license packages,
  zero findings under the declared three-rule pattern scan (not exhaustive secret
  detection). Runtime archive SHA-256:
  `0260ba4595823f87fb4a38306ea75f37cb998121199fdc2db4f80c8cebebbe4c`.
  Artifacts remain local under `target/app-continuation-runtime-f041715`; no upload,
  hosted attestation, signed Linux release or engine-pin update is claimed.
- Remaining external gates: qualified USD 0 Linux/HTTPS/key-storage service and
  independent qualification roots; signed workers on that host; latest-committed
  recovery authority/fencing; PR CI/review/merge; explicit real operator activation;
  and live fresh-repository check → Focus → HLD → LLD → source plus revocation/
  cost/restart/recovery acceptance. Existing Actions/Pages deployment, PR #24,
  billing and App settings remain unchanged. Beta is not launched.
## 2026-11-08 — approved stale HLD report retirement for redesigned canary

- The public canary publisher reached the unchanged 64 MiB site bound with the
  redesigned signed engine: the live site measured 62,319,338 bytes before
  admission, and the next snapshot added three neutral bookkeeping reports plus
  the redesigned workspace assets.
- The owner approved retiring the superseded draft PR32 report
  `41abc15e14eb1edcd07489e7cadb826057e5865273622fd69d7b159bc198d301`.
  The exact ID was added to the trusted `.codeshouldbehave/report-retirements.json`
  ledger. No capacity limit was changed and no Pages files were deleted manually.
- The next signed canary must prove the immutable withdrawal ledger, source-free
  expiry record, complete site readback, redesigned workspace assets and report
  preservation for every non-retired report.

## 2026-11-08 — approved historical HLD report retirement

- The first approved stale-report retirement still left the exact redesigned
  snapshot above the unchanged 64 MiB site bound because the active canary pass
  admitted one new report bundle.
- The owner approved retiring the second superseded PR32 report
  `a1f293a0cfc5bc0ab2e4dc3ef5d0186f6e419edc9291165af7ad5f65fb6cf7e3`.
  This is the historical PR32 HLD bundle; the exact ID was added to the trusted
  retirement ledger. Capacity limits and report admission rules remain unchanged.

## 2026-11-08 — redesigned public canary deployed

- PR #45 merged at `5c6e48ec387c49f0a09b1b5861a98da76c53b069`; PR #46 merged at
  `3c5882ad973187078278ce577d44dce267846fc0`.
- Controller run
  [42494668980](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42494668980)
  passed plan, native analysis, signed publication, Pages deployment, live
  readback, receipt commit and Architecture-check delivery.
- Live workspace:
  https://ssc-sde.github.io/CodeShouldBehave/workspace/
  serves the redesigned workspace CSS byte-for-byte:
  `0fe1c422676f56f459367e77c5912b155e9425cea365b79109a8ba3aaee8391f`.
  The verified site manifest is
  `3aeb13fa55e3efb9d6a87ba144590b717477876d32f206e818e6a449c94fe795`,
  with 1,551 files and 47,865,534 bytes.
- The two approved stale PR32 report URLs return 404 and the live catalog retains
  six non-retired reports. No site-cap or admission limit changed; beta remains
  disabled.
## 2026-11-08 — approved phase 7 nested HLD implementation start

- The owner approved the Mockup 2 C4-style HLD drilldown implementation plan.
- Created branch `phase-7` from `origin/main` at `ddc0111`.
- Scope: evidence-backed system → container → component hierarchy, relationship canvas, evidence inspector, and HLD-to-LLD drilldown. No runtime architecture claims will be inferred.
- Implementation and public canary evidence are pending; no PR, release, or merge is claimed.

## 2026-11-14 — plug-and-play AI extension plan (documentation only)

- Recorded the owner-aligned optional-extension contract in the
  [detailed AI implementation plan](AI_EXTENSION_IMPLEMENTATION_PLAN.md).
  First proposed delivery is Explain this HLD boundary through a server-side
  local Ollama adapter; existing Focus structure, HLD/LLD and source navigation
  remain independently usable.
- The plan defines provider-neutral contracts, capability/API states, immutable
  evidence and cache identities, disclosure/endpoint restrictions, separate
  admission limits, ordered Phase 7 substeps and acceptance/rollback evidence.
  Hosted/static delivery limitations and later provider qualification are explicit.
- AI implementation and acceptance remain pending. This record does not complete
  existing Phase 7 steps or change the separately recorded nested-HLD scope.
  No model download/call, source disclosure, provider credential, branch/PR
  creation, merge, deployment, billing or activation is authorized or performed
  by this documentation update.

## Approved Focus Review desk — implemented locally, native verification blocked

- Owner approved the Review desk layout and full-page Moria colored-text
  backdrop, then authorized implementation in the maintained application.
- Resuming the existing local `phase-7` branch at
  `ddc01110ff8a0088e027b071ede4dd1c19f7b095`; remote `main` was observed at
  that same commit. No matching Phase 7 PR or remote branch was returned.
- Scope: real authorized HLD boundary navigation, bounded contextual Mermaid
  maps, exact-source inspector, existing HLD-to-LLD navigation, and an optional
  personal Focus backdrop. Standard and LLD presentation remain unchanged.
- Existing unrelated design/backend edits are preserved. AI implementation,
  release, deployment, phase completion, PR publication and merge are not
  part of this UI change. No new commit or release identity is claimed.
- Implemented Focus-only Review desk navigation, five-boundary overview pages,
  isolated boundary graphs, native snapshot comparison, keyboard-selectable
  nodes and exact-source inspector. Standard/LLD controls return to their
  original containers when leaving Focus HLD.
- Added the approved 288×120 colored-text Moria backdrop as same-origin assets.
  Existing local/account preference validation migrates older documents without
  resetting personal settings; backdrop, legend and zoom controls respect saved
  preferences. Native/App/static/release asset allowlists include the new files.
- Verification: JavaScript syntax checks passed for the main workspace and new
  scripts; 146 tests passed across workspace, Focus model, HLD, LLD, public
  transport and preference suites. Python 3.14 packaging checks passed:
  public site 13, public release 6 and development asset host 2 tests.
- Browser proof used the maintained source assets at
  `http://127.0.0.1:3222/workspace/` with an existing authorized static fixture,
  not a rebuilt native service. At 1440px and 390px: scoped graphs, comparison,
  search, next boundary, zoom, exact proposal/baseline source, evidence reopening,
  LLD drill-down, Standard restoration and backdrop save/reload were exercised.
  The 390px viewport had no document horizontal overflow. Final browser error
  collection was empty. The real integrated desktop surface was shown before
  automated test suites; final screenshots followed interaction verification.
- Native gate attempted:
  `cargo test --locked --features github-app --lib workspace_`.
  Compilation failed in existing unrelated backend edits: references to absent
  `DesignVersion.sections` and a moved `root` in `src/design/generate.rs`.
  The chained `github_app_setup::report_host::tests` command therefore did not
  run. Native asset/preference tests remain unverified until that backend work
  compiles; no unrelated backend edits were overwritten to force a green result.
- Updated `CHANGELOG.md` and `docs/PUBLIC_MVP_USAGE.md` for the unreleased UI.

### Focus merge preparation

- The owner subsequently authorized merging the approved UI implementation.
  `origin/main` remained `ddc01110ff8a0088e027b071ede4dd1c19f7b095`.
  GitHub reported no branch protection or applicable branch rules for `main`.
- The scoped index includes Focus UI, assets, preference migrations, associated
  tests/docs and the previously requested AI plan (documentation only).
  Unrelated in-progress design/backend changes remain unstaged and excluded.
- A clean archive of the staged tree passed
  `cargo test --locked --features github-app --lib workspace_` (11 tests) and
  `cargo test --locked --features github-app --lib github_app_setup::report_host::tests`
  (3 tests). This resolves the native verification gap for the merge scope;
  it does not repair or discard the unrelated working-tree backend changes.
- Rust formatting checks passed with edition 2024 for the four touched Rust
  files. JavaScript syntax and Python compilation checks passed. The earlier
  146 JavaScript tests, 21 packaging tests and browser evidence remain applicable.
- Review, PR creation and provider-confirmed merge results will be recorded
  separately; this entry does not claim a merge, deployment or Phase 7 completion.

### Focus integration PR ready

- Created [PR #49](https://github.com/SSC-SDE/CodeShouldBehave/pull/49),
  `phase-7` → `main`, for implementation commit
  `a3fca3c099af3bddbd3b51271b70a813921c4172`.
- GitHub reported `MERGEABLE` / `CLEAN`, no required review decision, and the
  `CodeShouldBehave nudge / wake` check passed:
  [run 43580193251](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/43580193251).
  This wake check is not a test suite; local test evidence is recorded above.
- Scoped implementing-assistant security/correctness review found no blocking
  findings and was posted on the PR. Separate local reviewer jobs failed because
  their model service was unavailable; no independent approval is claimed.
- User merge authorization applies to this scoped PR. No deployment, release-pin
  change or completion of the remaining Phase 7 implementation is authorized.

### Focus integration merge confirmed

- GitHub confirmed [PR #49](https://github.com/SSC-SDE/CodeShouldBehave/pull/49)
  merged into `main` at `2026-11-14T09:23:12Z`.
  Provider-reported squash merge:
  `4933728d42b557fd266b539d159344749262fad5`.
- Final reviewed PR head was `1e5a4aa95d94ba5897dcdf08a73fa2f4eced1771`.
  Its `CodeShouldBehave nudge / wake` check passed in
  [run 43580282371](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/43580282371).
  Merge used an exact-head guard and the owner's explicit merge authorization.
- This documentation-only bookkeeping branch starts from the confirmed merge;
  it does not write directly to `main`. No new runtime change, deployment,
  release-pin update or Phase 7 completion is claimed.
- The original checkout remains on `phase-7` with the five unrelated backend
  working-tree changes preserved and excluded from the merged PR.
