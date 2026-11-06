# App onboarding development checkpoint

Phase 6 / APP-1 and inactive APP-2 scope setup, November 6, 2026. **Local implementation, activation off.**
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
  Migration 16 adds expiring scope previews and immutable inactive scope revisions.
  It preserves existing review/preference tables. The schema cannot persist an
  enabled state; selecting a repository neither fetches file contents nor creates jobs.

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
feature. Tests construct `router(Arc<AppSetupHost<P>>)` explicitly. The optional `csb-app-host` launcher explicitly composes setup routes only; no
configuration switch enables automatic PR analysis or publication.
The browser screen is available at `/github-app/setup`; the factory host shows
setup unavailable and makes no account-service calls beyond its capability check.
The explicitly mounted router supports the picker and scope review after login.
The optional inactive host launcher, explicit source preview, architecture review
and webhook inbox are described in [source preview](APP_SOURCE_PREVIEW.md).
Live OAuth/host qualification and automatic check delivery remain pending.

## Browser setup and scope review

1. Open `/github-app/setup` on the optional Rust service. On the factory host,
   expect **Setup unavailable · checks off**. This page is not deployed to Pages.
2. On an explicitly configured test/setup host, **Continue with GitHub** returns
   to the picker. Only installed public repositories with current admin access
   appear. A setup `installation_id` hint affects ordering, never authorization.
3. Select a repository. This reads GitHub repository/ref/commit/tree **metadata**
   for the exact default-branch commit. The ordinary commits endpoint (which can
   return patches), Contents API and blob API are not used. No source scan occurs.
4. Select file or directory roots and optional exclusions. A directory includes
   its eligible descendants. Sensitive paths, symlinks and submodules cannot be
   included. Use **Review scope** to inspect the exact file list and coverage limits.
5. Acknowledge the displayed boundary, then **Confirm scope · keep checks off**.
   This saves an immutable inactive intent with numeric actor, commit, tree, path
   policy and inventory fingerprint. It is not architecture acceptance, an analysis
   grant, publication consent or review-default activation.
6. Reload to retrieve the saved revision; use **Revise scope** for a new preview.
   Edited paths invalidate confirmation. Expired previews, changed commits or
   competing saves return 409 and require reload/review. No silent overwrite.

Inventory is capped at 2,000 entries and 256-byte paths. Truncated/oversized or
malformed trees fail closed. Each scope has at most 32 included and 32 excluded
roots and a 512 KiB stored document. Previews expire after 15 minutes (256 global);
confirmation history is capped at 100 revisions per repo and 1,024 global rows.
Capacity errors do not erase accepted history or enable work. These are defensive
limits; final host quotas and retention still need qualification.

The UI has one active step, labeled controls, keyboard focus recovery, small-screen
stacking and reduced-motion support. It stores no setup data in browser storage.
Logout or access denial clears repository/scope UI. Network errors preserve an
explicit uncertain state; reload checks the server revision before retrying.

## Setup API contract

The following routes exist only on the explicitly constructed setup router:

| Method / route | Input and result |
| --- | --- |
| GET `/v1/github-app/login` | Optional positive `installation_id` hint; redirect to GitHub with state/PKCE and a secure browser-binding cookie |
| GET `/v1/github-app/callback` | `state`, `code` and matching cookie; consumes state before exchange; sets a fresh session cookie; redirects to the same-origin setup screen |
| GET `/v1/github-app/repositories` | Session cookie; fresh provider-verified choices; no GitHub tokens, source or context |
| POST `/v1/github-app/repositories` | Exact service Origin and session; only `installation_id`, `repository_id`; revalidates access and records `installed_unconfigured`; `scan_started: false` |
| POST `/v1/github-app/inventory` | Repository identity, Origin/session; bounded metadata and latest inactive scope |
| POST `/v1/github-app/scope/preview` | Identity, exact commit, expected revision, includes/excludes; expiring actor-bound preview |
| POST `/v1/github-app/scope/confirm` | Identity, preview ID, expected revision and explicit acknowledgement; rechecks current inventory/access and atomically saves an inactive revision |
| POST `/v1/github-app/logout` | Exact service Origin/session; revokes session and clears cookie |

All routes send `Cache-Control: no-store`, `Referrer-Policy: no-referrer`,
`X-Content-Type-Options: nosniff` and restrictive CSP (the setup HTML permits only same-origin scripts, styles and
API requests). Cookies use the `__Host-`
prefix, Secure, HttpOnly, SameSite=Lax and Path=/. Duplicate session cookies,
caller-supplied user identities and mismatched Origin are rejected. Request
bodies are limited to 16 KiB. Setup error codes distinguish 400 invalid, 401
login required, 403 access denied, 409 stale preview/revision, 429 capacity and 503 provider unavailable.
No unauthorized repository names are returned.

## Remaining work, in order

The source-preview and inactive architecture acceptance slice is now implemented;
see [current boundaries and operator notes](APP_SOURCE_PREVIEW.md). Remaining:

1. Qualified USD 0 Linux host, independently verified signed preview worker and
   live OAuth/permission acceptance.
2. Installation-token broker, current-PR reconciliation, fair analysis scheduling
   and full isolated PR worker binding. The durable webhook inbox is implemented.
3. Explicit publication consent, App-specific signed report receipts/promotion,
   safe recovery, outbox and App-owned exact Focus check delivery.
4. Real fresh-repository browser acceptance, HLD/LLD/source traversal, withdrawal,
   backup/restore and explicit activation approval.

The owner has no existing server/cloud account. [Hosting qualification](APP_HOSTING_OPTIONS.md)
is pending; no cloud resources, App permissions, callback URLs or secrets were
changed. Before live OAuth, configure approved HTTPS origins and a client-secret
store, suppress query/body/cookie logging on auth routes, and review the concrete
host/permission/deployment diff. No customer should need an API key or terminal.

Protocol references: [GitHub web authorization and PKCE](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app),
[installation/user repository APIs](https://docs.github.com/en/rest/apps/installations),
[HTTP client configuration](https://docs.rs/ureq/3.5.0/ureq/config/struct.ConfigBuilder.html).


## Offline browser verification

The optional browser test uses installed Playwright and Chrome. It intercepts
all requests with synthetic responses; it does not authenticate or access GitHub.
It is separate from the Rust tests of the real router/storage/provider adapter.

```sh
PLAYWRIGHT_MODULE=/absolute/path/to/playwright \
CSB_CHROMIUM_EXECUTABLE=/absolute/path/to/chrome \
node scripts/app_setup_browser.cjs
```

Expect thirteen passing UI scenarios and screenshots/result JSON in
`target/app-setup-browser/`. Covers unavailable host, picker/path selection,
confirmation invalidation, inactive save, reload, conflict, provider failure,
mobile/focus, access revocation and logout. See
[scope setup evidence](evidence/P6.app2/acceptance.md).
