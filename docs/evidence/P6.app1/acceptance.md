# APP-1 local identity implementation evidence

Date: November 6, 2026 (IST). Branch: `phase-6`. Base:
`c877af8ae2d25c2cf2773c70a0c6fa8499ba3d84` (merged PR #30).
Status: **in_progress**. Local test evidence is not live App acceptance.
No reviewer acceptance, deployment approval or completed phase gate is inferred.

## Scope and boundaries

OAuth/PKCE, numeric identity, current admin/installation intersection, repository
selection and additive migration 15. No source reads, scope grants, worker
jobs, webhook subscription or check/report publication. Factory routes stay off.
Optional `github-app` feature contains the concrete HTTPS adapter; default CLI
and ordinary `server` builds keep their previous network dependency boundary.

## Verification

- Full Rust 1.87 all-feature suite passed: 292 tests, including 25 App identity/
  protocol/HTTP tests and the migration-14-to-15 preservation test.
- Negative tests cover wrong browser, replay, expired state, failed code exchange,
  forged installation hint, removed/private/non-admin/suspended access, mismatched
  numeric user, duplicate/truncated/changed pagination, cross-origin writes,
  duplicate cookies, browser-supplied user ID and logout during provider lookup.
- OAuth PKCE matches the RFC 7636 vector; verifier and credentials are absent
  from redirects/JSON. Repository selection is idempotent and cannot enable jobs.
- Provider tests verify fixed endpoints, TLS-only/no-proxy/no-redirect settings,
  request/concurrency bounds, failed status, oversized and malformed responses.
- Fresh/reopened storage and upgrade tests pass without altering existing review
  schema or saved preference revision/document.
- Exact-lockfile license inventory passed: 108 packages, 200 original notice files.
- Strict Rust 1.87 Clippy passed with all targets for both all-feature and
  no-default-feature builds. The no-default-feature test suite also passed.
- All 119 Python publication tests passed with local Unix sockets allowed. The
  first sandboxed run could not start the synthetic publisher socket; rerunning
  unchanged tests with socket permission passed. No live provider was used.
- Formatting, whitespace and 30 relative documentation links passed validation.
  Cargo dependency trees confirm the offline CLI and existing server omit the
  optional App HTTP/TLS client.

## Not verified / not activated

Live web authorization, TLS proxy integration, browser setup, fresh installation,
real GitHub permission changes and host performance are pending. OAuth protocol
fixtures are explicit fakes; there is no claim of live provider certification.
No new host/account is available. No paid fallback or infrastructure was created.
Existing PR #24 and its Actions/Pages dashboard remain unchanged and unmerged.
