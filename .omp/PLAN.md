# Execution plan: mainstream Rust HLD/LLD

Read `docs/MAINSTREAM_EXECUTION_PLAN.md` in full before touching code.
Checkout: `/Users/shankhyachatterjee/Code/CodeShouldBehave-csb-cli`
Base: `origin/main` after each preceding merge. One milestone per branch/PR.
Never edit `/Users/shankhyachatterjee/Code/CodeShouldBehave-public`.

## M0 — distribution unblock (owner gate)
- [ ] M0.1 Verify PR #89 mergeable/checks; head `05877c67`.
- [ ] M0.2 OWNER MERGE GATE: merge #89; record provider merge SHA.
- [ ] M0.3 Tag `v0.2.0`; verify publish workflow, multi-arch, anonymous pull; update tracker.

## M1 — `csb review --design auto` (`feature/design-auto`)
- [ ] M1.0 Read anchors: csb.rs:308, csb_site.rs:18-107, report.rs:312, review_artifact.rs:208-264, design.rs:309-469, design/generate.rs:742, Cargo.toml features, Dockerfile, tests/cli.rs:88.
- [ ] M1.1 `src/design.rs` + `src/design/generate.rs` - add `ContextAuthority`, authority assertion text + provisional gap; `src/review_artifact.rs` - `ArtifactHld.authority`.
- [ ] M1.2 `src/design/auto.rs` - `provisional_context()`; read Cargo.toml from commit; workspace members/single package; caps/gaps.
- [ ] M1.3 `src/bin/csb.rs` - `--design auto|off`, `--design-context`, `--disable-hld/-lld`; server-feature gate; call `ReviewArtifact::generate_hld/lld`.
- [ ] M1.4 `src/csb_site.rs` - `DesignReport`/`DesignRow`; bounded rows; `design.md` + `artifact.json`; manifest/index routes.
- [ ] M1.5 `src/report.rs` + CSS - structured HLD/LLD rows, provisional badge, collapsed Mermaid text; no Mermaid JS; preserve unavailable copy.
- [ ] M1.6 `tests/csb_cli.rs` - workspace fixture, provisional emit, determinism, non-Cargo error, off golden, context file, no-server test.
- [ ] M1.7 `Dockerfile` - `--features server`; in-image `--design auto` + verify smoke; size <= 60 MB; docs.
- [ ] M1.8 Gate: fmt, clippy both profiles, tests both profiles, node+python suites, browser 1440/390; draft PR; owner merge then tag `v0.3.0`.

## M2 — `csb explain` BYOK (`feature/csb-explain`)
- [ ] M2.0 Read `src/ai.rs`, lib exports, csb.rs dispatch, Cargo pins, AI plan §3-§7.
- [ ] M2.1 `Cargo.toml` + `src/ai/` - `ai` feature; move ai.rs to mod.rs; `transport.rs`, `ollama.rs`, `openai.rs`; 60s/no-retry/64KiB caps.
- [ ] M2.2 `src/csb_explain.rs` - verify dir; require artifact design; one boundary; <=16 spans/16KiB; bundle identity; redaction.
- [ ] M2.3 `src/bin/csb.rs` - `explain` command; preview default; `--send`; loopback Ollama; HTTPS remote; env/0600 key; `ai.md`/`ai.json`.
- [ ] M2.4 Schema validation - 10/10/10 caps; citation/identity/stale rejection; advisory label; exit code 5.
- [ ] M2.5 `tests/csb_explain.rs` + unit tests - fake server; preview no network; request shape; key safety; failure modes; determinism.
- [ ] M2.6 Docs - `docs/AI_USAGE.md`, CLI_USAGE, CHANGELOG; Ollama/OpenAI-compatible; Claude via gateway; Copilot/Codex unsupported.
- [ ] M2.7 Gate: full matrix + `--features ai`; draft PR; owner merge then tag `v0.4.0`.

## M3 — conditional `csb init`
- [ ] M3 only if M1 evidence shows provisional boundaries are materially wrong. Get scope approval first.

## Stop conditions
- No invented evidence; provisional labels mandatory.
- Stop at >5 min or >16 MiB design artifact; keep `--design off` default.
- No reports/keys/prompts committed. No merge/tag without owner authorization.
