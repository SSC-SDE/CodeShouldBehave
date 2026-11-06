# CodeShouldBehave

CodeShouldBehave shows how a pull request changes Rust structure and local
behavior. Open the PR's **CodeShouldBehave / Architecture** check, then follow
HLD → component → LLD declaration/function → exact baseline/proposal source.
The same dark workspace provides combined, before/after and split/unified views.

**Open source · MIT · experimental.** Created by Shankhya Subhra Chatterjee,
with contributor attribution retained in [LICENSE](LICENSE). Development,
issues and contributions belong in the public
[CodeShouldBehave repository](https://github.com/SSC-SDE/CodeShouldBehave).

## Current status

The repository's **Actions → Architecture check → hosted Focus dashboard** demo
is working. [PR #24's check](https://github.com/SSC-SDE/CodeShouldBehave/runs/127446113035)
was followed in-browser through HLD, LLD and exact baseline/proposal source diffs.
[Controller 42083656278](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42083656278)
passed publication and check delivery; its application REST audit used 71/100 calls.
This is own-repository acceptance, not a general hosted-service or capacity claim.

The deployed [signed Linux release](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-c46c28a509b6)
passed signature, checksum, production bootstrap and native sandbox verification.
See [release evidence](docs/RELEASE_STATUS.md).

| Installation | Status | Maintainer setup | Reviewer entry |
| --- | --- | --- | --- |
| Repository Actions + Pages | Working on this repository; available for reviewed public-repo setup | Confirm paths/components, prepare pinned config/workflows, configure Pages and budget controls, approve activation | Architecture check → Open CodeShouldBehave report → Focus |
| GitHub App install-and-confirm | In development; not available for installation yet | Target: install App, choose repo, preview/confirm scope in browser | Automatic App check → Focus |

Cloning the project or installing the registered App alone does not currently
activate checks in another repository. The [App onboarding plan](docs/GITHUB_APP_ONBOARDING_PLAN.md)
defines the simpler target and its hosted-service, scope, security and cost gates.
The existing public CI profile needs no reviewer terminal, API key or local server.

The App implementation includes OAuth/PKCE, a browser repository picker and
versioned path-scope confirmation, opt-in isolated source preview and inactive
architecture acceptance. The PR worker, host dispatcher, signed report server and
check delivery are connected and tested locally. Bounded PR discovery, queued/
running/failure check states, signed operator approval and exact offline recovery
tools are implemented on the local `phase-6` continuation; they have not been
pushed to PR #32. Dispatch defaults paused, and browser consent never activates it.
Qualified hosting, signed Linux workers, externally fresh recovery authority and
live App acceptance remain required. Installing the App does not activate this
development integration.
See the [execution checkpoint](docs/APP_EXECUTION_DEVELOPMENT.md) and
[$0 hosting assessment](docs/APP_HOSTING_OPTIONS.md) for current limits.

## Public landing page

New site builds open on a responsive product introduction, with system light/dark
themes and real captures of a published PR review. **Open workspace** preserves
the existing reviewer entry; exact report links are unchanged. Actions/Pages
setup and the unavailable App route are clearly separated. This landing change
is local development, not a deployment. See [preview and image provenance](docs/PUBLIC_MVP_USAGE.md#public-landing-page).

## Reviewer journey

1. After repository activation, open **CodeShouldBehave / Architecture** on the
   PR revision, then **Open CodeShouldBehave report** in its summary. GitHub may
   route Details to its own check page. The report link opens **Focus** with the
   pinned comparison; direct dashboard entry opens **Standard**.
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

## Install on another public repository today

This profile is for **one opted-in public repository**. Setup requires a
base-owned public path policy, confirmed component context, immutable analyzer
package/workflow pins, Pages configuration and hard-zero Actions/Packages budget
controls. A PR cannot widen its own policy. Related/private repositories and
external services are outside the initial public context.

Installation templates live under `templates/public-ci/`; they are not active
workflows merely because the source exists. The installer must reject unresolved
pins, missing context, invalid activation proof or disabled live-write consent.
Use the [maintainer setup guide](templates/public-ci/README.md)
for preparation and activation. Generate the new repository's own confirmed
context and policy; do not copy this repository's IDs, consent, budget evidence
or receipt history. Review the installation diff before enabling it. If that
repository already has a Pages site, resolve the hosting conflict first.

The planned App route moves this work into the service. It needs a real backend
and isolated workers; free static hosting alone cannot provide it. No hosted
App service or new spending is activated by this implementation.

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
