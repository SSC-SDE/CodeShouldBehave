# v0.6.0 fresh-repository acceptance

Date: 2026-11-21. Image: `ghcr.io/ssc-sde/csb:v0.6.0`
(`sha256:53e53a1f6374c58dacf90e1c52eca4130d1c82531f217edb56f31a43e507c075`).

## Fixture

A clean repository with a two-member Cargo workspace and no pre-existing config:

- Baseline `56a14abd0bcc1465c49aee9a2ac7436f18e100f8`: `core::add`; `api::handle`
  and `api::other` both call `core::add`.
- Proposal `b5a74c403a5c0049ea19671d5f167f16be27dfc3`: `core::add` uses
  `checked_add`; `core::subtract` is added.

## Results

| Check | Command | Result |
|---|---|---|
| Review | `csb review --design auto --summary check` | Exit `1`; review written; coverage explicitly incomplete |
| Verify | `csb verify --dir out1` | `OK 8 artifacts · index 2b0073d97d155c24610e0312d58f76590d2448ee34aef80a507580d71faa4483` |
| Determinism | second review into `out2`; `cmp out1/manifest.json out2/manifest.json` | Byte-identical |
| Boundary list | `csb explain --list` | Exit `0`; two deterministic rows with 52 evidence bytes each |
| Component preview | `csb explain --component <core> --model fixture-model` | Exit `0`; no provider contact |
| Preview cap | inspect `ai-preview.json` | 9 items, 991 bytes, max item 311 bytes (limits: 16 items / 16 KiB) |
| Preview shape | inspect `ai-preview.json` | 3 `component`, 3 `lld_body`, 2 `call_candidate` |
| No silent send | check `ai.md` | Absent |
| Report untouched | manifest SHA before/after explain | `84c7e4ba10d415835f3c400947936df23ade027fd078b2a06b110adcdbef0633` unchanged |

Review summary: `4 files analyzed · 6 unsupported`; flow landmarks changed in
`crates/core/src/lib.rs`; no cross-file flow candidates. HLD coverage is
`complete: false` because the workspace-root `Cargo.toml` has no unique
component owner. No boundary was invented for it.

## Provisional boundaries

| Component | Root | Workspace member |
|---|---|---|
| `core` | `crates/core` | `crates/core` |
| `api` | `crates/api` | `crates/api` |

One `local_package_dependency` relation `api → core` is retained. Both component
roots match `Cargo.toml`; the only gap is the unowned workspace-root manifest,
which is disclosed rather than assigned. No component collapsed, no root
contradicted the manifest, and no manual correction was needed.

## M3 decision

`csb init` remains **skipped**. None of the M3 triggers was observed:

- components did not collapse;
- roots did not contradict `Cargo.toml`;
- no repeated manual correction occurred.

The acceptance also produced no usage evidence for changing the `--design off`
default.

## Limitations

- The fixture is small and synthetic; no private repository was used.
- Review coverage is intentionally incomplete for the unowned workspace-root
  manifest.
- Component explanation was preview-only; the provider was not contacted in this
  acceptance run.
- AI remains advisory, opt-in and off by default.
