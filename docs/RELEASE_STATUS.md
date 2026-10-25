# Linux release checkpoint

CodeShouldBehave is the public MIT-licensed project at
https://github.com/SSC-SDE/CodeShouldBehave (repository ID 1423092555).

The Linux candidate is built from exact public commit
`2917f1bb352bd5cee38d5da9be5bf79817434eab`, including Studio HLD and the MIT
owner/contributor notices. The trusted workflow is manually dispatched on main;
it does not activate automatic PR analysis or deploy a dashboard.

Build run: https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/39753808150

Attempt 1 stopped before compilation because the operator observation used a
UTC offset rather than the required Z format. Attempt 2 uses the corrected,
locally verified observation. USD 0 Actions/Packages stops remain enabled.

Historical releases retain their old repository signer identities. Their
signatures are not rewritten to match the renamed repository. New engine pins
must refer to the new build's exact commit, artifact hashes and signer workflow.

Attempt 2 passed build and attestation. Downloaded source/runtime checksums match;
`gh attestation verify` passed for the Linux archive with exact source and signer
commit constraints and hosted runners required.

Release: https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-2917f1bb352b

Updated `.codeshouldbehave/engine-pins.json` binds the new release and signer.
Merge this pin update before running the existing manual sandbox preflight.
Successful compilation and signatures do not substitute for that native startup
check or for the later PR-to-dashboard acceptance. No automatic PR workflow or
Pages deployment is enabled by this change.

The unchanged bootstrap also passed against the actual published download URLs,
verified the offline signature and ELF identities, and extracted the pinned
binaries locally. Native Linux execution remains the next manual preflight.

## Native startup verified

Public PR #1 merged at `ad26461d57b58da1b3782c3854e57672ee9e82b2`.
[Manual preflight 39756007309](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/39756007309)
passed on that exact commit: signed Ubuntu Bubblewrap provisioning, the unchanged
credential-free empty sandbox, release download/signature verification and actual
`flowdiff --version` execution inside the networkless sandbox.

This proves startup compatibility, not complete PR analysis. Repository policy,
accepted component context, controller installation, Pages/check publication and
end-to-end acceptance are still pending. No automatic PR trigger is installed.

## Refresh after PR #5 — 2026-10-24

PR #5 merged at `39b89ac088beb8823da738e9587c6ab07c315676` (provider verified).
Fresh account UI observation confirmed GitHub Free and account-wide Actions and
Packages budgets at USD 0, Usage blocked. Existing controls were not changed.
The observation remains private and expires after twelve hours.

[Build 40087980741](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/40087980741)
at that exact source passed on attempt 2. Attempt 1 stopped before compilation
on a budget proof digest mismatch from trailing-newline transport; exact-byte
normalization corrected the proof without relaxing its guard. Public contract
tests, Ubuntu 22.04/Rust 1.87 build and hosted attestation passed.

[Canary release](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-39b89ac088be)
contains both exact manifest-root corrections. Downloaded asset checksums and
hosted public provenance passed verification with exact signer/source digests.
Engine pins and the disabled setup proposal now reference this package.
The unchanged bootstrap passed against the published URLs. Native startup passed
in [preflight 40088281103](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/40088281103)
on pin commit `e90ef4b6317451a4f6a0bd55155dc561109d6bcc`: signed Ubuntu
Bubblewrap installation, credential-free/networkless sandbox and actual
`flowdiff --version` execution all passed. This proves package startup, not a
complete live PR-to-dashboard journey. Activation remains off.

## Refresh after PR #9 — 2026-10-25 UTC

PR #9 merged at `6b2734e73ee14257858394ad67b672d0487b4d2f`, provider verified.
Fresh GitHub account UI inspection confirmed GitHub Free and account-wide USD 0
Actions/Packages budgets with Usage blocked. Controls were not changed.

[Build and attestation 40221410115](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/40221410115)
passed from that exact merge. Source/runtime checksums and offline attestation
verification passed with exact source and signer digests, the trusted workflow
identity and hosted runners required.

[Signed canary release](https://github.com/SSC-SDE/CodeShouldBehave/releases/tag/canary-6b2734e73ee1)
includes the native changed-path priority, public catalog paging priority,
worker sampling race fix and bounded public projection. Both engine-pins and
active public-ci engine configuration now select it on phase-6. Fresh runtime
budget evidence and its digest are updated together; policy, scope and operation
consents are unchanged. Historical setup proposals remain historical.

Native sandbox [preflight 40221747369](https://github.com/SSC-SDE/CodeShouldBehave/actions/runs/40221747369) passed on exact pin commit
`84ab8516e63d779dcd52936b7e02e5431437b469`: verified release download,
credential-free/networkless Bubblewrap sandbox and actual native binary startup.
Live PR-to-Pages acceptance requires the
pin update merged into main: the controller intentionally rejects non-default
branch runs. No branch result is claimed as a successful live deployment.

### Runtime observation correction after live attempt

The new release and sandbox preflight remain valid. The runtime observation
refresh in the pin update was incorrect for an already activated signed ledger:
it changed its immutable budget identity. Live controller `40222027727` refused
before analysis. PR #11 restores the original runtime budget identity while
retaining the new engine pins. The fresh release-build proof stays separate.
An existing attested activation may outlive the observation expiry with unchanged
policy and maintained hard-zero controls; initial activation still requires fresh
proof. This does not assert current billing controls from historical evidence.
