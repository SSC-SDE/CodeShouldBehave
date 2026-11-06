# App execution and signed delivery: development checkpoint

November 6, 2026. Implemented on `phase-6` in
[PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32).
**Activation remains off. This is a locally tested host integration, not a running
hosted App service. PR #24 and its existing Actions/Pages publisher are unchanged.**

## Implemented

### Installation credentials and fresh PR identity

With the optional `github-app` feature, `GitHubInstallationBroker` parses protected
operator RSA PKCS#1/PKCS#8 key bytes and signs RS256 App JWTs using `ring`.
The issuer is the public Client ID; JWT times allow 60 seconds of clock skew and
stay within GitHub's ten-minute limit. Credentials have no Debug/Serialize
implementation. No credentials are stored in SQLite, reports or worker input.

Issuance checks the current installation's numeric ID, App ID, suspension and
permissions, then requests exactly one `repository_ids` entry with Contents read,
Pull requests read and Checks write. The returned repository and permission set
must match. Variable-length token formats, including dots, are accepted within a
16 KiB cap. GitHub's UTC expiry is validated; local reuse ends after 60 seconds.
There is no refresh-token persistence or ambient credential fallback.

All calls use the fixed GitHub HTTPS API origin, sensitive authorization headers,
no redirects/proxy, a ten-second request timeout, 16 KiB response-header cap and
2 MiB response-body cap. One operation runs per broker instance. Errors omit
provider response text. No live App key was configured during development.

`InstallationSession` resolves the public repository ID, current PR base/head,
state, draft flag and immutable merge-base. Same-repository public PRs are supported
by this adapter; fork PRs fail closed pending their separate scoped-fetch gate.
Webhook SHAs and permissions are never analysis authority.

References: [App JWTs](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app),
[installation tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app).

### Durable automatic scheduling primitives

Migration 19 adds execution grants, analysis jobs and immutable signed intents.
It creates **no grants**. Existing inactive scope/architecture records are not
converted to publication consent. There is no browser route, CLI flag or production
helper that enables a grant in this checkpoint. Tests seed explicit isolated grants.

`AppExecutionQueue::reconcile_notification` consumes a leased PR notification and
persists the freshly resolved comparison in one transaction. A forged, expired or
consumed lease cannot acknowledge or enqueue it. `reconcile` supports bounded
future sweep callers. The optional host dispatcher now consumes inbox events;
missed-event sweeps remain pending.

Job identity binds App, installation, numeric repository, PR, selected base,
merge-base, head, scope revision, publication-consent digest, signed binary digest
and expected engine-metadata digest. The consent policy must match the saved
include/exclude paths and latest scope revision. The binary digest and metadata
digest are separate; engine metadata is not proof of binary authenticity.

Duplicates reuse the same job. Changed comparisons cancel prior pending/leased/
staged/publication-pending jobs. Closed and draft PRs cancel pending work. Claims
check current consent and sticky revocation blocks. Limits are two leased workers
globally, one per repository, 100 unfinished jobs globally, 25 per installation,
10,000 total retained job rows, a 15-minute hard lease and three total attempts.
Retries are delayed; stale leases cannot stage a report or report a failure.
FIFO admission plus tenant caps is implemented; fair round-robin dispatch and
explicit reanalysis of failed identities remain unfinished. Draft-to-ready or
reopened comparisons resume their exact cancelled job within the existing attempt
limit; previously signed bytes must pass promotion checks again.

Fresh-provider reconciliation currently holds a serialized SQLite write
transaction. This gives unambiguous ordering but is not a measured high-throughput
solution. Provider adapters must be bounded; arbitrary trait implementations are
trusted server code, never request-supplied plugins.

### Signed report intents and completed-check outbox

`stage` independently validates the complete exported `PublicBundle`, its exact
comparison, public repository, consent policy, projected scope version and engine
metadata. The source paths in the public artifact must equal the accepted policy.
Existing validators check hashes, source spans, HLD/LLD parent bindings and routes.
The caller must stage immutable bytes and verify the actual signed analyzer binary;
this API does not execute or attest an engine itself.

The separate `csb-app-receipt-v1` trust profile uses Ed25519 and an operator-trusted
public key. It signs the job, manifest digest, report identity, Focus URL, timestamp
and previous receipt digest. SQLite persists an immutable intent before promotion.
It does not claim GitHub Actions OIDC provenance. Recovery verifies the exact
receipt, job binding, current grant and revocation state.

