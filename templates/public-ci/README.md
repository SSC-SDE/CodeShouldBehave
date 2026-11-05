# Public repository CI templates

Files in this template directory are inert until installed. The upstream
repository now has active copies under `.github/workflows/` and a verified live
PR demo. Copying templates into another repository does not supply its consent,
confirmed architecture, valid budget evidence or Pages configuration. Nothing
in this directory installs a GitHub App or changes repository visibility.
`config/public-ci.example.json` is deliberately disabled and contains unresolved
trust pins. Runtime validation rejects it.

After reviewed installation and activation, this profile implements the following journey:

1. A source-free `pull_request` nudge wakes a default-branch controller. Fork
   workflow edits, checkout contents and artifacts are never consumed. A ten-minute
   scheduled/dispatch reconciliation remains the best-effort fallback.
2. The controller independently reads current public repository identity and PR
   base/head, restores attested durable state, and admits at most three comparisons
   with two concurrent workers under fixed application REST tickets. It uses base-owned policy and accepted architecture
   context only. PR changes to either file do not change that PR's authority.
3. The worker fetches only public Git objects into an empty bare repository. No
   checkout, hooks, submodules, builds, tests, package scripts, browser code or PR
   workflows execute. Linux Bubblewrap isolates the pinned Rust analyzer with no
   network, token, home directory or writable policy/context.
4. An independent publisher checks exact request identity, current PR eligibility,
   archive bounds, canonical policy digest and the Rust public-schema verifier. It
   restores the last published site using its independently attested manifest hash.
   A changed policy withdraws old-policy reports before another site is assembled.
5. Before the Pages API is called, an attested intent is stored in a generic
   Deployment receipt. After the actual Pages deployment succeeds, the live site
   is read back and a separate promotion receipt is committed. An orphaned intent
   stops later promotion until operator recovery; it is never treated as success.
6. GitHub's controller workflow supplies queued/running status. Only after the
   final receipt commits, the exact old head's advisory Architecture check links
   its immutable report. A crash cannot create an unrecorded custom check.
   Partial/unknown/deferred coverage is `neutral`, never a supported no-change
   result. GitHub can treat neutral required checks as passing; this check does not
   approve merging and should not be represented as a correctness gate.

The controller uses artifacts only from its own trusted run. It never downloads
an artifact from `github.event.workflow_run.id`. The uploader/downloader actions
are pinned by commit, with mismatch failure. State receipts retain their Sigstore
bundle independently of short-lived Actions artifacts. A Deployment record by
itself is not authority: `gh attestation verify` must cryptographically validate
the exact canonical subject, public repository numeric ID, default ref, pinned
workflow source, hosted runner and certificate-bound run/attempt.

## Prerequisites and installation boundary

