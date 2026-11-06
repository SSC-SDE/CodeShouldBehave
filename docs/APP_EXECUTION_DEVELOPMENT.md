# App execution and signed delivery: development checkpoint

November 7, 2026. Local continuation on `phase-6` for
[PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32).
**Activation remains off. This is a locally tested host integration, not a running
hosted App service. PR #24 and its existing Actions/Pages publisher are unchanged.**

The continuation started from inspected PR head
`6efe23e26118bcd7e5351b3946b7f7fdd49f9faf`. Implementation is committed locally at
`14012713ee3a505dd1f1d45bb7b33a049126183c`; runtime distribution follows at
`6edfc70c1129b4f23b6c384fad3d91e571a2cd3b`, with final progress-quota fairness
correction `f041715cb9a2498b603fcac412d026dedd511cf9`. None has been pushed.
Discovery, operator activation validation, check lifecycle delivery and offline
recovery tools have local evidence; no real installation was enabled. See
[implementation progress](implementation-progress.md).

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
converted to publication consent. Final authenticated consent now has separate
preview/confirm/status/revoke routes; it never creates or enables an execution
grant. A separate signed operator transaction can enable an exact grant after
independent engine/host qualification and fresh provider checks; it is never
mounted on the browser router.

`AppExecutionQueue::reconcile_notification` consumes a leased PR notification and
persists the freshly resolved comparison in one transaction. A forged, expired or
consumed lease cannot acknowledge or enqueue it. `reconcile` supports
caller-owned bounded sweeps. The dispatcher consumes inbox events, repairs tracked
comparisons and traverses initial open-PR inventories independently of webhooks.

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
Durable least-recently-served installation turns now order notification, analysis
and delivery admission. Analysis and delivery alternate when both have work.
Exhausted identities require explicit operator reconciliation of an existing check
or terminal abandonment; there is no attempt/budget reset or blind replacement POST.
Ordinary draft-to-ready/reopened jobs resume only within their existing limits.

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
The same immutable check now transitions through queued, in-progress and honest
failure states. Only verified full report readback permits the completed result
and Focus link. Progress failure cannot block deterministic analysis. Each status
has three delayed attempts; all requests still consume the original lifetime and
daily budgets. A lost queued POST response is reconciled using its persisted
initial payload, even if the desired state has since changed. PATCH first checks
the exact App, head and immutable job identity.
Quota/credential refusal before the provider pass also persists a ten-minute
backoff, allowing other pending checks to progress without refunding any tickets.
Status delivery itself can be deferred when credentials, provider access or
remaining reservations are unavailable. A previously queued/running GitHub check
is not proof of completion; operator recovery must preserve the same budgets.

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
execution grant, which onboarding still cannot create. Final consent records an
admin decision without granting runtime access. The dispatcher reacquires
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

### Explicit operator activation

`csb-app-operator` provides `preview`, `approve`, `commit` and `verify-evidence`.
Protected operator configuration pins three distinct Ed25519 public roots:
operator approval, independently verified signed-engine evidence and Linux host
qualification. Qualification binds origins, host target/identity, engine/metadata,
launcher and interpreter digests, retained provenance and expiry. Fixture
signatures verify this protocol; they do not qualify a real host or release.

Preview reads the latest unrevoked admin consent, saved scope and accepted
architecture, then checks current numeric repository/installation/admin access.
The full unsigned document binds expected consent/activation revisions and expires
within ten minutes. The operator reviews and signs it; commit independently
rechecks all bindings in the atomic grant/audit transaction. Exact retries are
idempotent; stale, expired, consumed or revoked approvals cannot overwrite newer
authority. Grant checks use the actual canonical consent-document digest, not a
synthetic scope/policy hash. Report staging also checks the consented origin.

`verify-evidence` and `approve` are offline: neither loads App credentials, opens
the database nor contacts GitHub. Operator PKCS#8 v1/v2 DER keys must derive the
independently configured public root. Live preview/commit reserve five
nonrefundable tickets before the fresh GitHub checks. Preview does not mutate
activation state, although its live request reservations are intentionally durable.
Run `csb-app-operator --help` for the exact command arguments. No automatic mode,
browser activation route or manual-SQL activation procedure is provided.

Operator `--config` is strict JSON in a protected regular absolute file:

| Fields | Contract |
| --- | --- |
| `app_id`, `client_id` | Actual positive numeric App ID and registered public client identifier |
| `state`, `artifacts` | Absolute existing App database and its artifact root |
| `app_private_key_file` | Protected RSA key file for live preview/commit; may be null for offline commands |
| `runtime` | `ActivationRuntimePins`: authentication/report HTTPS origins, engine/metadata/launcher/interpreter SHA-256 digests, host identity digest and qualified Linux target |
| `operator_public_key`, `engine_verifier_public_key`, `host_qualifier_public_key` | Distinct independently provisioned raw 32-byte Ed25519 roots encoded base64url without padding |
| `engine_file`, `launcher_file`, `interpreter_file` | Absolute trusted regular files, not group/world-writable; live commands verify bytes against runtime pins |
| `engine_metadata_file` | Protected JSON whose canonical hash matches the metadata pin |

