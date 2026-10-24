# Repository setup proposal — activation OFF

Repository: SSC-SDE/CodeShouldBehave, numeric ID 1423092555.
Scanned public commit: `ad26461d57b58da1b3782c3854e57672ee9e82b2`.

## Review scope

Include `Cargo.toml` and `src`; exclude `src/fonts`. No related repositories,
external services, AI destinations, scripts, tests, fixtures or Git history.
Compare the PR merge base with its head; maximum 100 LLD details. Unsupported
non-Rust files within src must remain explicit coverage gaps.

The native local scan read pinned Git objects only. It did not run repository
code. `architecture-proposal.json` is the unaccepted scan/detection/inspection
response, not an accepted runtime context. No confirm or activate action ran.

## Proposed architecture

One detected package-level component covers Cargo.toml and src. Its native role
is unclassified; suggested reviewer label: **CodeShouldBehave Rust engine and
optional local service**. This label is a proposal, not a native finding.
Cargo.toml declares the flowdiff package, optional server feature, SQLite,
Tree-sitter, Axum and Tokio dependencies. These are dependency declarations,
not proof of deployed services. No deployment provider is accepted.

The detector samples only eight files per manifest root. Internal import signals
and possible integrations require review; none grant more context access. This
initial package-level map is not a verified multi-service architecture.

## Files and safety state

- public-policy.json: proposed paths with publication_consent=false.
- public-ci.json: exact repository and release pins; all operations=false.
- architecture-proposal.json: pending component/provider/integration decisions.
- workflows/: verbatim pinned controller/nudge drafts, outside GitHub's active
  workflow directory. bootstrap.py is a verbatim draft too.

The runtime accepted-context.json intentionally does not exist. Its config digest
remains unresolved until owner decisions are recorded through native confirmation.
The budget proof is unavailable, not a fabricated or reused expired observation.
The proposed Pages path is /CodeShouldBehave/; actual Pages configuration must be
verified before activation. The installer correctly refuses this disabled draft.

## Owner review

1. Approve or narrow the paths above.
2. Confirm the single package boundary, or request a different component map.
3. Review unresolved integration/provider questions in the native proposal.
4. Only after that review: generate accepted context, refresh budget proof and
   review the final install diff. Installing or enabling workflows and publishing
   Pages/checks require separate activation approval.

Merging this proposal does not activate anything. Existing manual release and
sandbox workflows are unchanged.

Detected review questions: Rusqlite (database), ModelAdapter (interface contract),
and two internal-module-import candidates. Proposed treatment: retain as local
code/dependency evidence only, with no database access, model calls or additional
repository access. All four remain pending until you confirm that treatment.
The inventory completed with zero failed reads and eight unsupported entries;
unsupported entries must not yield a clean no-change verdict.

Native response caveat: the API includes an `accepted_context` placeholder, but
`usable=false`, confirmation=null and the accepted scope fingerprint is empty.
Its generic scan-gap wording does not mean the owner accepted any gaps. The
session is awaiting_confirmation. Do not install this placeholder as runtime
context. Validation checks these fields explicitly.

Review PR: https://github.com/SSC-SDE/CodeShouldBehave/pull/2

## Owner boundary approval recorded — 2026-10-24

PR #2 merged at d88752c9f8f8f6c50459e9bd32dfc1194e214e39. The owner approved
the scope and single component boundary. Native component decision is confirmed;
all four integrations are deferred, not authorized for added context. Earlier
statements above about pending component decisions describe the original proposal.

A validator mismatch prevented confirming the detector's explicit Cargo.toml
root. The correction accepts only an indexed manifest's exact authorized path
or its already supported manifest directory; unknown paths remain rejected.
22 onboarding tests and full-feature strict Clippy pass. The native decision
round trip now succeeds with the same paths and source commit.

Final confirmation has NOT run: state=awaiting_confirmation, confirmation=null,
review_defaults_active=false. The API now reports a usable projected component
map; that is not a confirmation or authorization to install it. No runtime
accepted-context.json exists, publication_consent=false and all operations=false.

Remaining owner decision: accept incomplete coverage for eight unsupported paths
and unknown hosting, with the four integrations deferred. Then generate the
confirmed context with activate_defaults=false. Workflow installation and live
activation remain separate. The engine correction needs merge and release refresh
before relying on it in the hosted onboarding path.
