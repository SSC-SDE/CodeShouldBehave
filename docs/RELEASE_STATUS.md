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
