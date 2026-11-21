# Public-repository case studies

Three independent reviews of public Rust changes, generated with the published
`v0.6.0` image (`sha256:53e53a1f6374c58dacf90e1c52eca4130d1c82531f217edb56f31a43e507c075`)
and checked with `csb verify`.

The reports are hosted as immutable release assets:
<https://github.com/SSC-SDE/csb-case-studies/releases/tag/case-studies-2026-11-21>

These are analyses of public commits, not endorsements by the upstream projects.
Every report says coverage is incomplete because the bounded profile does not
evaluate macros, conditional features, every dependency form or non-Rust files.
None of this proves runtime behavior.

| Case | Change | HLD | LLD | Report |
|---|---|---:|---:|---|
| `sharkdp/fd` | `2381562` pattern refactor | 152 changes | 163 changes | [`fd.zip`](https://github.com/SSC-SDE/csb-case-studies/releases/download/case-studies-2026-11-21/fd.zip) |
| `dtolnay/thiserror` | `f82a0cf` nested turbofish | 0 changes | 71 changes | [`thiserror.zip`](https://github.com/SSC-SDE/csb-case-studies/releases/download/case-studies-2026-11-21/thiserror.zip) |
| `BurntSushi/ripgrep` | `e2b508d` line terminators | 0 changes | 25 changes | [`ripgrep.zip`](https://github.com/SSC-SDE/csb-case-studies/releases/download/case-studies-2026-11-21/ripgrep.zip) |

## 1. `sharkdp/fd` — structural refactor

- Base: `b7a0a29d07c3d0e9e0a41155019acada08831a6c`
- Head: `23815624819c77ab665e19c8e6c28bddbcd96e1d`
- Commit: <https://github.com/sharkdp/fd/commit/23815624819c77ab665e19c8e6c28bddbcd96e1d>
- Review: 49 files analyzed, 148 flow landmarks edited, 21 cross-file lexical
  candidates, 1 component, 152 HLD changes, 163 LLD changes.
- Verify index: `c44a29edfed9c5b32c5988e234d80797f2227b002da919744bb3d79202f7d9d8`

The HLD correctly treats this as architecture: new `PatternType`, `Patterns`,
`Matcher`, `FixedStrings`, `MatchAll`, `ExactStrings` and `build_patterns`
declarations replace the removed `osstr_to_bytes` declaration, while `scan` and
`Config` are modified. One component boundary is marked uncertain because the
workspace-derived profile cannot confirm runtime ownership.

**What it shows:** a change that introduces a new abstraction is visible at the
HLD level, not only as a line diff.

## 2. `dtolnay/thiserror` — internal parser change

- Base: `72ea49262d6412ccbc08f1696dda8626409e657d`
- Head: `f82a0cf8f06c71263c2caa26e759f337d2778153`
- Commit: <https://github.com/dtolnay/thiserror/commit/f82a0cf8f06c71263c2caa26e759f337d2778153>
- Review: 138 files analyzed, 15 flow landmarks edited, 0 cross-file lexical
  candidates, 2 components, 0 HLD changes, 71 LLD changes.
- Verify index: `595505b29eff258a6ea22d57950efa1dff0e209f03d51f3355dfd570fb750786`

The change stays inside `parse_token_expr`; HLD correctly reports no
architectural delta. LLD reports signature, assignment/operation, call
observation, condition and unsupported-region changes for that function.

**What it shows:** HLD stays quiet when the architecture does not change, while
LLD keeps the behaviorally relevant detail.

## 3. `BurntSushi/ripgrep` — cross-crate internals

- Base: `97059eacb64e6cd09a11ed362f7d54e7b2a7cd29`
- Head: `e2b508de363c9f16fff2a6bb50e8692287cba015`
- Commit: <https://github.com/BurntSushi/ripgrep/commit/e2b508de363c9f16fff2a6bb50e8692287cba015>
- Review: 204 files analyzed, 7 flow landmarks edited, 38 cross-file lexical
  candidates, 9 components, 0 HLD changes, 25 LLD changes.
- Verify index: `25622ee9af62af872284b8e63fe45563b5e8e3a6f5ca51962a214f8882250020`

The HLD does not overstate this as a component-boundary change. LLD follows the
change through `grep-regex` (`to_regex`, matcher configuration) and
`grep-searcher` (`is_line_by_line_fast`), with 38 lexical cross-file candidates
for reviewer follow-up.

**What it shows:** multi-crate internals remain reviewable without inventing a
new architecture relation.

## What the three cases have in common

- The deterministic report is reproducible from exact commits and the pinned
  digest.
- HLD and LLD separate structural change from internal behavior change.
- Lexical call candidates are useful but labeled as candidates, never resolved
  runtime calls.
- Coverage is reported honestly instead of being rounded up.

## Open a report

```sh
curl -LO https://github.com/SSC-SDE/csb-case-studies/releases/download/case-studies-2026-11-21/fd.zip
unzip fd.zip -d fd
python3 -m http.server 8080 --directory fd
# open http://localhost:8080/
```

Each archive contains the self-contained review site, `manifest.json`,
`artifact.json` and `sources/`; it does not contain the upstream repository.

## Licenses

The reviewed projects retain their own licenses and copyright:

- `sharkdp/fd`: MIT / Apache-2.0.
- `dtolnay/thiserror`: MIT / Apache-2.0.
- `BurntSushi/ripgrep`: MIT / Unlicense.

Limited source spans appear in the reports for review context. Upstream names
identify the commits only.
