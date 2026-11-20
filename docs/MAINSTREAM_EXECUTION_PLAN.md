# Mainstream Rust HLD/LLD review — execution plan

Status: **planning only**. No implementation, merge, tag or deployment is
authorized by this document. Each owner gate is marked explicitly.
Baseline: public `main` `2d8347ca3ca85296fc8a098cfcbd752c77cb8f42`.
Execution checkout: `/Users/shankhyachatterjee/Code/CodeShouldBehave-csb-cli`
(clean, on `main` before starting). Do **not** use
`/Users/shankhyachatterjee/Code/CodeShouldBehave-public`; it holds unrelated
uncommitted design/LLD edits.

Read with:
- `docs/CLI_USAGE.md` — current CLI contract.
- `docs/AI_EXTENSION_IMPLEMENTATION_PLAN.md` — the approved server-side AI
  contract for steps 7.1–7.8.
- `templates/csb-ci/README.md` — the per-repository Pages path (PR #89).

---

## 0. Rules for the executing agent

1. One milestone per branch and PR. Never implement two milestones on one
   branch. Never commit directly to `main`.
2. Start each milestone from the latest `origin/main` after the preceding
   milestone PR is provider-confirmed merged.
3. Re-read every cited line before editing; line numbers are from the baseline
   above and may drift.
4. Do not execute reviewed repository code, install its packages, run its
   tests, or contact the network unless the step explicitly says so.
5. Do not invent evidence. Provisional/inferred data must be labeled
   `provisional` or `inferred` in every surface: artifact JSON, HTML, Markdown,
   PR comment and docs.
6. Do not commit generated reports, review output, `target/`, caches, API keys
   or model prompts containing source.
7. Run the narrowest gate for each step, then the milestone gate. Record exact
   commands, SHAs and outcomes in `docs/implementation-progress.md`.
8. Stop and report if a step cannot be completed without widening scope,
   relaxing a limit, deleting unrelated work, or changing a public contract.
9. Merges and tags are owner gates. Prepare them; do not perform them without
   explicit authorization.

---

## 1. Milestone map

| Milestone | Outcome | Branch | PR base | Owner gate |
|---|---|---|---|---|
| M0 | PR #89 merged; container `v0.2.0` published | none (merge existing) | `main` | merge + tag |
| M1 | `csb review --design auto` emits provisional HLD/LLD | `feature/design-auto` | `main` | merge |
| M2 | `csb explain` with Ollama + OpenAI-compatible BYOK | `feature/csb-explain` | `main` | merge + tag |
| M3 | Guided `csb init` context (only if M1 evidence requires it) | `feature/csb-init` | `main` | scope approval |
| Later | Server/Focus AI (7.2–7.4), App activation, Copilot/VS Code | separate | — | separate |

Recommended order: **M0 → M1 → M2 → decide M3**.

---

## 2. M0 — distribution unblock (no new code)

### M0.1 Verify PR #89 is mergeable

```sh
cd /Users/shankhyachatterjee/Code/CodeShouldBehave-csb-cli
git fetch origin --prune
gh pr view 89 --json state,isDraft,mergeable,headRefOid,statusCheckRollup
gh pr checks 89
```

Required: `state=OPEN`, `mergeable=MERGEABLE`, `wake` check passed, head
`05877c67`. If the branch is stale, rebase is not authorized; report instead.

### M0.2 Owner merge gate

**Owner authorization required.** Merge strategy must match repository policy;
record the provider merge commit. Do not squash/rebase without instruction.

```sh
gh pr merge 89 --merge   # only after explicit authorization
gh pr view 89 --json state,mergeCommit
```

Acceptance: `state=MERGED` with a provider-confirmed commit SHA.

### M0.3 Tag and publish `v0.2.0`

Only after M0.2. Tag the merge commit on `main`:

```sh
git switch main && git pull --ff-only
git tag -a v0.2.0 -m "csb v0.2.0: verified per-repository Pages adoption"
git push origin v0.2.0
gh run list --workflow "Publish csb container" --limit 3
```

Acceptance:
- Publish workflow succeeded for both `linux/amd64` and `linux/arm64`.
- `ghcr.io/ssc-sde/csb:v0.2.0` exists; index digest recorded.
- Anonymous pull works:

```sh
docker logout ghcr.io
docker pull ghcr.io/ssc-sde/csb:v0.2.0
docker run --rm ghcr.io/ssc-sde/csb:v0.2.0 --help
```

- `docs/implementation-progress.md` records merge SHA, tag, run URL and digest
  in a documentation-only PR (or in the M1 branch, not on `main` directly).

Stop conditions: failed attestation, missing architecture, anonymous pull
failure, or a workflow requiring secrets. Report; do not weaken the workflow.

---

## 3. M1 — provisional HLD/LLD in the local CLI (`--design auto`)

Branch: `feature/design-auto` from the M0 merge commit.

Goal: any Rust Cargo repository produces a browsable HLD/LLD review without
web onboarding, with every derived boundary labeled provisional. No AI, no
network, no model.

### M1.0 Confirm the code anchors

Read before editing:

| Anchor | Why |
|---|---|
| `src/bin/csb.rs:308` | `csb_site::emit(..., None)` — design is never emitted |
| `src/csb_site.rs:18-107` | `DesignArtifacts`, `emit`, `design.md`, `artifact.json` |
| `src/report.rs:312` | static “HLD unavailable / LLD unavailable” nav |
| `src/review_artifact.rs:208-264` | existing `generate_hld` / `generate_lld` |
| `src/design.rs:309-469` | `ConfirmedDesignContext`, `from_accepted`, `validate` |
| `src/design/generate.rs:742` | hardcoded “Confirmed onboarding role” assertion |
| `src/onboarding.rs:34-100` | `ComponentDecision`, `ComponentProposal` |
| `src/lld.rs:246-280` | `LldChange`, `LldDelta`, `LldReview` |
| `Cargo.toml` `[features]` | `server` gates design; `csb` has no required features |
| `Dockerfile` | builds `csb` with default features |
| `tests/cli.rs:88` | existing `csb-accepted-context-v1` fixture shape |

### M1.1 Add an explicit authority model

Files: `src/design.rs`, `src/design/generate.rs`, `src/review_artifact.rs`.

1. In `src/design.rs` add:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContextAuthority {
    #[default]
    Confirmed,
    Provisional,
}
```

2. Add `pub authority: ContextAuthority` to `ConfirmedDesignContext`.
   That struct does **not** derive `serde`; set the field inside
   `from_accepted` from `value["authority"] == "provisional"` (absent →
   `Confirmed`). Do not add a serde attribute there.
3. In `src/design/generate.rs:742`, replace the hardcoded assertion text with
   one selected by `context.authority`:
   - Confirmed: `Confirmed onboarding role: {role}`
   - Provisional: `Provisional workspace-derived role: {role}`
4. When provisional, append to the HLD coverage gaps exactly once:
   `Component boundaries are workspace-derived and provisional; runtime ownership and service boundaries are unconfirmed.`
5. Add `pub authority: String` to `ArtifactHld` in `src/review_artifact.rs`,
   set from `context.authority` in `generate_hld`.

Acceptance:
- `cargo +1.87.0 test --locked --features server --lib design` passes.
- Existing accepted-context fixtures decode with `authority = confirmed`.
- A serialized provisional artifact contains `"authority":"provisional"`.

### M1.2 Derive a provisional confirmed-context JSON

New file: `src/design/auto.rs`; export as `pub mod auto;` (or `pub use`) from
`src/design.rs`.

Public function:

```rust
pub fn provisional_context(
    repository: &std::path::Path,
    commit: &crate::SnapshotId,
    scope: &crate::ScopeRecord,
) -> Result<serde_json::Value, String>
```

Rules:

1. Read `Cargo.toml` from the **commit**, never the mutable worktree. Use
   existing Git helpers; if none read a bounded blob, add
   `git::read_commit_file(repository, commit, "Cargo.toml") -> Result<Vec<u8>, String>`
   with a 1 MiB cap.
2. Parse with the `toml` crate (already a `server` dependency).
3. Components:
   - `[workspace] members` present: one component per member path. Expand only
     single-level `*` globs; unsupported patterns add a gap, never fail.
   - Single `[package]`: one component whose root is `"."` if sources are under
     `src/`, otherwise the package directory.
   - Missing `Cargo.toml`: return
     `--design auto requires a Cargo.toml at the reviewed head commit`.
4. Component fields:
   - `id`: lowercase ASCII `[a-z0-9-]`, max 64, from package name or path stem;
     deduplicate with `-2`, `-3`.
   - `name`: package name or path stem.
   - `role`: `Provisional Rust component at {root}`.
   - `repository_index: 0`, `roots: [root]`, `stack_candidates: ["rust"]`,
     `signals: []`, `evidence: []`, `contradictions: ["Workspace-derived
     boundary has no human confirmation"]`, `confidence: "candidate"`,
     `decision: "confirmed"`.
5. Build the accepted-context JSON with:
   - `version: "csb-accepted-context-v1"`, `usable: true`,
     `sensitive_paths_excluded: true`, `authority: "provisional"`.
   - `repository`: exact `scope.primary_repository` fields
     (`security_domain`, `provider`, `owner`, `name`).
   - `commit`: the reviewed head commit.
   - `session_id: "auto-workspace"`, `session_revision: 1`.
   - `input_fingerprint` / `proposal_fingerprint`: SHA-256 over canonical
     component JSON and commit (64 lowercase hex, distinct is allowed).
   - `confirmation`: both fingerprints, `confirmed_at_ms: 0`,
     `actor: "auto-workspace"`.
   - `components`, `providers: []`, `integrations: []`, `contexts: []`,
     `gaps`: provisional warning + any unsupported glob.
6. Reject more than 100 components, roots longer than 100, or a root outside
   `scope` (`safe_path`).

Acceptance:
- Unit test with a synthetic 3-member workspace yields 3 deterministic
  components and valid fingerprints.
- Non-Cargo repository returns the usage error.
- Repeated calls produce byte-identical JSON.

### M1.3 CLI flags and generation plumbing

Files: `src/bin/csb.rs`, `Cargo.toml`.

1. Add to `ReviewOptions`:
   `design: DesignMode` where
   `enum DesignMode { Off, Auto, Context(PathBuf) }`, plus
   `disable_hld: bool`, `disable_lld: bool`.
2. Parse:
   - `--design auto`
   - `--design off` (default)
   - `--design-context <accepted-context.json>`
   - `--disable-hld`, `--disable-lld`
3. Help text: explain `auto` is provisional, `--design-context` is an
   operator-provided confirmed context, and both require a `server` build.
4. Without `--features server`, any non-off design flag returns
   `CliError::usage("--design requires a build with --features server")`.
5. Under `#[cfg(feature = "server")]`, after `analyze_review` and the export
   authorization:
   - Build `confirmed`:
     - auto: `auto::provisional_context(&repository, &result.proposal, result.scope())`;
     - context file: read it as bounded JSON (regular file, no symlink,
       ≤1 MiB) and accept either a full `csb-accepted-context-v1` object or an
       object containing `confirmed_context`. Do **not** use
       `load_onboarding_review_context` here: it requires GitHub provenance and
       will reject local `local-git/local-user` repositories. Pass the object
       to `ConfirmedDesignContext::from_accepted` directly; it already checks
       the repository identity and fingerprints.
   - `let mut artifact = ReviewArtifact::from_analysis(&result);`
   - `artifact.generate_hld(&result, &repository, &confirmed, &grants, &cancellation)?`
     unless `disable_hld`.
   - `artifact.generate_lld(...)` unless `disable_lld` or HLD absent.
   - Build `csb_site::DesignArtifacts` from `artifact.hld` / `artifact.lld`
     (see M1.4).
   - Pass `Some(&design)` to `csb_site::emit`.
6. Keep `--design off` behavior byte-identical to today.

Acceptance:
- `csb review --design` (no value) exits 2 with usage.
- `--design auto` on the test workspace writes `design.md` and `artifact.json`.
- `--design off` output is byte-identical to the pre-change baseline.

### M1.4 Emit a bounded design view

Files: `src/csb_site.rs`, `src/bin/csb.rs`.

1. Extend `DesignArtifacts` with a plain-data view (no design types so
   `csb_site` stays feature-neutral):

```rust
pub struct DesignReport {
    pub authority: String,
    pub hld_status: String,
    pub lld_status: String,
    pub hld_rows: Vec<DesignRow>,
    pub hld_questions: Vec<String>,
    pub hld_gaps: Vec<String>,
    pub hld_mermaid: String,
    pub lld_rows: Vec<DesignRow>,
    pub lld_gaps: Vec<String>,
}
pub struct DesignRow {
    pub kind: String,
    pub before: String,
    pub after: String,
    pub detail: String,
}
```

2. Build rows from `DesignDelta.changes` (`kind`, `before_name`, `after_name`,
   `reason`) and `LldDelta.changes` (`kind`, `classification`, `logical_key`,
   parent component names, `limitations`). Cap: 100 rows per level, 50
   questions/gaps, 500 bytes per cell, 256 KiB total `hld_mermaid`.
3. `csb_site::emit` already writes `design.md` and `artifact.json` when
   `design` is `Some`. Keep that. Add `design.md` sections:
   `# High-level design (provisional)`, baseline/proposal Mermaid, delta rows,
   LLD rows, questions, gaps.
4. `index.json` routes `design` and `artifact` must appear only when design is
   present (already implemented).
5. `csb verify` must cover the new files through the existing manifest; no
   verifier change.

Acceptance:
- `design.md` and `artifact.json` are listed in `manifest.json`.
- `csb verify --dir` returns 0.
- Two runs produce identical `manifest.content_sha256`.

### M1.5 Render HLD/LLD in the self-contained report

Files: `src/report.rs`, `src/style.css`, `src/report-dark.css`,
`src/csb_site.rs`, `src/bin/csb.rs`.

Do **not** add a Mermaid JS dependency. Render structured rows server-side.

1. Change the internal renderer to accept an optional view:
   `fn html(review: &Review, design: Option<&DesignReport>) -> String`.
   Keep `pub fn render_html(result: &Review)` as a wrapper with `None`.
   Add `pub fn render_html_with_design(result: &Review, design: &DesignReport)`.
2. Replace the static block at `src/report.rs:312` when design is present:
   - Nav: `Structural map` + `HLD · provisional` + `LLD · {n} changes`.
   - HLD article: authority badge, outcome, rows table, questions, gaps,
     collapsed `<details><summary>Mermaid source</summary><pre>…</pre></details>`.
   - LLD article: rows table, limitations, gaps, or an explicit
     `LLD unavailable · {status}`.
   - Escape every cell; never inject raw Mermaid into HTML.
3. When design is absent, keep the current “unavailable” copy verbatim.
4. CSS: reuse existing table/badge/`details` styles; add only classes prefixed
   `design-`. No external fonts or scripts.
5. `csb_site::emit` passes the view into `render_html_with_design`.

Acceptance:
- `index.html` contains `HLD · provisional` and the row data for the fixture.
- No `<script src=`, no `http://`/`https://` asset references.
- Existing report tests pass unchanged for `design=None`.

### M1.6 Tests

Files: `tests/csb_cli.rs`, `src/report.rs` unit tests,
`tests/design_report.test.cjs` only if JS changes (none expected).

Add to `tests/csb_cli.rs`:

1. `Repo::cargo_workspace()` fixture:
   `Cargo.toml` with `[workspace] members = ["crates/core", "crates/app"]`,
   each member with `Cargo.toml` + `src/lib.rs`; one base commit and one head
   commit changing a public function and a dependency.
2. `auto_design_emits_provisional_hld_and_lld`:
   run `review --design auto`; assert exit 0 or 1, `design.md`,
   `artifact.json`, `index.html` contains `provisional`, `manifest.json`
   lists design files, `csb verify` passes.
3. `auto_design_is_deterministic`: two runs → identical manifest hash.
4. `auto_design_requires_cargo`: non-Cargo repo → exit 2 with the exact error.
5. `design_off_is_unchanged`: compare output to a pre-change golden fixture
   (store hashes, not full files).
6. `design_context_accepts_operator_file`: use the `tests/cli.rs:88` fixture
   shape; assert `authority = confirmed`.
7. `#[cfg(not(feature = "server"))] design_flag_requires_server`: exit 2.

Acceptance: all new tests pass under `--features server`; core-only test
passes under `--no-default-features`.

### M1.7 Docker and runtime

Files: `Dockerfile`, `docs/CLI_USAGE.md`, `templates/csb-ci/*` (only if flags
need documenting), `CHANGELOG.md`.

1. Change the builder to
   `cargo build --release --locked --features server --bin csb`.
2. Extend the in-image smoke test:
   `csb review --design auto ...` on a Cargo fixture, then `csb verify`.
3. Verify the image still works with the default-feature `csb` path and that
   `--design auto` is available.
4. Record image size. Stop if it exceeds 60 MB compressed; report instead of
   dropping server features silently.
5. Docs: `--design auto`, provisional meaning, limits, `--disable-lld`,
   determinism, no network.

Acceptance:
- `docker build -t csb:design-local .` succeeds.
- In-image `review --design auto` + `verify` pass.
- Image `csb:design-local` runs `help`, `version`, `review`, `serve`.

### M1.8 Milestone gate and release

Run in this order:

```sh
cargo +1.87.0 fmt --check
cargo +1.87.0 clippy --locked --all-targets --features server -- -D warnings
cargo +1.87.0 clippy --locked --all-targets --no-default-features -- -D warnings
cargo +1.87.0 test --locked --features server
cargo +1.87.0 test --locked --no-default-features
node --test tests/*.test.cjs
python3 -m unittest discover -s tests -p 'public*test.py'
git diff --check
```

Plus: browser check of `index.html` at 1440 px and 390 px, no horizontal
overflow, no console errors, HLD/LLD rows visible, `csb verify` OK.

Open `feature/design-auto` → `main` as a draft PR with:
- exact commands and outcomes;
- fixture SHAs;
- image size and digest;
- explicit “provisional” limitation;
- no merge claim.

After owner merge, tag `v0.3.0` and repeat M0.3 verification.

Stop conditions for M1:
- HLD/LLD generation exceeds 16 MiB artifact or 5 minutes on the fixture.
- Determinism fails.
- Any change would require relaxing a bound, weakening labeling, or storing
  reports in the repository.
In those cases, keep `--design off` default and report the measurement.

---

## 4. M2 — `csb explain` with the user’s own model (BYOK)

Branch: `feature/csb-explain` from the M1 merge commit.

Scope: local CLI only. Server/Focus AI (the approved 7.2–7.4 work) stays a
separate, later milestone; this milestone does not weaken or rename it.

Goal: an explicit, auditable explanation over an existing verified review,
using either local Ollama or any OpenAI-compatible endpoint. AI remains
optional, advisory and off by default. Deterministic review output never
changes.

### M2.0 Confirm the code anchors

`src/ai.rs` (contract, `build_evidence_bundle`, `redact_sensitive`),
`src/lib.rs` AI exports, `src/bin/csb.rs` dispatch, `src/csb_site.rs` verify,
`Cargo.toml` features/pins, `docs/AI_EXTENSION_IMPLEMENTATION_PLAN.md` §3–§7.

### M2.1 Feature and module layout

Files: `Cargo.toml`, `src/ai/` (move `src/ai.rs` → `src/ai/mod.rs`),
`src/ai/ollama.rs`, `src/ai/openai.rs`, `src/ai/transport.rs`.

1. Add feature `ai = ["dep:ureq"]`; `default = []` unchanged. `ureq` stays
   pinned `=3.5.0` with rustls. Use `serde_json::Value` for requests/responses;
   no new serde derive is required.
2. Keep `ai` independent of `server`; the CLI must work without Axum/Tokio.
3. `transport.rs`: one bounded HTTP client:
   - 60 s total deadline, no retries, no redirects, no ambient proxy.
   - 64 KiB response cap, JSON only.
   - Never logs headers, keys or bodies.
4. `ollama.rs`: `POST {endpoint}/api/chat`, `stream:false`, `format` JSON
   schema, model identity recorded.
5. `openai.rs`: `POST {endpoint}/chat/completions`,
   `response_format: {type:"json_object"}`, bearer key.
6. Both adapters implement the existing `ModelAdapter` trait but return typed
   provider errors; do not change `ModelCapability` semantics.

### M2.2 Evidence bundle for a review directory

New file: `src/csb_explain.rs` (CLI-side orchestration).

1. `csb verify --dir <dir>` equivalent runs first; refuse on mismatch.
2. Require `artifact.json` with `hld`/`lld`; otherwise exit 2:
   `run csb review --design auto first`.
3. Select exactly one boundary:
   - `--boundary <hld-change-id>` or `--component <component-id>`.
   - No whole-PR summary in this milestone.
4. Build at most 16 evidence spans / 16 KiB UTF-8:
   - the selected HLD change(s), their evidence entries, changed declarations,
     and exact source spans already present in `artifact.json`/`sources/`.
   - redact via `redact_sensitive`; never read unrelated files.
5. Canonical bundle identity: review ID, manifest `content_sha256`, baseline/
   head SHAs, boundary ID, evidence hashes, provider/model, rule version.

### M2.3 Command surface and consent

Extend `src/bin/csb.rs`:

```
csb explain --dir <review-dir> --boundary <id> --provider ollama|openai
            --model <id> [--endpoint <url>] [--key-file <path>]
            [--preview] [--send] [--out <file>]
```

Rules:
- `--preview` (default when neither flag is given): write
  `ai-preview.json` with destination, model, span count/bytes and paths; exit 0.
  No network.
- `--send`: required to dispatch. Refuse if preview was not requested in the
  same invocation unless `--out` exists with the same bundle identity.
- `--provider ollama`: default endpoint `http://127.0.0.1:11434`; only
  loopback or explicitly allowlisted hosts.
- `--provider openai`: endpoint must be `https://`; `--allow-http` only for
  loopback.
- Keys: `CSB_AI_API_KEY` env var or `--key-file` (regular file, Unix mode 0600,
  max 4 KiB). Never accept keys in URLs, prompts or logs. No key is required
  for loopback Ollama.
- Outputs: `ai.md` and `ai.json` in the review directory (or `--out` base).
  The deterministic report and manifest are untouched; `ai.md` is not added to
  the manifest.

### M2.4 Output schema and validation

Server-side schema enforcement, even though the CLI is local:

```json
{
  "headline": "string <= 200",
  "changes": [{"claim": "string <= 500", "side": "baseline|proposal|both", "evidence_ids": [0]}],
  "questions": ["string <= 300"],
  "unknowns": ["string <= 300"]
}
```

Reject: invalid JSON, unknown fields, more than 10 changes / 10 questions /
10 unknowns, citations outside the bundle, stale bundle identity, HTML tags or
control characters. Render text only. Prefix the rendered `ai.md` with
`Advisory AI interpretation — not observed evidence`.

### M2.5 Tests

Files: `tests/csb_explain.rs` (new), `src/ai/*` unit tests,
`tests/design_report.test.cjs` only if UI changes (none).

Use a std `TcpListener` fake provider; no external network.

1. Preview sends nothing and lists exact spans/destination.
2. Ollama request shape: `stream:false`, model, schema; response parsed.
3. OpenAI-compatible request: bearer header present; key never echoed.
4. Unknown/foreign citation rejected; oversized lists rejected.
5. Timeout, truncated body, non-JSON, HTTP 500 → exit 5, report unchanged.
6. Secret sentinel in source is redacted before dispatch.
7. Endpoint allowlist: remote HTTP rejected, redirects rejected.
8. Deterministic files are byte-identical before/after `explain`.
9. `--send` without preview/consent is refused.
10. `cargo test --no-default-features` still compiles `csb` without `ai`.

Define exit code 5 = AI/provider failure; document it in `docs/CLI_USAGE.md`.
Do not renumber existing 0–4.

### M2.6 Docs and privacy

Files: `docs/CLI_USAGE.md`, `docs/AI_USAGE.md` (new), `CHANGELOG.md`,
`templates/csb-ci/README.md` (note: CI reports are static; AI is local-only).

Cover:
- Ollama quickstart (`ollama pull qwen2.5-coder:7b`, endpoint, model).
- OpenAI-compatible endpoints (OpenAI, Azure, OpenRouter, vLLM, LM Studio).
- Claude via an OpenAI-compatible gateway; no native adapter yet.
- Copilot/Codex: no third-party inference API; not supported as providers.
- Subscriptions do not grant API access.
- Exact disclosure preview, redaction limits, no auto-fallback cloud, no key
  storage in the repository.
- AI is advisory; it cannot change scope, design, checks or merge state.

### M2.7 Milestone gate and release

Same test matrix as M1.8 plus `cargo test --features ai` and
`cargo test --no-default-features --features ai`. Browser evidence: none
required for the deterministic report; `ai.md` inspected manually.

Container: build with `--features server,ai`; extend the smoke test to
`csb explain --help` and a fake-provider preview. Record image size; ceiling
for M2 is **80 MB** compressed. Document that local Ollama inside a container
needs an explicit host endpoint or host networking, and that no key is
required for loopback.

After owner merge, tag `v0.4.0`; repeat M0.3.

### M2.8 Server/Focus follow-up (not this milestone)

The approved `docs/AI_EXTENSION_IMPLEMENTATION_PLAN.md` 7.2–7.4 work remains:
capability states, consent/payload preview in Focus, memory cache,
single-flight, reauthorization, UI states, eval corpus. It reuses the adapters
from M2.1 but is a separate PR. Do not claim 7.x completion from M2.

---

## 5. M3 — guided `csb init` (conditional)

Start only if M1 evidence shows provisional HLD/LLD is materially wrong for
real Rust workspaces (e.g., multiple components collapsed into one, or
boundaries contradict `Cargo.toml`).

Scope:
- `csb init --repo .` asks at most five questions (workspace members,
  component roots, exclusions, role labels, known gaps).
- Writes `.codeshouldbehave/context.json` locally (never auto-committed) with
  `authority: confirmed`, the same schema as the accepted context.
- `csb review --design-context .codeshouldbehave/context.json` consumes it.
- No web UI, no OAuth, no server.
- Tests mirror M1.6 plus confirmation fingerprint stability.
- Owner decides whether to merge; no release until M1 has usage evidence.

---

## 6. Global definition of done

A milestone is done only when all are true:

1. The scoped behavior works on the checked-in fixture and on this repository.
2. The full gate matrix for the touched features passes, recorded with exact
   SHAs and commands.
3. Output stays deterministic and `csb verify` passes.
4. No report, cache, key or prompt is committed.
5. Docs and `CHANGELOG.md` describe the exact supported surface and limits.
6. The PR states what was **not** verified.
7. The tracker records the PR, tested head, gate evidence and merge SHA (after
   the owner gate).

---

## 7. Risks and stop conditions

| Risk | Mitigation / stop condition |
|---|---|
| Provisional context is mistaken for confirmed architecture | Authority is serialized and rendered everywhere; provisional gap is mandatory; docs repeat it. |
| HLD/LLD generation too slow or too large | Measure on the fixture and this repo; keep `--design off` default; stop at 5 min / 16 MiB. |
| Determinism breaks | Two-run manifest equality is a gate; no clocks in design payloads. |
| Container grows past 60 MB | Measure; report; do not silently drop `server`. |
| AI leaks source or keys | Preview-before-send, loopback default, HTTPS for remote, redaction, no logs, fake-server tests. |
| AI output invents claims | Citations required, schema validated, advisory label, no write-back. |
| Scope creep into the App/service | M2 is CLI-only; 7.x server work is a separate milestone. |
| Worktree contamination | Use the CSB CLI checkout; never edit the dirty public checkout. |

Owner decisions (defaults preselected):

1. `--design` default: **off** in M1; flip to `auto` only after usage evidence.
2. Container size ceiling: **60 MB** compressed.
3. AI key storage: **env var or 0600 file**; OS keychain is deferred.
4. AI providers in M2: **Ollama + OpenAI-compatible only**; no native Claude
   adapter, no Copilot/Codex.
5. M3: **conditional**, not committed.