Engine/host evidence and approval are `SignedActivationDocument<T>` JSON: exact
typed document plus Ed25519 signature over canonical JSON, base64url without
padding. Public types are reexported from `flowdiff::github_app_setup`; see
`ActivationEngineEvidence`, `ActivationHostEvidence`, `ActivationDocument` and
`ActivationSelection`. Configuration/evidence JSON is
bounded to 64 KiB. No sample credentials or self-issued qualification are supplied.

The setup consent status distinguishes `operator_authorized` from runtime
availability (`unknown`). Signing out of setup is not consent revocation.

### Host runtime and report listener

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
staging. Bounded maintenance now removes expired/withdrawn known report directories;
old unreferenced staging is collected only after a day with no signed intent or
live analysis lease. Trusted backup restoration remains an operator gate. The router serves only the
catalog, authorized report descriptors and embedded approved workspace assets.
It uses no cookies, account routes or external assets, and sends `no-store`, CSP,
`nosniff` and `no-referrer` headers. Root opens Standard; signed PR links open Focus.

`HttpsReportReadback` forbids redirects/proxies/cookies, verifies the canonical
signed manifest, exact catalog record, all report files and viewer asset bytes,
then validates the whole bundle. Chunk reads use eight concurrent requests with a
60-second admission deadline and five-second request timeouts. Readback failure
prevents completed-check creation. The local HTTP test uses an ephemeral loopback
transport; production accepts HTTPS only.

## Final consent and durable safeguards

### Browser consent, with checks still off

After architecture acceptance, **Publication consent** displays the exact scope,
source revision, public report origin, engine digest, source visibility, seven-day
retention and request limits. Three unchecked permissions cover automatic scoped
analysis, public source excerpts and advisory checks. All three are required.

`POST /v1/github-app/publication/preview` derives the document from saved scope,
accepted architecture and trusted operator settings. Confirm binds its digest,
current numeric actor, expected consent revision and ten-minute preview expiry.
Fresh GitHub admin/public-installation access and current inventory are checked;
a final SQLite transaction fences session, scope and consent revisions. Stale,
expired, tampered, cross-origin or unauthenticated confirmations fail. The browser
cannot supply a different destination, engine or publication policy.

`publication/confirm` appends an immutable record in migration 21, with **activation
off**. It also disables any previous grant, cancels unfinished jobs and withdraws
previous reports so consent replacement cannot leave old publication authority
running. It never creates an enabled grant. `publication/status` restores current
saved consent after login/reload. Changed scope or operator pins require new consent.
`publication/revoke` atomically revokes consent, disables grants, cancels unfinished
work and withdraws reports. Re-consent creates a new revision; it cannot un-revoke
old records or restore old report URLs. Browser storage contains none of this state.

These routes require the existing secure session and exact Origin/CSRF boundary.
Consent preview is available only when the host has explicit operator report/pin
settings. No live operator configuration or consent was created during development.

### Request reservations and scheduling

Before provider IO, `AppRequestBudget` reserves durable, non-refundable tickets:

| Operation/boundary | Reserved request limit |
|---|---:|
| One notification reconciliation or tracked-PR repair | 10 |
| One analysis pass, including acquisition and stage recheck | 80 |
| One report-delivery pass | 32 |
| Lifetime immutable comparison/job | 384 |
| Lifetime notification/repair-window identity | 50 |
| Installation per UTC day | 2,000 |
| App per UTC day | 10,000 |

The broker decrements an in-memory ticket before **every** actual HTTP request;
without an admitted pass it cannot issue a request. Reservations commit before IO
and include unused tickets, errors and crashes. A restart does not refund them.
All three durable bucket updates are atomic. Accounting rows are capped at 20,000;
capacity exhaustion stops work rather than deleting history to reset limits.
These are App REST safeguards, not a claim that cloud hosting/billing is configured.
OAuth/setup requests retain their earlier bounded adapters; these dispatch limits
do not replace reverse-proxy abuse/rate limits required during host qualification.

Persistent installation turns prevent a busy tenant from monopolizing notification,
analysis or delivery admission. A persistent phase toggle alternates analysis and
delivery. A delivery pass claims a 15-minute slot before IO; at most three passes
are allowed. Failed readback delays the next pass by ten minutes. Exhaustion remains
failed and does not silently reset the unknown-create fence or lifetime budget.

An idle pass repairs one **already tracked** PR's fresh head/base state, with a
persisted ten-minute checkpoint before provider requests. Duplicate input does not
create another job/check; a changed comparison is queued normally. An App-owned
check rerequest resolves only through the locally recorded check ID, installation
and repository, then reconciles fresh provider state. Unknown/foreign checks are
ignored without token issuance. Neither mechanism resets failed job attempts.
Migration 22 adds durable initial PR discovery: one inventory page or one pending
PR per tick, alongside ordinary analysis/delivery. Pages contain at most 20 PRs;
page/PR passes reserve four/five REST tickets including credential issuance.
Leases last 120 seconds, failures defer 600 seconds, completed walks repeat after
one hour. At most 1,000 pending hints per repository and 10,000 globally are kept.
Capacity/crash/failure cannot acknowledge an unfinished number or advance a failed
page. Turns rotate installations/repositories and pending failures. Already tracked
exact-authority jobs stay with ordinary repair; drafts remain discoverable.
Only an empty page ends a walk. REST pagination is not snapshot completeness;
moving inventories require repeated walks. Fork/deleted-head inventories do not
expand the existing same-repository worker support.

