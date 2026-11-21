# Mainstream Rust HLD/LLD review — implementation plan

Status: **M0–M2 complete. M3 conditional. M4 implemented on branch; draft PR #101 is open and
owner merge is pending.**
Current release: `v0.5.0` (LLD body/call evidence).
Release baseline: `bd227af64d84339aa6ae231b1c6333abdc629731`.
Plan maintenance since then is documentation-only; current `main` also includes
the consolidated status and this plan update.

Execution checkout: `/Users/shankhyachatterjee/Code/CodeShouldBehave-csb-cli`.
Do not use `/Users/shankhyachatterjee/Code/CodeShouldBehave-public`; it holds
unrelated working-tree edits.

Read with:
- `docs/CLI_USAGE.md` — current CLI contract.
- `docs/AI_USAGE.md` — providers, keys, evidence and limits.
- `docs/implementation-progress.md` — release and evaluation ledger.
- `docs/AI_EXTENSION_IMPLEMENTATION_PLAN.md` — separate server/Focus contract.

---

## 0. Rules for the executing agent

1. One remaining milestone per branch and PR. Never commit to `main` directly.
2. Start from the latest `origin/main` after the preceding milestone merges.
3. Re-read the cited code before editing; this plan does not freeze line numbers.
4. Do not invent evidence. Provisional or inferred data keeps an explicit label.
5. Do not commit generated reports, review output, `target/`, caches, prompts
   containing source, or credentials.
6. AI remains advisory and off by default. It never changes the deterministic
   review, scope, design, checks or merge state.
7. Run the narrowest check for each step, then the milestone gate. Record exact
   commands, SHAs and outcomes in `docs/implementation-progress.md`.
8. Stop and report if work needs a widened scope, a relaxed bound, deletion of
   unrelated work, or a public-contract change.
9. Merges and tags are owner gates unless explicitly authorized for that scope.

---

## 1. Completed baseline

The local CLI/container track is merged and released. Do not re-implement it.

| Release | Merge | Capability |
|---|---|---|
| `v0.3.0` | `de7a7ad7` | `csb review --design auto`; provisional HLD/LLD from a committed Cargo workspace; explicit `confirmed`/`provisional` authority |
| `v0.4.0` | `b4360925` | `csb explain`; local Ollama and OpenAI-compatible BYOK; preview-first; citation and schema validation |
| `v0.4.1` | `c92d9aae` | 1024-token budget; truncation detection; five-item schema; uncertainty guard and neutral headline |
| `v0.5.0` | `ffb409dd` | LLD declaration-body and lexical-call evidence; `source_kind`/`lld_entry` |

All images are `linux/amd64` + `linux/arm64`, anonymously pullable, and
smoke-tested in-image. `:latest` points at `v0.5.0`.

Current supported surface:

- `csb review --design auto` and `--design-context`; `--design off` is default.
- `csb explain` with preview as default and explicit `--send`.
- Evidence: HLD spans, most-specific LLD body, lexical call observations,
  16 spans / 16 KiB, redacted.
- Providers: Ollama and OpenAI-compatible only.
- Model guidance: `qwen2.5-coder:7b` is the default; 1.5B/3B are narrow-boundary
  only and fail broad boundaries with invented citations.

Do not add a sixth provider, a larger default model, an agent framework, tool
execution, a vector store, or live AI in the static report as part of M3/M4.

---

## 2. M3 — guided `csb init` (conditional)

Status: **not started; only if evidence requires it.**
Trigger: M1 provisional boundaries are materially wrong for real Rust
workspaces (components collapse, roots contradict `Cargo.toml`, or repeated
manual correction is observed). Otherwise skip M3.

Scope:

- `csb init --repo .` asks at most five questions: workspace members, component
  roots, exclusions, role labels, known gaps.
- Writes `.codeshouldbehave/context.json` locally with `authority: confirmed`,
  using the same accepted-context schema; never auto-committed.
- `csb review --design-context .codeshouldbehave/context.json` consumes it.
- No web UI, OAuth, server, network or new dependency.
- Tests mirror the existing design-context fixture plus confirmation
  fingerprint stability and deterministic output.
- Acceptance: the generated context passes `ConfirmedDesignContext::from_accepted`,
  produces the same artifact as a hand-written confirmed context, and never
  overwrites an existing file.

Owner decision required before starting. If skipped, record why in the tracker.

---

## 3. M4 — component and caller context (implemented on branch)

Status: **implemented; draft PR #101 is open and owner merge is pending.** Both component and caller
context were implemented (the default recommendation). M3 remains conditional
and unchanged.
Reason: real-model testing showed that deeper explanation quality needs better
evidence scope, not a larger model. Single-declaration boundaries produce
restatement and generic questions.

### 3.1 Component-level boundary selection

- Accept `--component <component-id>` (or `--boundary component:<id>`).
  Resolve the component from `artifact.hld.baseline.elements` where
  `kind == "component"`.
- Build evidence from the component's changed contracts, relations, possible
  impacts and LLD entries whose `parent.component_id` matches.
- Include at most 8 declarations and 8 relations, then apply the existing
  16-span / 16 KiB cap and redaction.
- Label derived component facts `source_kind: "component"`.
- Acceptance: a component selection yields at least two declarations or
  relations; a 7B explanation cites more than one item and references at least
  one relation; the deterministic report is unchanged.

