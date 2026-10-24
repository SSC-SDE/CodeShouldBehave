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
Native startup verification of these new pins is pending. Activation remains off.