`confirm_readback` requires a complete independently read-back bundle, not a
manifest-only success probe. Only after validation and another current-PR check
can a job enter the publication-ready outbox. The host now performs bounded HTTP readback from its configured cookie-free public
origin, including the exact catalog entry and viewer assets. No production origin
was configured or activated during this implementation.

`deliver` builds the App-owned Architecture check with the exact head and stable
job external ID. Its Details and summary links pin the report/bundle and specify
`mode=focus&design=hld`. Changed or partial evidence produces `neutral`; success
requires complete supported HLD/LLD with no detected change. Static results never
claim runtime equivalence or approve merging.

The real GitHub adapter searches only that head's Architecture checks, checks
ownership/external ID and verifies the exact payload. Incomplete or excessive
check inventories fail closed. It records a create attempt **before** sending it.
If a write's outcome is unknown, subsequent attempts only reconcile the matching
check; they never blindly create another. Missing or mismatched outcomes remain
blocked for explicit recovery. Existing foreign checks are never overwritten.
No queued/running/failure check lifecycle is mounted yet.

## PR worker, host dispatch and report hosting

### Exact scoped objects into the existing engine

`InstallationSession::fetch_pr_objects` uses only fixed GitHub Git commit/tree/blob
endpoints. It reconstructs and verifies every Git SHA-1 object before import.
Tree metadata covers the two snapshots, but blob requests are limited to authorized
regular files, excluding sensitive paths. No clone, archive, patch response,
checkout, submodule, symlink target or PR-controlled executable is run.

Limits per acquisition: 70 requests, 45 seconds between requests, 16 MiB aggregate
responses, 2,000 tree entries per snapshot, 64 unique blobs across both revisions,
256 KiB per blob, and 16 MiB serialized worker input. A final response may consume
the existing ten-second request timeout after the acquisition deadline. Exceeding
limits stops analysis; it never pretends to have analyzed a reduced scope.

