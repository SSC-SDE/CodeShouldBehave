# APP-2 browser picker and inactive scope intent

Date: November 6, 2026 (IST). Branch `phase-6`, PR
[#31](https://github.com/SSC-SDE/CodeShouldBehave/pull/31).
Starting commit: `36100505837e2eafceb7779db849d7226ed068ca`.
Status: **in_progress** for full APP-2. This local slice is implemented; live
provider/host acceptance and source/architecture confirmation remain pending.

## Implemented and tested

- Searchable repository picker after the authenticated same-origin OAuth return.
- Bounded metadata-only path inventory pinned to default-branch commit and tree.
  No source/blob/patch request. Truncated or malformed metadata fails closed.
- Literal `PathScope` inclusions/exclusions plus existing sensitive-path policy;
  symlinks/submodules excluded. Exact preview includes files, Rust candidates,
  unsupported-file count and explicit absence of source/architecture analysis.
- Expiring actor/repository-bound previews; confirmation checks current provider
  access, inventory fingerprint and revision in a transactional save. Competing
  admins cannot overwrite; stale/replayed previews require a fresh review.
- Migration 16 preserves selected repos, preferences and review schema. Immutable
  scope history records numeric actor, commit/tree, paths and time, always off.
  Existing repository setup state remains `installed_unconfigured`; no analysis,
  architecture acceptance, publication consent or default activation is created.
- Factory host exposes the setup page but an unavailable capability. No sign-in
  request or account-service enumeration occurs from that disabled page. Static
  reports, workspace/HLD/LLD assets and public deployment settings are unchanged.

## Evidence

Rust router/storage/provider tests cover success, repeat/reload, stale commit,
expired and replaced preview, wrong repository/user, withdrawn permissions,
concurrent admins, empty/invalid/secret-only scope, input tampering, CSRF,
truncated tree and refusal of source-bearing GitHub endpoints.

Offline Chrome UI tests: **10 scenarios passed**, including unavailable host,
selection, safe filename rendering, editing invalidation, inactive save, reload,
conflict recovery, provider failure, narrow screens/focus, revocation and logout.
All requests use explicit local fixtures; zero unexpected external requests and
no page exceptions. Desktop/mobile screenshots were visually inspected. This is
not a live OAuth, GitHub permission or deployment test.

Reproduce: `node scripts/app_setup_browser.cjs` with Playwright/Chrome paths as
specified in [development notes](../../APP_ONBOARDING_DEVELOPMENT.md). Results
and screenshots are local `target/app-setup-browser/` artifacts, not public data.

Final regression results (Rust 1.87.0, locked dependencies):

- `cargo +1.87.0 test --locked --all-features`: 302 passed.
- `cargo +1.87.0 test --locked --no-default-features`: 119 passed.
- Strict all-target Clippy with all features and no default features: passed.
- `cargo +1.87.0 fmt --all -- --check` and `git diff --check`: passed.
- `python3.13 -m unittest discover -s tests -p 'public*test.py'`: 119 passed.
- Offline browser contract: 10 scenarios passed, 21 mocked API requests,
  zero external requests. No live provider acceptance is implied.

The legacy `check_implementation_plan.py` cannot run in this public snapshot:
its private historical `docs/IMPLEMENTATION_PLAN.md` input is not distributed.
It was not reported as passing or bypassed; current public onboarding plan and
changed documentation links were checked separately.

## Limits and remaining gates

Inventory: 2,000 entries; 256-byte paths; 32 roots per include/exclude list; HTTP
body 16 KiB; stored scope document 512 KiB. Preview lifetime 15 minutes, 256
previews globally. Confirmed history: 100 revisions/repo, 1,024 globally. No
silent deletion, partial success or paid fallback when a limit is reached.

Selected-path source scanning, component/gap review, architecture/publication
consent, verified webhooks, workers, App receipts/checks and real host acceptance
remain pending. Host unqualified; activation off; PR #24 unmerged.

## Committed checkpoint

Implementation: `49a0b80cc18afec57341bd69945f4387052315ef` on `phase-6`, existing PR #31.
Committed distribution boundary: **355 files**, zero secret-pattern findings.
Changed documentation: **27 local links** verified. No package was uploaded or
release built by this check. The dependency set is unchanged from APP-1.
GitHub confirmed PR #24 remains open, unmerged, at
`b312bbf3c3f01299d04745b73ca3b81d24582a40`.
