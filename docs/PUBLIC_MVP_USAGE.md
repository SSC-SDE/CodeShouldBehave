# CodeShouldBehave public review workspace

CodeShouldBehave shows how a pull request changes Rust structure and local
behavior. Open the PR's **CodeShouldBehave / Architecture** check, then follow
HLD → component → LLD declaration/function → exact baseline/proposal source.
The same dark workspace provides combined, before/after and split/unified views.

Status: local implementation and verification are underway. The public CI
installation and hosted URL are not yet claimed live. No reviewer terminal,
API key or local server is required once an accepted public deployment exists.

## Reviewer journey

1. Open the exact Architecture check on the PR revision you want to review.
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

From the source repository root:

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
