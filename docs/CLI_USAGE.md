# `csb` — local behavioral review

`csb` compares two exact Git commits and writes a self-contained review site:
source diff, structural architecture map, flow landmarks, cross-file call
candidates and an explicit coverage record.

It is deliberately narrow:

- **No network.** No GitHub token, App key, telemetry or model call.
- **No execution.** It never checks out, builds, runs hooks, installs packages
  or executes reviewed code. It reads Git objects only.
- **Exact commits only.** `--base` and `--head` must be full 40-character
  commit IDs. `HEAD`, branch names, tags and abbreviated IDs are rejected.
- **Deterministic.** The same repository and commit pair produce byte-identical
  output. Every artifact is hashed in `manifest.json`.
- **Never overwrites.** `--out-dir` must not exist.

## Install

Container:

```sh
docker pull ghcr.io/ssc-sde/csb:latest
docker run --rm ghcr.io/ssc-sde/csb:latest --help
```

From source:

```sh
cargo install --path . --bin csb
```

## Quick start

```sh
# Pick an exact pair
BASE=$(git rev-parse HEAD~1)
HEAD=$(git rev-parse HEAD)

csb review --repo . --base "$BASE" --head "$HEAD" --out-dir review/

# Serve it locally (file:// is blocked by browser fetch rules)
csb serve --dir review/ --open

# Recheck every artifact hash
csb verify --dir review/
```

Container equivalent:

```sh
docker run --rm \
  -v "$PWD:/repo" \
  -v "$PWD/review:/out" \
  ghcr.io/ssc-sde/csb:latest review \
    --repo /repo --base "$BASE" --head "$HEAD" --out-dir /out

docker run --rm -p 8080:8080 \
  -v "$PWD/review:/review" \
  ghcr.io/ssc-sde/csb:latest serve --dir /review --host 0.0.0.0
```

## Output

```
review/
├── index.html       Self-contained interactive review
├── index.json       Machine-readable index and verdict
├── manifest.json    Content-addressed list of every artifact
├── review.json      Structured review evidence
├── review.md        Full Markdown report
├── summary.md       Pull-request summary
└── sources/         Per-file patch and flow evidence
```

`index.html` inlines its CSS, JavaScript and data. It renders from `file://` in
most browsers, but `csb serve` is the supported path because some browser
profiles block local file reads.

## Commands

### `csb review`

```
csb review --repo <path> --base <sha> --head <sha> --out-dir <new-dir>
           [--merge-base] [--context <scope.json>] [--cache-dir <path>]
           [--allow-dirty] [--summary check|markdown|json]
```

| Flag | Meaning |
|---|---|
| `--repo` | Repository path. Must be a Git worktree. |
| `--base`, `--head` | Exact 40-character commit IDs. Required. |
| `--out-dir` | New directory. Existing paths are refused. |
| `--merge-base` | Compare the merge base of base/head with head. |
| `--context` | Explicit local scope manifest for additional evidence. |
| `--cache-dir` | Analysis cache. Defaults to a per-repository temp directory. |
| `--allow-dirty` | Skip the clean-worktree check. Analysis still reads commits only. |
| `--summary` | Stdout format: `check` (default), `markdown`, `json`. |

A dirty worktree is rejected by default because uncommitted changes are not
part of the comparison. CI should pass `--allow-dirty` when the runner holds
build outputs outside the reviewed commits.

### `csb serve`

```
csb serve --dir <review-dir> [--host <addr>] [--port <port>] [--open]
```

Read-only HTTP server for the generated site. Defaults to `127.0.0.1:8080`.
It rejects path traversal, serves only files inside the directory and sends
`Access-Control-Allow-Origin: *` so the page can load its own artifacts.
Use `--host 0.0.0.0` inside a container with a published port.

### `csb verify`

```
csb verify --dir <review-dir>
```

Recomputes every SHA-256 in `manifest.json`, checks file sizes, and confirms
that `index.json` matches `content_sha256`. Prints `OK <n> artifacts · index
<sha>` or the first failure.

### `csb version`, `csb help`

Print the version or command help.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Review complete; coverage complete within the selected scope. |
| `1` | Review written but coverage is incomplete (unsupported, failed or deferred files). `csb verify` also uses `1` for verification failure. |
| `2` | Usage error: bad arguments, missing repository, dirty worktree, symbolic ref, existing output. |
| `3` | Git error: unknown commit, missing object, analysis failure. |
| `4` | Internal error: invariant violation or artifact write failure. |

## CI integration

Ready-to-copy templates live in [`templates/csb-ci/`](../templates/csb-ci/):

- GitHub Actions posts or updates one pull-request summary comment and uploads
  the interactive review as an artifact.
- GitLab CI and Bitbucket Pipelines publish the review as a pipeline artifact.

All templates pass exact commit IDs from the provider. `--allow-dirty` is used
because CI worktrees commonly contain untracked build outputs.

## Determinism and verification

Two runs over the same repository and commit pair produce identical bytes. The
manifest records:

- `content_sha256`: SHA-256 of `index.json`
- one `sha256` and byte count per emitted artifact

`csb verify` fails if any file changes, is missing, changes size, or if
`index.json` no longer matches `content_sha256`. Volatile cache counters are
excluded from the site so a warm cache cannot change the output.

## Scope and limitations

- Behavioral flow extraction supports Rust plus the parser's JavaScript,
  TypeScript and Python profiles. Unsupported files are counted and disclosed;
  they are never silently treated as unchanged.
- Architecture analysis indexes at most 150 changed files and 512 KiB per blob.
  Truncation and deferral are reported in coverage.
- Cross-file arrows are lexical call-name candidates, not resolved execution
  paths. The report says so in place.
- HLD and LLD cards require a confirmed onboarding context and the server
  feature. The local CLI site shows them as unavailable rather than inventing
  design evidence. Use `flowdiff review --onboarding-context <file>` for that
  workflow.
- The review is advisory. It is not a merge approval and does not prove runtime
  behavior.

## Security model

- Git commands run with `core.hooksPath=/dev/null`, `protocol.allow=never`,
  `GIT_TERMINAL_PROMPT=0` and `GIT_NO_LAZY_FETCH=1`.
- `csb serve` binds loopback by default, canonicalizes every requested path and
  refuses anything outside the review directory.
- The generated HTML uses a restrictive Content-Security-Policy with no network
  origins.
- No secrets are read and no credentials are accepted.

## Troubleshooting

**“the worktree is dirty”** — commit or stash changes, or pass `--allow-dirty`.
The comparison always uses committed objects.

**Blank page when opening `index.html` directly** — use `csb serve --dir review/
--open`. Some browsers block local file reads.

**Port already in use** — pass `--port 0` to choose a free port; the chosen URL
is printed.

**“commit not found”** — fetch the commits first. `csb` never contacts a remote.

**Coverage incomplete** — open `summary.md` or the Coverage gaps section. Add
supported files to the comparison or treat the missing evidence as unknown.