The own-repository live demo is accepted: [PR #24 Architecture check](https://github.com/SSC-SDE/CodeShouldBehave/runs/127446113035)
opens Focus with HLD, LLD and exact source diffs. Signed recovery and automatic
controller [42083656278](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/42083656278)
passed; its complete application REST evidence used 71 of 100 calls. This does
not establish arbitrary-repository, fork or 100+ PR/day capacity acceptance.

The deployed [canary-c46c28a509b6](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-c46c28a509b6)
passed signed bootstrap and Linux sandbox startup. Current exact engine pins
are in `.codeshouldbehave/engine-pins.json`; use the [release record](../../docs/RELEASE_STATUS.md)
for evidence. New repositories still need their own review/activation below.

The [planned GitHub App flow](../../docs/GITHUB_APP_ONBOARDING_PLAN.md) eliminates
customer workflow and Pages setup using an operator-hosted service. It is not
implemented. Installing the registered App does not perform this CI setup.

### One-time setup sequence for this Actions profile

1. Verify the target is public and the owner authorizes public report excerpts.
   Resolve an existing Pages-site conflict before configuring a new destination.
2. Choose include/exclude paths and baseline policy. Scan an exact commit through
   native onboarding; confirm components and explicitly accept known gaps. Export
   a fresh confirmed context with authorized export permission.
3. Verify the signed Linux package using the trusted bootstrap. Retain its exact
   source, package, binary and attestation digests; do not build the PR's code.
4. Record fresh owner-observed USD 0 Actions and Packages hard stops. Fill the
   new repository's numeric ID, default branch, receipt signer, Pages origin/path,
   scope/context/budget digests and operation switches in a reviewed config.
5. Run the create-only installer below in a local branch. Review all files together.
   The installer creates four files; policy, context and budget evidence must
   already exist. Never reuse this project's signed deployment ledger.
6. With separate activation approval, configure Actions/Pages and restrict the
   github-pages environment to the default branch; merge the installation PR.
   Scheduled processing can start immediately after merge. No customer App key
   or release-build workflow is needed; the controller uses GITHUB_TOKEN.
7. Dispatch the controller once, then open a scoped Rust-change PR. Verify the
   exact-head Architecture check, summary link, Focus and HLD/LLD/source journey.
   Missing evidence is a failure/incomplete result, never a clean no-change result.

Prepare the following base-owned files in a **public canary** only after the
owner authorizes that action:

- `.codeshouldbehave/public-policy.json`: strict `csb-public-policy-v1`, immutable
  repository ID, `public:true`, `publication_consent:true`, explicit include/exclude,
  `baseline_mode` (`exact` or `merge_base`) and `max_details` (0 through 100).
- `.codeshouldbehave/accepted-context.json`: the accepted architecture context
  produced by the existing onboarding tool, with current public scope. Analysis
  never invents accepted HLD components or confidential related-repository context.
- A recent owner-observed hard-zero Actions **and Packages** proof, kept at the
  config's base-owned evidence path and pinned by its raw SHA256.
- A copy of the example config with exact policy/context hashes, real Linux
  package/source/binary/attestation pins, numeric repository ID and explicit
  operation switches. Pages is the repository's project site (`/REPOSITORY/`).

The engine config pins `package_url`, `package_sha256`, `package_signer_workflow`,
`package_attestation_url` and `package_attestation_sha256`. Bootstrap downloads
only fixed public GitHub release assets, verifies both byte hashes and the
Sigstore subject before extracting a bounded link-free archive. It does not
execute code from that archive until verification succeeds. `gh --bundle` avoids
implicit GitHub attestation discovery; bounded trust-root verification can still
contact the Sigstore trust infrastructure.

After repository-specific prerequisites are reviewed, the create-only local preparer is:

```sh
python3 /PATH/TO/VERIFIED/PACKAGE/templates/public-ci/install.py \
  --config /PATH/TO/REVIEWED-CONFIG.json \
  --repository-root /PATH/TO/PUBLIC-CANARY \
  --package-root /PATH/TO/VERIFIED/PACKAGE
```

It writes only two workflow files, the base-owned bootstrap and the config, and
prints their paths. It refuses existing files, symlinks, placeholders, missing
consent, stale initial cost evidence and mismatched Linux binaries. It never
pushes or merges them. `engine-build.yml` is the upstream engine-release template,
not a consumer analyzer workflow and is not copied by this preparer.

## Cost, storage and recovery contract

Activation proof must be no more than 24 hours old. Its unchanged budget policy
is then bound into attested state for unattended runs. Later account controls are
**last observed, not live API verified**. The owner must keep enforced USD 0
Actions and Packages hard stops enabled. A repository token cannot observe all
account-wide usage; unknown usage is not assumed to be empty. No larger runner,
paid tier, model, paid cache or paid-host fallback exists.

Repository-wide nonexpired Actions artifacts plus upload reservations are capped
at 172 MiB. Public bundles are at most 8 MiB expanded / 4 MiB compressed; handoffs
are at most 5 MiB each under 32 MiB total. Static sites are at most 64 MiB, and a
Pages archive at most 70 MiB. Before another snapshot, cleanup protects the exact
attested last-good artifact ID; after promotion it can retain that snapshot and
one prior artifact. Only controller-owned, provider-verified artifacts are deleted.
Other repository artifacts still count against the guard. Consumed handoffs and
control artifacts are removed only after final state is committed; failed data
expires after one day. No cache is enabled.

The two newest verified receipt checkpoints remain in the Deployment ledger.
Cleanup verifies provenance before deletion; an unverifiable same-namespace
record, discontinuous state chain, malformed/truncated history, missing live site,
manifest mismatch or unresolved promotion intent requires explicit recovery.
The helper never resets such state to an empty successful dashboard. Receipt
recovery, rollback after withdrawal, uninstall/disable and release compromise
handling require the documented canary acceptance before general use.

Check delivery is kept in a bounded durable repair ledger. A missed PATCH is
retried on a later controller run and acknowledged only by a later signed state.
Repeated acknowledgements are idempotent; no update can attach an old report to a
new head. Two failed worker attempts become a visible terminal failure for that
comparison; a new head/policy/context/engine creates a new comparison.

## Local validation and CLI boundaries

```sh
PYTHONPYCACHEPREFIX=target/public-ci-pycache \
  python3 -m unittest discover -s tests -p public_ci_test.py -v
python3 scripts/public_ci.py validate-config --config config/public-ci.example.json
python3 scripts/public_ci.py validate-bundle --bundle /PATH/TO/PUBLIC-BUNDLE
```

`plan-offline`, `pack-handoff`, `unpack-handoff` and `prepare-matrix` are local
operations. `github-plan`, `analyze-request`, `upload-preflight`, `stage-publication`
and `finalize-publication` make bounded reads when given activated real config.
`analyze-request --offline-mirror` is explicitly local and still requires the
credential-free Linux sandbox. No insecure macOS fallback is supplied.

`repair-checks`, `commit-receipt`, `deploy-pages` and `cleanup`
require **both** activated base-owned operation switches and the explicit
`--allow-remote-writes` flag. `queue-checks` is not used by the template and
refuses any request absent from a committed durable receipt. Their presence in this template does not authorize
running them against the private application repository.

## Enforced application REST budget

One trusted controller pass has disjoint durable tickets whose allocations sum
exactly to 100:

| Stage | Maximum jobs | REST calls per job | Total reservation |
| --- | ---: | ---: | ---: |
| `plan` | 1 | 25 | 25 |
| `analyze-0`, `analyze-1`, `analyze-2` | 3 | 4 | 12 |
| `publish` | 1 | 63 | 63 |

The plan assigns unique request IDs to at most three analysis slots. Runtime
admission also checks actual free repository artifact bytes before reserving the
**whole** handoff batch and one MiB of control data. It does not let concurrent
workers each spend the same free bytes. Each bounded handoff is uploaded once by
its assigned job; no other controller pass overlaps. Unrelated repository
uploaders are outside this coordination, still count in observed inventory, and
can cause provider quota refusal. No account-wide usage or empty-capacity claim
is made.

Every `GitHub.api` request durably reserves its ticket **before** transport,
including failed provider calls. A new CLI/provider instance reuses that job's
fixed ledger and cannot reset its allowance. Live Actions transport without a
ticket refuses. The fixed default-branch workflow has exactly one plan/publisher
and one job per unique assigned analysis slot. The run/attempt, config, workflow,
role and request identities are checked; reruns use a new pass identity. Missing,
corrupt or reset ledgers fail closed.

One private Unix-socket publisher process remains in the trusted job across the
two attestation action steps. It reuses only immutable workflow-at-commit,
run-attempt and exact verified-proof bytes. Repo/PR, default-ref and Deployment
ledger checks remain fresh. No network listener or browser API is exposed.

The final aggregate requires the plan, publisher and every assigned worker's
counter proof exactly once. A failed worker that cannot return a snapshot makes
that pass's evidence explicitly incomplete; it is never reported as zero calls.
Local fake-provider integration exercised a cold three-worker pass (73 calls)
and a second pass with retained receipts/check delivery (89 calls). These synthetic counts do not establish live multi-worker capacity. The separate
verified own-repository run above used 71/100 application REST calls.

`begin-promotion`, `prepare-matrix` and publisher IPC clients perform no provider
REST. Git public fetch, Pages file readback, five maximum release bootstrap
verifications and up to two `actions/attest` invocations are separately bounded
activities. The bootstrap uses the fixed Sigstore bundle; trust-root/SDK/platform
internal requests are not observable application transports and are not falsely
included in the proved 100 application REST calls.

The analyzer job uses the explicit `ubuntu-22.04` standard hosted image and
installs Bubblewrap from its signed distribution repositories with
`apt-get update -qq` and `apt-get install -y --no-install-recommends bubblewrap`.
Only this package setup uses `sudo`; the analyzer and exporter run without it.
Plan and publisher jobs remain on `ubuntu-24.04`.

The trusted Linux engine build job also uses `ubuntu-22.04` so its GNU libc
requirements match the analyzer image. Its separate attestation job remains on
`ubuntu-24.04`; that job handles the built archives without executing the native
binaries. Do not substitute an engine built on a newer distribution merely
because its archive and signature verify.

The 22.04 builder selects Python 3.12 with the immutable official setup action
recorded in `action-pins.json`. Its default system Python is not assumed to meet
the source packager's Python 3.11+ requirement. It runs the public release
contract suite on the actual builder before packaging. No pip cache is enabled.

In [native compatibility probe run 39018952784](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/39018952784)
(job 117356721397), release provenance verification and the strict empty sandbox
passed on 22.04, but the engine built on 24.04 failed to start: its runtime needed
`GLIBC_2.39`, which the analyzer image did not provide. The corrected build image
requires a new Linux build, release digest and attestation verification, followed
by actual native execution inside the unchanged sandbox. This historical incompatibility was resolved. The currently deployed release
listed above subsequently passed native startup and the live review journey. Do not modify the verified archive or install a
different libc to bypass this failure.

The real hosted [sandbox probe run 39016540661](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/39016540661)
passed the unchanged strict Bubblewrap command on `ubuntu-22.04`
(job 117349713231), while `ubuntu-24.04` failed it (job 117349713122) after the
package installation. This establishes the measured compatible sandbox profile,
not a completed native worker or public analysis result. The exact cause of the
24.04 namespace refusal remains unconfirmed; no AppArmor or kernel setting was
changed to bypass it.

The template still requires a usable `/usr/bin/bwrap` and tests an actual empty
`--unshare-all --clearenv` sandbox before invoking the engine. Missing packages,
package-manager failures, blocked user namespaces or AppArmor denial stop the
job. Do not disable AppArmor, relax kernel isolation or run reviewed code outside
the sandbox to make this check pass. Each new deployment must retain native worker, provenance and public canary
acceptance; the upstream own-repository result does not waive those checks.

## Reviewer entry and current activation state

Published check links explicitly select Focus and pin the exact report/bundle.
Direct dashboard entry opens Standard. Complete supported no-change analysis is
green; detected changes or incomplete coverage require review. No-change is not
a runtime equivalence guarantee. Studio is the default Focus HLD appearance.

The upstream project has active nudge/controller workflows and a verified hosted
review journey. This directory remains a template for other repositories.
Their installation, scope confirmation and end-to-end acceptance are separate.
Checks in this profile are owned by GitHub Actions; App-owned automatic checks
remain planned. A neutral review-needed result is advisory, not merge approval.
