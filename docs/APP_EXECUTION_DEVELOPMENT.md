# App execution and signed delivery: development checkpoint

November 6, 2026. Implemented on `phase-6` in
[PR #32](https://github.com/SSC-SDE/CodeShouldBehave/pull/32).
**Activation remains off. This is a tested library integration, not a running
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
future sweep callers. These functions are not mounted in a host polling loop yet.

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
can a job enter the publication-ready outbox. The future publisher is responsible
for actually fetching those bytes from the approved cookie-free public origin.
No HTTP publisher or public report handler is mounted by this implementation.

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

## Run the local acceptance checks

Requires Rust 1.87, the locked dependency cache, Git and OpenSSL. OpenSSL generates
an ephemeral RSA key for one test; the key is neither saved nor printed. Runtime
signing uses `ring`, already present in the lockfile; no new crate version is added.

```sh
cargo +1.87.0 test --locked --all-features github_app_setup
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

1. Implement a bounded installation-token scoped Git-object fetcher and full PR
   analysis sandbox adapter. The existing isolated onboarding preview worker is
   not a PR-analysis worker. Feed accepted architecture and explicit export consent
   to the existing HLD/LLD engine without exposing credentials to it.
2. Add the distinct authenticated final consent transaction and operator policy
   (origins, independently verified binary, storage/API/CPU budgets). Keep it
   disabled until deployment/activation approval. Do not enable via manual SQL.
3. Compose the durable inbox/queue/worker/outbox loop with bounded sweeps, missed
   event repair, check rerequests, fair dispatch, API ticket accounting and
   queued/running/failure status. Keep failed-job reanalysis explicit.
4. Add immutable filesystem publication, atomic catalog, actual HTTP readback,
   cookie-free report serving, withdrawal/expiry, receipt-chain backup/restore and
   the exact interrupted-publication recovery procedure. A supplied in-memory
   readback bundle is not proof that a public URL is reachable.
5. Qualify a USD 0 Linux host and the signed worker, configure protected App/signing
   keys and approved HTTPS origins, then obtain activation approval. No suitable
   host/account currently exists; no service or spending was provisioned.
6. Run fresh-repository install/select/confirm, new-head/base/fork/revocation tests
   and a real PR-check → Focus → HLD → LLD → exact-source browser test before
   claiming the hosted installation journey complete.

The browser wizard, loopback host, existing deployment and public demo retain their
previous behavior. No new installation or activation commands are needed from the
user at this checkpoint.