### 3.2 Caller-graph context

- For each selected declaration, collect at most 8 callers from
  `lld.delta.possible_callers` and caller entries in the same side.
- Include the caller signature and call-site line when retained; otherwise the
  logical key only.
- Label callers `source_kind: "call_candidate"` and state that they are lexical
  candidates, not resolved runtime calls.
- Deduplicate by caller entry ID; keep the 16-span / 16 KiB cap.
- Acceptance: an explanation names at least one caller and cites it; it makes
  no runtime-order or reachability claim; absent callers produce an explicit
  unknown, never an invented edge.

### 3.3 Boundary discovery UX

- Add `csb explain --list` printing boundary ID, kind, path, declaration name,
  evidence bytes and the `uncertain` flag.
- Keep explicit selection mandatory; never silently explain a whole PR.
- Sort deterministically and cap the listing.

### 3.4 Gates and stop conditions

- Fixtures with two callers and a component containing multiple declarations.
- Real Ollama 7B comparison against the current single-declaration output;
  record latency, citation use and whether questions become change-specific.
- `csb verify` and the deterministic manifest stay byte-identical.
- No new provider, no larger default model, no whole-repository graph.
- If caller evidence is absent or ambiguous, say so; never infer a call graph.
- Stop if evidence exceeds the caps or the explanation cannot cite component or
  caller evidence.

Owner decision required before starting: component-only, caller-only, or both.
Default recommendation: implement 3.1 and 3.2 together, then re-evaluate with 7B.

### Implementation record — 2026-11-21

Branch: `feat/m4-component-caller-context` from `origin/main` `f634683`
(draft PR #101; merge SHA pending).

- 3.1 and 3.2 implemented together: `--component` / `--boundary
  component:<id>` and lexical `call_candidate` evidence.
- 3.3 implemented: `csb explain --list`, no provider contact, no file writes,
  sorted and capped at 100 rows.
- Rule version moved to `csb-explain-v2`; the deterministic review artifact and
  manifest are not written by explain and are unchanged.
- Fixture covers a component with multiple declarations and two lexical callers.
- Gates passed: `cargo fmt --check`; `cargo check --features ai,server --bin
  csb`; `cargo test --features ai,server --test csb_explain` (10 passed);
  `cargo test --features ai,server --lib csb_explain::tests` (4 passed).
  Tests assert `manifest.json` bytes are unchanged and no `ai.md` is written in
  preview mode.
- Real `qwen2.5-coder:7b`: single-declaration boundary 15.1 s and one citation
  with generic questions; component boundary 20.5 s, three citations
  (`4, 5, 8`), and change-specific questions naming `handle`, `other` and the
  dependent `api` component. No runtime-order or reachability claim was made.
- Not verified: the provider result is advisory and was not asserted by tests;
  the model used one local loopback run, not a multi-seed evaluation.

---

## 4. Global definition of done

A milestone is done only when all are true:

1. The scoped behavior works on the checked-in fixture and on this repository.
2. The gate matrix for touched features passes, recorded with exact SHAs and
   commands.
3. Output stays deterministic and `csb verify` passes.
4. No report, cache, key or prompt is committed.
5. Docs and `CHANGELOG.md` describe the exact supported surface and limits.
6. The PR states what was **not** verified.
7. The tracker records the PR, tested head, gate evidence and merge SHA after
   the owner gate.

---

## 5. Risks and stop conditions

| Risk | Mitigation / stop condition |
|---|---|
| Provisional context mistaken for confirmed architecture | Authority is serialized and rendered everywhere; provisional gap mandatory. |
| HLD/LLD generation too slow or too large | Measure on fixture and repo; keep `--design off` default; stop at 5 min / 16 MiB. |
| Determinism breaks | Two-run manifest equality is a gate; no clocks in design payloads. |
| Container size grows unexpectedly | Record size per release; no silent feature removal or growth. |
| AI leaks source or keys | Preview-before-send, loopback default, HTTPS remote, redaction, no logs, fake-server tests. |
| AI invents claims or citations | Schema, citation, markup and truncation validation; advisory label; no write-back. |
| Small models hallucinate citations | 7B default; 1.5B/3B narrow-only; broad boundaries must fail closed. |
| Evidence scope produces restatement | Prefer component/caller context (M4) over larger models. |
| Scope creep into the App/service | CLI-only; server/Focus work is a separate plan. |
| Worktree contamination | Use the CSB CLI checkout; preserve unrelated edits. |

---

## 6. Remaining owner decisions

1. **M3:** start only if the provisional-boundary trigger is observed, otherwise
   record it as skipped.
2. **M4 scope:** component-only, caller-only, or both (default: both).
3. **AI key storage:** env var or 0600 file; OS keychain deferred.
4. **AI providers:** Ollama and OpenAI-compatible only.
5. **`--design` default:** remains `off` until usage evidence supports `auto`.

---

## 7. Explicitly deferred

- Server/Focus AI implementation (approved 7.2–7.4 in
  `docs/AI_EXTENSION_IMPLEMENTATION_PLAN.md`).
- GitHub App activation, qualified USD 0 hosting, signed Linux worker and live
  fresh-repository acceptance.
- Native Anthropic adapter; Copilot/Codex (no third-party inference API).
- VS Code/Copilot extension surfaces.
- Multi-language analysis beyond the current Rust-first profile.