**Commit support is intentionally limited.** GitHub's Git commit response does not
provide arbitrary raw commit bytes. Signed verification payloads/signatures and
conventional unsigned UTC commits can be reconstructed; the result must match the
exact requested object hash. Extra headers, other unsigned time zones/encodings,
missing verification payloads or non-reconstructable signatures fail explicitly.
This is not support for every Git commit. See GitHub's
[Git commit response](https://docs.github.com/en/rest/git/commits) and
[recursive tree bounds](https://docs.github.com/en/rest/git/trees).

`csb-app-pr-worker` receives credential-free JSON and validates object reachability,
path authorization, accepted context and hashes before creating an ephemeral bare
object store. It analyzes exact merge-base/head revisions with the existing Rust
engine, generates HLD/LLD and exports the existing validated public bundle. It
preserves selected-base provenance, uncertainty and exact source routes. Temporary
objects/cache are removed on completion. Repository code is never built or run.

`scripts/app_pr_sandbox.py` verifies an operator-pinned binary, copies it immutably,
and launches it with Linux bubblewrap/prlimit: network/PID namespace isolation,
cleared environment, no home/credential/repository mounts, 1 GiB address space,
60 CPU seconds, 90 seconds wall time, 64 MiB file-size limit and 64 file descriptors.
Input/output are capped at 16 MiB; the public bundle still has its existing 8 MiB
limit. Unsupported hosts have no unsandboxed fallback. Local policy tests do not
qualify an actual Linux host.

### Paused dispatcher and separate report listener

`AppDispatcher::tick` handles at most one notification and one analysis/delivery
item. No enabled grant means no GitHub requests. The host timer is ten seconds;
`--dispatch-mode paused` is the default. `consented` additionally requires a saved
execution grant, which onboarding still cannot create. The dispatcher reacquires
short-lived credentials and checks current PR/consent before analysis, staging,
publication and check delivery. Worker processes receive no token or signer.

The optional host execution configuration is all-or-nothing:

| Option | Required operator value |
|---|---|
| `--app-private-key-file` | Protected RSA PEM, regular file, mode 0600 or stricter |
| `--report-signing-key-file` | Protected Ed25519 PKCS#8 DER, regular file, mode 0600 or stricter |
| `--report-origin` | Approved HTTPS origin distinct from the authentication origin |
| `--report-listen` | Separate loopback address behind that origin's HTTPS proxy |
| `--pr-launcher`, `--python` | Absolute qualified launcher/interpreter paths |
| `--pr-engine`, `--pr-sha256` | Independently verified PR worker path and binary digest |
| `--dispatch-mode` | `paused` (default); `consented` is not a consent grant |

This extends the existing host options; authenticated webhook intake also requires
`--webhook-secret-file`. It is configuration documentation, not an activation
command. App IDs/client IDs remain public; secrets never belong in command strings,
repositories, browser storage, reports or logs. HTTPS proxies must not log cookies,
OAuth query parameters, request bodies or authorization headers.

Migration 20 adds a publication catalog without modifying review records or creating
grants. `AppReportStore` stages verified immutable bytes through fsync and atomic
rename, then publishes a catalog entry only for the exact persisted signed intent
and fresh provider state. Recovery reuses that identity; it cannot extend expiry or
undo withdrawal. Every report read checks current consent/revocation and file hashes.

Reports expire after seven days. At most 100 staged report directories are retained
(including orphaned staging), with at most 8 MiB each; reaching the cap refuses new
staging. Disk cleanup/backup restoration remains a gate. The router serves only the
catalog, authorized report descriptors and embedded approved workspace assets.
It uses no cookies, account routes or external assets, and sends `no-store`, CSP,
`nosniff` and `no-referrer` headers. Root opens Standard; signed PR links open Focus.

`HttpsReportReadback` forbids redirects/proxies/cookies, verifies the canonical
signed manifest, exact catalog record, all report files and viewer asset bytes,
then validates the whole bundle. Chunk reads use eight concurrent requests with a
60-second admission deadline and five-second request timeouts. Readback failure
prevents completed-check creation. The local HTTP test uses an ephemeral loopback
transport; production accepts HTTPS only.

## Run the local acceptance checks

Requires Rust 1.87, the locked dependency cache, Git and OpenSSL. OpenSSL generates
an ephemeral RSA key for one test; the key is neither saved nor printed. Runtime
signing uses `ring`, already present in the lockfile; no new crate version is added.

```sh
cargo +1.87.0 test --locked --all-features
python3.13 -m unittest discover -s tests -p 'app_*sandbox_test.py'
python3.13 -m unittest discover -s tests -p 'public*test.py'
cargo +1.87.0 test --locked --all-features --test public_report app_signed_delivery
cargo +1.87.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.87.0 clippy --locked --all-targets --no-default-features -- -D warnings
```

The integration fixture generates real Rust HLD/LLD before and after a condition/
return-value change, exports exact source routes, signs/persists the intent,
reconstructs disk readback and captures the completed check payload. It verifies
both source commit IDs and differing source content. GitHub responses/check writes
are fixtures. This is **not** live App or browser end-to-end evidence.

## Remaining implementation and activation gates

1. Add the authenticated final publication-consent transaction and operator policy
   covering origins, independently verified binary, storage/API/CPU budgets. No
   production grant writer exists. Do not enable grants via manual SQL.
2. Finish fair scheduling, periodic missed-event reconciliation, check rerequests,
   queued/running/failure checks and persistent per-job/tenant API ticket accounting.
   Current admission is FIFO with caps; delivery failures defer for ten minutes,
   but do not yet have a durable lifetime retry/request budget.
3. Qualify the Linux sandbox and signed PR worker release. The scoped adapter
   deliberately refuses unrepresentable Git commits and oversized scopes (limits
   below); arbitrary repository/commit support and fork analysis are not claimed.
4. Add expired/orphaned-file cleanup, durable revocation-ledger backup/restore,
   receipt-chain recovery procedure and retention operations. Existing exact-intent
   recovery is tested; backup restoration is not qualified. Full storage stops at
   the hard cap rather than silently deleting or expanding it.
5. Qualify a USD 0 Linux host, configure protected App/signing keys and separate
   approved HTTPS origins, then obtain activation approval. No suitable host/account
   currently exists; no service, spending, App settings or release pins changed.
6. Test a fresh repository install/select/confirm and real new-head/base/revocation
   journeys, followed by PR check → Focus → HLD → LLD → exact-source browser
   acceptance. Local mocks and HTTP readback are not live onboarding acceptance.

No activation action is required from the user at this checkpoint. Existing
Actions/Pages publication and PR #24 remain unchanged and unmerged respectively.
