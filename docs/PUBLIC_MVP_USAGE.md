# CodeShouldBehave

CodeShouldBehave shows how a pull request changes Rust structure and local
behavior. Open the PR's **CodeShouldBehave / Architecture** check, then follow
HLD → component → LLD declaration/function → exact baseline/proposal source.
The same dark workspace provides combined, before/after and split/unified views.

**Open source · MIT · experimental.** Created by Shankhya Subhra Chatterjee,
with contributor attribution retained in [LICENSE](LICENSE). Development,
issues and contributions belong in the public
[CodeShouldBehave repository](https://github.com/SSC-SDE/CodeShouldBehave).

The [Linux x86_64 experimental release](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-2917f1bb352b)
is built and signed; checksums and release bootstrap verification passed.
Native sandbox startup for this release and the live PR-to-dashboard journey
remain pending. Automatic PR analysis and a hosted dashboard are **not enabled
by cloning this repository**. See [release status](docs/RELEASE_STATUS.md).

After one-time maintainer setup and activation, opening, reopening or updating a
PR is intended to trigger analysis automatically. Reviewers follow the check's
Details link without terminal commands, API keys or a local server.

## Reviewer journey

1. After repository activation, open the exact Architecture check on the PR
   revision you want to review. Details opens **Focus** with the pinned report;
   direct dashboard entry opens **Standard**.
2. Verify repository, baseline and proposal commits. The link pins both analysis
   and public export identities; a newer PR head does not change that report.
3. Open **Architecture diff**, inspect HLD direction/component changes and choose
   a component to drill into the published LLD catalog and local flow.
4. Select a node or evidence item for exact code. Use split/unified and the
   before/after controls to inspect the change. Hover/focus exposes source;
   pin it when needed and press Escape to close the preview.
5. Select files in the left panel for status and added/deleted line counts.
   Use the file tick or **Mark all files reviewed** for personal inspection state.
6. Review gaps before drawing a conclusion. Missing/deferred detail cannot be
   requested from a hidden native server; it needs another authorized analysis.

## Workspace appearance

**Profile / Settings** offers independent Standard and Focus layouts, selectable
sections, panel ordering/sizing, presentation filters and application themes:
Graphite, Blueprint, Terminal and Contrast. **HLD appearance** offers Studio
(the Focus default) and Classic (the Standard default). Studio changes HLD
presentation only; it does not change LLD or analysis evidence.

Static reports save layout preferences in the browser only. The optional Rust
account service supports authenticated personal preferences with revision
conflicts; it is not supplied by static Pages hosting. Temporary PR adjustments
remain separate from saved defaults. No credentials belong in these preferences.

Personal reviewed files, questions and bookmarks live in page memory. Reload
clears them. They are not shared reviewer state, GitHub approval or a privacy
boundary between projects on the same Pages origin. Feedback exports locally;
nothing is automatically sent to another person or analytics provider.

## Maintainer installation contract

This profile is for **one opted-in public repository**. Setup requires a
base-owned public path policy, confirmed component context, immutable analyzer
package/workflow pins, Pages configuration and hard-zero Actions/Packages budget
controls. A PR cannot widen its own policy. Related/private repositories and
external services are outside the initial public context.

Installation templates live under `templates/public-ci/`; they are not active
workflows merely because the source exists. The installer must reject unresolved
pins, missing context, invalid activation proof or disabled live-write consent.
The final installation walkthrough records actual release digests, repo ID and
Pages URL rather than asking a reviewer to enter host/PR values repeatedly.

GitHub Actions produces this profile's check. It does not impersonate the
registered CodeShouldBehave GitHub App. A complete check is advisory and does
not approve merging. Unsupported, limited or deferred analysis is explicit.

## Build and verify locally

Development commands require the source archive or repository checkout. The
runtime archive contains executables, browser/workflow assets and notices,
rather than the complete development tree.

Install Rust 1.87.0, Node.js and Python 3.11+ first. From the source repository:

```sh
git clone https://github.com/SSC-SDE/CodeShouldBehave.git
cd CodeShouldBehave
rustup toolchain install 1.87.0 --profile minimal --component clippy --component rustfmt
```

Then build and test:

```sh
cargo +1.87.0 build --locked --all-features --bins
cargo +1.87.0 test --locked --all-features
node --test tests/*.test.cjs
python3 -m unittest discover -s tests -p 'public_*test.py' -v
```

The exporter consumes a **private native artifact** and an explicit public policy;
the trusted controller independently verifies provider visibility and policy
origin. Never upload that private artifact directly.

```sh
target/debug/csb-public-report --help
target/debug/csb-public-report verify --bundle /absolute/path/to/public-bundle
```

Build a new local site from a verified bundle, supplying the real UTC observation
time. The output directory must not already exist:

```sh
python3 scripts/public_site.py build \
  --bundle /absolute/path/to/public-bundle \
  --output /absolute/path/to/new-site \
  --verifier /absolute/path/to/verified/csb-public-report \
  --verifier-sha256 VERIFIED_BINARY_SHA256 \
  --observed-at ACTUAL_UTC_TIMESTAMP
python3 scripts/public_site.py verify --site /absolute/path/to/new-site
python3 -m http.server 3220 --bind 127.0.0.1 --directory /absolute/path/to/new-site
```

Open `http://127.0.0.1:3220/workspace/`. This last command is only a local preview;
the accepted hosted report does not need a reviewer to keep a terminal open.
The site builder prints its manifest hash/bytes. A restore additionally requires
the independently trusted previous-site hash and current withdrawal ledger.

## Scope, cost and recovery

Static inventories are capped at 20,000 files and a 4 MiB manifest so every
published generation remains within the restore reader’s bounds. New output
that exceeds a cap is refused before publication.

Source is bounded to approved excerpts; the initial LLD detail budget is 100
entries. Unpublished declared counterpart pairs and native design editing are
unavailable in the static profile. Exported Markdown contains public evidence.
Unsupported languages/constructs remain explicit; Rust is the first semantic
profile, and static analysis does not prove runtime behavior.

The public workflow uses standard runners and Pages, with cache off and finite
site/artifact reservations. Owner hard-zero budgets must remain in force. Global
account capacity is not observable through a repository token: quota failure can
stop publication. The application does not buy credits, upgrade a plan or use a
paid fallback. The index is last observed/published, not a live-current promise.

Disable installed workflows to stop new analysis/publication. Retain the verified
last permitted site/receipt; apply withdrawals before any rollback. Removed
reports get a source-free expiry record. Previously downloaded public content
cannot be recalled. Follow SECURITY.md for private reporting and release notices
for exact compatibility/upgrade instructions.

## Release preparation

`scripts/build_public_release.py` requires Python 3.11+, an exact committed source
SHA and the locked Cargo registry sources. It prepares source/runtime archives,
checksums, a lockfile dependency SBOM and original third-party notices locally.
It excludes private history, evidence, state and screenshots and never uploads
anything. A pattern scan is not an exhaustive privacy audit; public source and
canary activation require a separately reviewed manifest and final approval.

## Contributing and licensing

Use branches and pull requests against this public repository. Read
[CONTRIBUTING.md](CONTRIBUTING.md) and report vulnerabilities privately using
[SECURITY.md](SECURITY.md). Earlier private development history is not part of
this public repository and is not needed to contribute.

Project code is MIT licensed; retain copyright and license notices when
redistributing. Dependencies and bundled fonts retain their own licenses; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [font notices](src/fonts/README.md).

### Preserve the activated budget identity during release refreshes

The signed controller state retains the budget policy and observation digest from
initial activation. Keep that exact runtime budget configuration and evidence
when changing engine pins. Its observation expiry gates **initial activation**;
previously verified activation can continue only with the identical budget policy
and the operator maintaining the USD 0 hard stops. It does not claim a live billing
API reading. A renewed release-build observation is separate and must not replace
this runtime identity. Record subsequent real control checks in the release log.

If a pin update inadvertently replaced the runtime observation, restore the exact
previously activated evidence and budget block through a reviewed PR. Preserve
engine pins and signed state. Never delete receipts, manufacture activation, or
relax policy comparison to get a run through. A deliberate budget-policy change
requires a separately designed and reviewed reactivation path.

### Explicit recovery of a signed Pages intent

Recovery does not delete receipts or reset state. A maintainer-reviewed
`.codeshouldbehave/pages-recovery.json` must name the exact repository, SHA-256
of the current signed state, SHA-256 of its pending intent, action
`resume_exact_intent`, schema `csb-pages-recovery-v1`, and an expiry within 24 hours.
The file authorizes only that immutable artifact/version. It is read from trusted
main, never from the PR being reviewed. Refresh expiry only after reconfirming the
same intended state and operator authorization; changed state needs fresh review.

Deploy the signed engine containing this support, then manually dispatch the
unchanged public controller on main. Scheduled/nudge runs cannot recover. Planning
creates zero analysis requests; publication rechecks the exact approval and state.
Recovery checks the originating trusted run and unchanged scope/destination. If
Pages already succeeded it verifies the served site; otherwise it may recreate the
same unexpired artifact/version once. A missing status is polled within the existing
30-poll bound without repeating the creation request. Missing/expired artifacts,
scope drift, readback digest mismatch or concurrent state changes leave the intent
intact. No paid fallback or state deletion is used.

After successful readback, the normal workflow attests and appends a chained
`site_promotion` receipt. The authorization cannot replay against the new state.
An ordinary subsequent controller pass analyzes open PRs and publishes their
exact-head checks; recovery alone does not declare reviewer acceptance.

Diagnostics expose only operation categories (`pages_create`, `pages_status`,
`artifact_metadata`, `provider_read`, `provider_write`) and HTTP status codes.
Tokens and provider bodies are never added to error messages. A creation 404 is
not treated as a successful deployment or automatically retried.

### Pages commit-version compatibility

New Pages intents use the exact controller commit SHA. GitHub's deployment
service rejects synthetic version strings despite the REST field's general
wording. Legacy synthetic intents require the explicit
`resume_exact_artifact_at_origin_commit` authorization action with a
`pages_build_version` equal to the verified original controller SHA. Existing
state/intent hashes and expiry still apply. No artifact, site digest, scope or
historical signed receipt changes. Successful served-site readback remains
mandatory; a green workflow alone is not dashboard acceptance.
