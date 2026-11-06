# Inactive source preview, architecture acceptance and webhook inbox

November 6, 2026. Phase 6 continuation after merged
[PR #31](https://github.com/SSC-SDE/CodeShouldBehave/pull/31), merge
`c52b5527d8d480c5b9e62863fe3ff4e94cf4bab5`. Activation remains **off**.

## Browser journey

1. Sign in, select an installed public repository, choose paths and save scope.
2. On a host with the preview worker configured, check the one-time source consent
   and choose **Preview selected paths**. Merely installing, selecting, saving or
   reopening a repository never starts source reading.
3. Review component candidates, stack observations, exact source evidence and gaps.
   Correct names or narrow evidenced roots; include, reject or defer each component.
4. Accept known gaps and explicitly defer all hosting/integration connections, then
   choose **Confirm architecture · keep checks off**. No publication consent or
   automatic analysis grant is created. Reload shows the saved receipt.
5. **Revise scope** starts a new revision. Old architecture acceptance is never
   silently carried onto a different scope or commit.

Include the relevant `Cargo.toml` when selecting Rust sources. Without an evidenced
manifest boundary, the preview reports a gap and cannot accept an invented
component. Unsupported files remain visible as coverage gaps without source reads.
This is an architecture onboarding preview, not a PR behavior comparison.
The existing HLD/LLD viewer and live Actions/Pages demo are unchanged.

## Exact boundaries

The broker obtains fresh administration/installation access, verifies the saved
scope revision and default-branch inventory fingerprint, and reads only selected
supported immutable blobs. It never calls a path-based Contents API, checks out a
repository, runs hooks/builds/package scripts, follows symlinks or contacts detected
providers. Supported preview inputs: Rust, Cargo manifests and static
Vercel/Netlify/Railway configuration. Integration connections remain deferred.

The worker verifies each Git blob hash and reuses the existing component/hosting
detectors. Content stays in bounded transient memory and an isolated temporary
Git object store; only derived evidence/context is persisted. Actor identity,
commit, scope revision, inventory fingerprint, decisions and acceptance time are
bound to the accepted context. The API never accepts a target user ID.

- At most 32 supported source/config files per scan, 256 KiB per file, 16 MiB
  protocol input and 1 MiB worker output. Larger scopes require narrowing; there
  is no silent sampling by the fetcher. Existing detector sampling limits remain
  explicit gaps.
- One source preview runs per host instance. Metadata/provider request limits
  from the [setup checkpoint](APP_ONBOARDING_DEVELOPMENT.md) also apply.
- Preview lifetime: 15 minutes, 64 saved architecture previews globally, 2 MiB per
  derived document. One immutable acceptance per saved scope revision. Existing
  scope-history limits bound total acceptances.
- Revoked permissions, logout, replaced previews, hash mismatch, a changed commit,
  name/identity drift or a concurrent scope save prevent acceptance.
- No AI, credentials in worker input, browser storage, implicit exports or enabled
  checks. Schema constraints keep activation off and publication consent false.

## API contract

All architecture routes require the current HttpOnly session, exact service Origin
and fresh repository-admin/installation access. Identity inputs are only numeric
`installation_id` and `repository_id`; ownership comes from the session.

| Route | Additional input / result |
| --- | --- |
| POST `/v1/github-app/architecture/preview` | `scope_revision`, `allow_selected_source_scan: true`; returns expiring `preview_id`, component/provider candidates, gaps and activation off |
| POST `/v1/github-app/architecture/confirm` | `scope_revision`, `preview_id`, one `{id,name,roots,decision}` per component, `accept_known_gaps: true`, `defer_integrations: true`; returns exact commit, inactive accepted context and publication consent false |
| POST `/v1/github-app/architecture/status` | Identity only; returns acceptance for the latest saved scope revision, without starting a scan |

A changed name/root requires decision `corrected`; unchanged inclusion uses
`confirmed`. `rejected` and `deferred` exclude the corresponding component.
Observed evidence cannot be edited. No `activate`, export or target-user field is
accepted. Statuses: 400 invalid/unsupported confirmation, 401 login required,
403 access denied/blocked, 409 stale or consumed preview/scope, 429 bounded capacity,
503 worker/provider unavailable. The standard 16 KiB request-body limit applies.

## Operator composition (not a deployment instruction)

`csb-app-host` is an optional `github-app` binary. It binds loopback only and needs
an operator-managed HTTPS reverse proxy, protected state directory and credentials.
No host is qualified and this command has **not** been run against the live App.

```sh
cargo +1.87.0 build --locked --release --features github-app --bin csb-app-host --bin csb-app-preview
```

Required operator arguments:

| Argument | Meaning |
| --- | --- |
| `--listen` | Loopback IP and nonzero port, e.g. `127.0.0.1:3199` |
| `--state` | Absolute private directory; a new directory is created with mode 0700 |
| `--app-id`, `--client-id` | Registered App public identifiers |
| `--origin` | Approved external HTTPS service origin, matching callback configuration |
| `--client-secret-file` | Regular private file, mode 0600 or stricter; no secret CLI value |
| `--webhook-secret-file` | Optional private file; mounts authenticated notification intake only |
| `--python`, `--preview-launcher`, `--preview-engine`, `--preview-sha256` | All four required together to configure source preview |

Preview uses `scripts/app_preview_sandbox.py`. Production construction rejects
non-Linux hosts and incorrect engine digests. The launcher copies verified binary
bytes into private staging, then uses Bubblewrap with network/PID isolation and
cleared environment; `prlimit` applies 512 MiB address space, 30 CPU seconds,
16 MiB file size and 64 file descriptors. Wall timeout is 45 seconds. No fallback
runs unisolated. The operator must independently verify a signed release before
supplying its digest. This new worker has not yet been included in a refreshed
signed release or qualified on the future host.

The launcher exposes setup, not a dashboard/publication service. Its `/workspace`
route reports explicitly unavailable; it does not substitute a fixture report.
No activation argument exists. Never expose its loopback listener directly in
place of the required HTTPS proxy. Auth query/body/cookie logging must be disabled
at that proxy before live qualification.

## Webhook foundation

The optional `/v1/github-app/webhook` route verifies raw HMAC-SHA256 before parsing,
limits bodies to 1 MiB, rejects ambiguous headers/unknown installations and stores
only bounded notification metadata and a body digest. PR bodies, source, tokens
and proposed permission/commit assertions are not retained as authority.
Duplicates are idempotent; reused delivery IDs with different contents conflict.

The durable inbox has 60-second leases, five attempts and a hard 1,000-entry cap.
There is no silent cleanup or paid fallback. Removal, privacy conversion and
uninstall/suspension block the selected repository or installation, invalidate
pending leases and fence cached scope/architecture reads. Safety blocks persist
even if intake capacity is exhausted. A later added/unsuspended event cannot
restore old consent. Reconciliation/explicit restoration is still required.

**Claiming a notification does not enqueue analysis.** The fresh-provider
reconciler, fair analysis scheduler binding and App installation-token broker are
not implemented by this inbox. It is unmounted in the factory workspace.
Existing public Actions workflow/check ownership is unchanged.

## Validation and remaining work

Native tests exercise the real detectors and temporary Git objects using synthetic
sources. HTTP tests exercise consent, Origin, permissions and inactive confirmation.
Browser tests intercept all requests locally. Sandbox tests verify launcher policy
and fail-closed behavior; they do not qualify a Linux host.

```sh
cargo +1.87.0 test --locked --all-features
cargo +1.87.0 clippy --locked --all-targets --all-features -- -D warnings
python3 -m unittest discover -s tests -p 'app_preview_sandbox_test.py'
```

Still required before automatic hosted PR checks: a qualified USD 0 host and
signed worker release; live OAuth; installation-token issuance and current-PR
reconciliation; isolated PR worker scheduling; publication consent; the App-specific
signed report receipt/promotion/outbox and check writer; withdrawal/restore and
real second-repository HLD/LLD/source acceptance. Activation remains a separate
owner approval. This checkpoint does not complete the one-click App rollout.

Protocol references: [GitHub immutable blob reads](https://docs.github.com/en/rest/git/blobs#get-a-blob)
and [webhook validation](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries).