### Recovery and report retention

Opening a report store verifies this App's complete retained Ed25519 receipt chain:
raw receipt hashes, signature key, predecessor, job ID and App binding. Cross-App
chains are separate. Invalid chains stop the store; directory contents never become
publication authority. Repeated exact-intent promotion still requires fresh PR and
consent checks and cannot change catalog identity, expiry or withdrawal.

Dispatch maintenance validates the chain and marks expired or no-longer-authorized
known reports withdrawn before deleting at most two report directories per tick.
Deletion refuses symlinks/special files and enforces the existing file/byte bounds.
Interrupted deletion remains withdrawn on restart. Immutable catalog tombstones
and receipt/accounting history are retained. Paused dispatch does not perform disk
maintenance, but every report read still enforces expiry and current authorization.
Unreferenced staging older than a day is eligible for the same bounded cleanup,
after checking that no signed intent or active analysis lease needs it. New staging
and recoverable signed reports are retained under the 100-directory cap. Malformed
or unsafe paths stop cleanup for operator investigation.

`csb-app-recovery` creates signed exact offline snapshots and verifies/copies them
into a **new** destination only. It reuses consistent SQLite/artifact backup and
binds database/manifest hashes, receipt chain, all App tables, consent/revocations,
withdrawals, activation records, attempts and nonrefundable reservations.
Verification requires the expected App/key and an independently retained anchor
SHA-256; a signature or valid receipt prefix alone is insufficient.

Stop all writers through capture, verification and copy. The trusted external
anchor must cover the **latest committed safety state**, not merely the newest
available periodic backup. After any subsequent mutation that older anchor is not
restore authority. Continuous external freshness/fencing and operator storage
qualification remain unresolved deployment gates; this tooling cannot prove them.
Report directories need separate immutable retention and normal validation.

`csb-app-host` accepts a one-shot verified import into a nonexistent `--state`
using `--restore-from`, `--recovery-anchor-file`, `--recovery-anchor-sha256`,
`--recovery-receipt-key-file` (protected raw 32-byte public key) and
`--recovery-safety latest-committed-quiesced`. It verifies before migrations or
serving, refuses `consented` dispatch, disables all imported execution grants,
and preserves budgets/attempts/tombstones. New operator approval is required to
re-enable execution. This safety mutation itself needs a new external anchor
before a later restore. Import is not live recovery qualification.

Privileged `recover_job` decisions can abandon an exhausted/unknown job or
reconcile an exact existing App-owned progress/completed check. Completed
reconciliation additionally requires signed intent, complete validated readback
and an unwithdrawn/unexpired catalog entry. No recovery decision creates or
updates an unknown check, refunds tickets or restarts attempts. Abandonment is
terminal. Database triggers retain decisions and prevent counter/create-fence or
withdrawal-history resets. Provider-backed recovery callers must reserve the same
lifetime/daily request budget before issuing broker requests.

Before upgrading existing state, stop all writers and preserve an approved
consistent database/artifact backup with its current external safety authority.
Opening the new binaries applies additive migrations 22–25; older binaries reject
that newer schema. The new recovery CLI requires the recovery schema and is not
a pre-upgrade backup command for older databases. Rollback needs a compatible,
verified database/artifact restore, not just an older executable, and may never
refund later reservations or undo revocations. No live upgrade was performed.

### Offline runtime distribution

The clean-source runtime package now includes `csb-app-preview`,
`csb-app-pr-worker`, `csb-app-host`, `csb-app-operator` and `csb-app-recovery`,
alongside the existing review/export/onboarding binaries. A local isolated
`aarch64-apple-darwin` build and clean extraction exercised the recovery command,
operator help and invalid-worker-input rejection. It is not a signed Linux
release or production sandbox qualification; existing release pins are unchanged.

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

1. Review the local commits and obtain required remote PR/head CI evidence.
   Local tests and packaging do not publish these changes or authorize a merge.
2. Qualify a USD 0 Linux host, freshly signed sandbox workers, HTTPS proxies,
   protected key stores and independent engine/host authorities. No account,
   resource, App setting, deployment, release or pin was configured here.
3. Qualify external backup authority/fencing against the latest committed safety
   state; periodic old snapshots cannot restore revoked consent or spent budgets.
   Local exact-snapshot tools do not solve unavailable external freshness evidence.
4. Preserve scoped acquisition limits: 64 authorized blobs, exactly reconstructable
   commits and no fork worker adapter. Larger/unsupported inputs fail explicitly.
5. Run live fresh-repository install/select/consent/explicit operator approval and
   PR check → Focus → HLD → LLD → exact-source browser acceptance, including cost,
   permission, revocation, restart and recovery qualification.

No activation action is required from the user at this checkpoint. Existing
Actions/Pages publication and open PR #24 remain unchanged.
