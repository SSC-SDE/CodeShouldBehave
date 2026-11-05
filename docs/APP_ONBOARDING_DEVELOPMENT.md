# App onboarding development checkpoint

Phase 6 / APP-1, November 6, 2026. **Local implementation, activation off.**
The install/select/confirm/automatic-check experience is still in development.
Installing the registered App today does not enable this hosted profile.

## What is implemented

- The existing Rust workspace exposes `GET /v1/github-app/capability` with
  `available: false` and `activation: "off"`. Its login route returns 503 and
  does not contact GitHub. Existing local device login and CI reports are unchanged.
- `github_app_setup::AppSetupHost` implements browser-bound, single-use OAuth
  state, S256 PKCE, session rotation, expiry/logout and current authorization.
  State lifetime: ten minutes; local session lifetime: at most one hour or the
  provider token expiry, whichever is earlier. Restart requires login again.
- The optional `GitHubWebProvider` exchanges web authorization codes, calls
  `/user`, enumerates user-accessible installations and their repositories,
  and accepts only public repositories with current user admin rights and
  matching, non-suspended App installation permissions. Names are display fields;
  numeric App/installation/repository/user IDs establish ownership.
- Paginated inventory rejects missing/changed totals, duplicate entries and
  truncation. Provider operations allow at most 32 calls over 30 seconds, ten
  seconds per call, 2 MiB per response and four concurrent provider operations.
  Admission returns an explicit capacity error; there is no automatic retry storm.
  This is a local bound, not a qualified production rate or capacity claim.
- TLS uses fixed GitHub destinations and checked certificates. Redirects, ambient
  proxy configuration, cookies and compression are disabled in the HTTP client.
  Client secrets/access tokens stay server-side; Authorization headers are marked
  sensitive. Provider error bodies never appear in HTTP responses.
- Migration 15 adds OAuth state digests and unconfigured repository metadata.
  It preserves existing review/preference tables. The schema cannot persist an
  enabled state; selecting a repository neither fetches source nor creates jobs.

No live GitHub OAuth request has been made for this implementation. Provider
responses in acceptance tests are synthetic. See [evidence](evidence/P6.app1/acceptance.md).

## Local verification

Use Rust 1.87.0 and Python 3.11+ from the repository root:

```sh
cargo +1.87.0 test --locked --all-features
cargo +1.87.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.87.0 clippy --locked --all-targets --no-default-features -- -D warnings
python3 -m unittest discover -s tests -p 'public*test.py'
```

The Python publisher tests use local Unix sockets and fake providers. A sandbox
that prohibits socket binding cannot run that test; allow local sockets rather
than removing a production check. These commands do not deploy or enroll a repo.

`--features github-app` enables the HTTPS adapter and implies `server`. The
factory workspace deliberately mounts only `disabled_router()` even with that
feature. Tests construct `router(Arc<AppSetupHost<P>>)` explicitly. There is no
production launcher or configuration switch that silently mounts active routes.
Host composition and browser setup are the next implementation work.

## Setup API contract for the next UI step

The following routes exist only on the explicitly constructed setup router:

| Method / route | Input and result |
| --- | --- |
| GET `/v1/github-app/login` | Optional positive `installation_id` hint; redirect to GitHub with state/PKCE and a secure browser-binding cookie |
| GET `/v1/github-app/callback` | `state`, `code` and matching cookie; consumes state before exchange; sets a fresh session cookie; returns authenticated capability JSON |
| GET `/v1/github-app/repositories` | Session cookie; fresh provider-verified choices; no GitHub tokens, source or context |
| POST `/v1/github-app/repositories` | Exact service Origin and session; only `installation_id`, `repository_id`; revalidates access and records `installed_unconfigured`; `scan_started: false` |
| POST `/v1/github-app/logout` | Exact service Origin/session; revokes session and clears cookie |

All routes send `Cache-Control: no-store`, `Referrer-Policy: no-referrer`,
`X-Content-Type-Options: nosniff` and restrictive CSP. Cookies use the `__Host-`
prefix, Secure, HttpOnly, SameSite=Lax and Path=/. Duplicate session cookies,
caller-supplied user identities and mismatched Origin are rejected. Request
bodies are limited to 4 KiB. Setup error codes distinguish 400 invalid, 401
login required, 403 access denied, 429 capacity and 503 provider unavailable.
No unauthorized repository names are returned.

## Remaining work, in order

1. Repository picker, post-install setup return and browser login/error handling.
   Preserve explicit unavailable state while the host is unconfigured.
2. Path metadata, explicit **Preview selected paths** authorization, exact-commit
   scan, component/gap editing and atomic scope/architecture/publication consent.
   Connect to the existing onboarding and scope model; do not create competing
   analysis identities or silently scan an entire repository.
3. Verified webhook inbox, durable jobs, revocation, isolated workers and resource
   limits measured on an approved Linux host.
4. App-specific signed publication receipts, safe recovery and App-owned checks
   linking exact Focus reports; no simulated GitHub Actions provenance.
5. Real fresh-repository browser acceptance, HLD/LLD/source traversal, withdrawal,
   backup/restore and explicit operator activation approval.

The owner has no existing server/cloud account. [Hosting qualification](APP_HOSTING_OPTIONS.md)
is pending; no cloud resources, App permissions, callback URLs or secrets were
changed. Before live OAuth, configure approved HTTPS origins and a client-secret
store, suppress query/body/cookie logging on auth routes, and review the concrete
host/permission/deployment diff. No customer should need an API key or terminal.

Protocol references: [GitHub web authorization and PKCE](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app),
[installation/user repository APIs](https://docs.github.com/en/rest/apps/installations),
[HTTP client configuration](https://docs.rs/ureq/3.5.0/ureq/config/struct.ConfigBuilder.html).
