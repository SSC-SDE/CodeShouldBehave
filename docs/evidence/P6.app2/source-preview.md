# Explicit source preview and inactive architecture acceptance

November 6, 2026. Phase 6 continuation from provider-confirmed PR #31 merge
`c52b5527d8d480c5b9e62863fe3ff4e94cf4bab5`.

Implemented locally: exact saved-scope scan consent; bounded immutable GitHub blob
adapter; credential-free native preview protocol using existing component/hosting
detectors; digest-pinned Linux sandbox launcher; component corrections/rejection,
explicit gap/integration deferral; immutable acceptance; browser reload/revision
handling; optional inactive loopback host composition. Migrations 17/18 preserve
existing scope, review and preference records.

Evidence:

- Rust 1.87.0 locked all-feature suite: **316 passed**. Core-only: **119 passed**.
- Strict all-target Clippy in both feature configurations, format and whitespace
  checks pass. The base64 crate is now a direct optional dependency; its version
  was already present in the lockfile, so no new registry package was introduced.
- Existing publication suite: **119 passed**; sandbox launcher policy: **4 passed**.
- Offline browser: **13 scenarios passed**, no external requests. Includes source
  opt-in, component correction, both acknowledgement gates, inactive receipt,
  reload and returning to edit scope. Desktop screenshot visually inspected.
- Actual compiled native preview binary: synthetic Cargo/Rust input returned the
  expected component/import evidence; mismatched Git object returned failure and
  no output; source sentinel did not appear in derived output. No repository code
  or installation hooks ran. Inactive host rejected a non-loopback bind.
- Tests reject missing scan consent, absent worker, stale/foreign/replaced/expired
  previews, invalid roots, missing gap acceptance, hash mismatch, permission
  revocation, revised scope, Origin violations and activation field injection.
- Thirty local links checked across changed onboarding/README documentation.

The native detector test on this development machine is **not** a Linux sandbox
qualification. Production construction refuses non-Linux hosts; launcher tests
use mocks and verify refusal when isolation is unavailable. No live OAuth, App
credential, installation, webhook endpoint or signed-release setting changed.
No deployment or activation is claimed. No PR analysis/check writer is connected.

See [operator/use notes](../../APP_SOURCE_PREVIEW.md) for limits and remaining gates.

Implementation commit: `7373248ea3120861431aa86539a61021124edcdf`.
Draft [PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32).
Committed distribution boundary scan: 364 files, zero secret-pattern findings.
No package uploaded; activation off; PR #24 remains unmerged.
