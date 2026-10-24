# Installation and consent review — NOT applied

PR #6 merged at `bd5c06f6566c9523e48dddcefc10f85caf4c6236`.
This directory is a review artifact only. Merging it does not install workflows,
grant consent, activate defaults or deploy a site.

## Exact proposed file placement

`installation.patch` shows the six base-owned files at their intended runtime
paths. `file-manifest.json` pins each staged source. The policy, context and config
in this patch remain disabled. Do not apply or merge this patch as an activation
step: workflow placement alone registers scheduled/event runs even when the
runtime refuses their disabled configuration. The installer has NOT been run.

`consent-preview.diff` shows the proposed publication consent and three operation
switches. It is illustrative and deliberately incomplete, not an executable
activation patch: changing policy bytes invalidates their existing digest, and
budget proof is still unavailable. No evidence or approval is manufactured.

## What approval would authorize

Only public repository SSC-SDE/CodeShouldBehave, numeric ID 1423092555:

- Read PR merge-base/head Git objects within Cargo.toml and src, excluding src/fonts.
- Export authorized code evidence, file paths, symbols, architecture/behavior
  graphs and source excerpts into publicly accessible static review reports.
  Public reports do not require login. No additional repositories or integrations.
- Publish reports at https://ssc-sde.github.io/CodeShouldBehave/ and advisory
  Architecture checks on exact PR heads. Check links open Focus with the exact
  comparison; ordinary dashboard entry opens Standard.
- Run on PR opened/reopened/synchronize/ready-for-review/converted-to-draft/closed
  events, with a ten-minute reconciliation schedule and manual dispatch fallback.
- Use existing standard GitHub-hosted runners, fixed Linux canary
  canary-39b89ac088be, and the existing USD 0 account hard stops. No paid fallback.

Existing gaps remain explicit: eight unsupported paths, unknown hosting,
macro/conditional/runtime uncertainty and deferred integrations. They must not
be reported as a supported clean no-change result. Checks are advisory, not merge
approval. End-to-end provider, recovery and fork acceptance are still unproven.

## Permissions of the reviewed template

Plan: contents/actions/deployments/attestations read. Analyzer: contents read.
Publisher: contents read; actions/deployments/attestations/id-token/pages/checks
write, scoped to its job, with github-pages environment. Actions write supports
bounded artifacts/cleanup; deployments and attestations support durable verified
state; OIDC signs it; Pages and checks publish results. Nudge has no permissions
and executes no PR source. This flow uses the workflow token; it does not require
expanding the registered GitHub App's permissions or adding a private key.

## Required steps after explicit approval, before activation

1. Reverify main and scope drift. Regenerate the context through native onboarding
   with explicitly authorized export permission. Do not hand-edit export_allowed
   or copy the old confirmation/fingerprint onto a changed scope. Preserve the
   approved boundary and deferred integrations; review the new fingerprint.
2. Reobserve current GitHub Free / USD 0 Actions and Packages controls. The earlier
   release secret is not a runtime budget proof. Prepare the schema-minimal
   operator observation at a reviewed base-owned path and hash its exact bytes.
   Its account, Free plan, timestamps and hard-stop status would be public if
   committed for this template. Do not include screenshots or other billing data.
3. Update policy/context/budget hashes, operation switches and the exact template
   hash. Use the already verified Linux runtime's create-only installer in a
   disposable checkout. Verify its output and present the complete final diff.
4. Separately approve and merge that final activation diff. Merging active
   workflows to main is consequential: scheduled reconciliation can start before
   a new PR is opened. Do not merge installation separately from valid consent.
5. Verify Pages environment protections and default-branch deployment access.
   Read-only API check on 2026-10-24 found build_type=workflow, the expected project
   URL and status=null; this is configuration, not proof of a live dashboard.
6. Run a bounded canary and verify exact-head checks, Focus landing, HLD/LLD/source
   navigation, coverage, fork isolation and recovery before claiming MVP readiness.

## Stop / withdrawal

Before activation, stop by leaving these artifacts unapplied. After activation,
disable controller and nudge workflows to stop new work. Disabling alone does not
remove already public Pages bytes; withdrawal requires the verified publication
withdrawal/recovery procedure or separately authorized Pages removal. Do not
promise that disabling a check retracts a report.

## Current state

No runtime files were installed. Export permission, publication consent, review
defaults and analysis/publication/check operations remain false. No Pages or
workflow settings changed. Only the two existing manual release/preflight
workflows are installed. Approval of this review document alone is not activation.
