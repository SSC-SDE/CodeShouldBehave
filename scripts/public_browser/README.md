# CodeShouldBehave local public-viewer verification

These optional helpers verify the documented `fixture/public` synthetic report in the existing dark workspace. They do not install dependencies, contact GitHub, authorize a deployment, or represent production traffic. The application itself does not require Playwright.

Prepare a public synthetic bundle with the Rust integration test, then assemble and verify the local static site with `scripts/public_site.py`. Keep the generated `private-artifact.json` outside the bundle and site. Serve the assembled directory on an isolated loopback port; do not use the user's running application.

Set these environment variables before running the helpers:

```sh
export REPO_ROOT=/path/to/CodeShouldBehave
export PUBLIC_SITE="$REPO_ROOT/target/phase6-public-site"
export PUBLIC_BASE_URL=http://127.0.0.1:3220/workspace/
export PUBLIC_BROWSER_OUTPUT="$REPO_ROOT/target/public-browser-qa"
export PUBLIC_SITE_SHA256=<verified-site-manifest-sha256>
export PLAYWRIGHT_MODULE=/path/to/node_modules/playwright
export CHROMIUM_EXECUTABLE=/path/to/chromium
```

`PLAYWRIGHT_MODULE` defaults to the installed `playwright` module. Omitting `CHROMIUM_EXECUTABLE` uses that installation's existing Chromium executable; no download is attempted. `PUBLIC_REPORT_ID` selects an exact bundle when the index contains more than one report. The helper rejects non-loopback URLs and known application ports. `PUBLIC_BASE_URL` may include a project prefix ending in `/workspace/`.

Run from the repository root:

```sh
node scripts/public_browser/lifecycle.cjs
node scripts/public_browser/session-oracle.cjs
node scripts/public_browser/performance.cjs
```

Run one browser workload at a time. Performance qualification requires an otherwise idle machine; do not compile, run native replay workers, or run the memory oracle concurrently. Reports and screenshots go to `PUBLIC_BROWSER_OUTPUT`, outside the served site. Preserve a failed report before rerunning it.

The lifecycle checks exact Markdown download, file split/unified diffs, reviewed ticks and bulk review, temporary notes/bookmarks, all HLD/LLD comparison modes, component drill-down, source hover/pin/Escape, Back and reload, malformed/mismatched bundle links, mobile control bounds, and absence of native API requests.

The performance profile runs three fixed seeds, each with ten excluded and explicitly declared warmup pairs followed by 25 sequential navigation sessions with four graph/source pairs. Each seed retains 100 graph and 100 source durations. It uses one desktop headless browser page per seed, a tiny synthetic function, and the immutable static transport's warm cache. It does not measure cold-network delivery, arbitrary repository size, native fresh authorization, or 25 concurrent network users. Graph/source p95 targets remain 300/500 ms. Both the exact UI endpoint and the more inclusive automation endpoint are retained.

The separate memory oracle instantiates 25 independent controllers and transports. Its fake byte transport is not a network or capacity test. Input bundle selection and the exact scope-fault model are recorded in its report.

Every helper records the site-manifest digest, public manifest digest, source and served-asset digests, and all helper digests. Before/after checks reject source, assets, or helper changes during a run. The report's working-tree hashes identify the tested bytes; they are not a claim that an earlier Git commit contained uncommitted code.

## Appearance and entry-mode verification

Use explicit `mode=focus` links for the PR entry journey; direct workspace entry
uses Standard. Check Studio and Classic HLD appearance, application themes and
Settings Save/Cancel independently. Studio is Focus's default and must not alter
LLD evidence or source drill-down. These additional UI checks are not implied
by a historical passing lifecycle report.

The public source and experimental Linux release are available at
[CodeShouldBehave](https://github.com/SSC-SDE/CodeShouldBehave). Local browser
results do not establish automatic PR workflow or hosted dashboard acceptance.
