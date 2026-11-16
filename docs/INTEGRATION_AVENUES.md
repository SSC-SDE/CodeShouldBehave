# Integration avenues beyond the GitHub App

Status: **proposal, 2026-11-16. Nothing here is shipped or activated.** The
GitHub App is one delivery mode, not a requirement. This document compares
provider-agnostic ways to put the analyzer in front of more people without
turning the project into a multi-tenant hosted service.

## What can be reused

- The Rust engine already produces deterministic HLD/LLD data from exact Git
  commits and never executes reviewed code.
- The public exporter already turns that data into an immutable static bundle.
- The static workspace already renders HLD → component → LLD → exact source.
- Existing CI templates and the local site builder already prove the static
  publication path.

The missing layer is a stable, supported command that takes a repository and two
commits and emits a review site. Today that flow is assembled from internal
components and tests, not exposed as a product CLI.

## Options

| Avenue | What users do | Host needed | Effort | Best for |
| --- | --- | --- | --- | --- |
| **Local CLI** | `csb review --repo . --base origin/main --head HEAD --out review/` then serve/open the static site | None | Medium | Any Git host; developers evaluating a change |
| **OCI/Docker one-shot** | Run the same CLI in a pinned container in any CI or on a laptop | None | Medium | Reproducible local and CI use without a Rust toolchain |
| **GitHub Action, artifact/check only** | Add one workflow; action runs the container, uploads the report artifact and optionally writes an advisory check with `GITHUB_TOKEN` | User's repo or Pages | Medium | GitHub users who do not want an App install |
| **GitLab CI component** | Include a component in `.gitlab-ci.yml`; job emits a report artifact and optional GitLab Pages site | User's GitLab | Medium | GitLab merge requests |
| **Bitbucket Pipe / Azure task** | Run the container in a pipeline job; publish the static site | User's provider | Medium each | Teams already on those providers |
| **Self-hosted single-tenant server** | `docker compose up` with a local web UI and optional webhook adapter | User's server | Medium/large | Organizations that want a private review service but no SaaS |
| **Hosted public-repository analyzer** | Paste a public repository URL and two commits; receive a report | Project-operated | Large | Zero-install trials on public code |
| **Private upload / web UI** | Upload a Git bundle or archive for analysis | Project-operated | Large | Private code, only after retention and deletion are designed |
| **IDE extension** | Analyze the working tree or a local diff inside the editor | None | Large | Developer loop and pre-push review |

## Recommended sequence

### Phase 1: Stable local CLI and container

Ship a supported `csb review` command:

```sh
csb review \
  --repo /path/to/repository \
  --base <exact-commit> \
  --head <exact-commit> \
  --include src \
  --out /path/to/new-review-site
```

Acceptance criteria:

- exact commits only; reject dirty worktrees, symbolic refs and ambiguous input;
- no checkout, build, hooks or package scripts executed;
- deterministic output for the same repository and commits;
- emits the verified static site, public bundle and manifest hash;
- non-zero exit for unsupported language, incomplete scope or failed verification;
- no GitHub token, App private key or network access required;
- documented local serve command and release binaries.

This is the foundation for every other avenue. It also makes the project
testable by contributors without configuring Pages.

### Phase 2: CI-native adapters without an App

- **GitHub Action:** composite action around the container; upload the report as
  an artifact and optionally create an advisory check or PR comment. No App
  installation, no Pages requirement.
- **GitLab CI component:** pinned image, MR pipeline job, report artifact, optional
  GitLab Pages deployment.
- **Bitbucket Pipe / Azure task:** same container contract, provider-specific
  metadata only.

Each adapter should require the user to choose include/exclude paths and accept
the coverage model. Provider tokens stay inside that provider's job.

### Phase 3: Self-hosted single-tenant service

A Docker Compose stack with the existing host and a provider-neutral local UI:

- one repository or a small allowlist per deployment;
- webhook adapters for GitLab/Bitbucket or manual commit selection;
- persistent SQLite and report storage owned by the operator;
- no shared multi-tenant control plane;
- explicit backup, restore and deletion instructions.

This is the realistic path for private repositories before any hosted product.

### Phase 4: Project-hosted public analyzer

A zero-install page for public repositories:

- user supplies repository URL, base and head;
- analyzer fetches only public Git objects in a bounded sandbox;
- report is immutable and expires under a published retention policy;
- quotas and abuse controls are explicit;
- no private code, no secrets, no arbitrary CI execution.

This requires a real host and operational budget. It must not be presented as a
general multi-tenant service until isolation, quotas, retention and recovery are
proven.

### Phase 5: IDE and deeper provider support

- editor extension for local diffs and working-tree review;
- deeper GitLab/Bitbucket/Azure integrations only after the CLI contract is stable;
- additional languages only after their analysis profiles and coverage semantics
  exist.

## Non-goals for the first wave

- Removing the GitHub App path; it remains the lowest-friction GitHub option.
- Private-code multi-tenant hosting.
- Non-Rust analysis.
- Running PR builds, tests, package scripts or arbitrary containers.
- Claiming merge approval. Every current check is advisory.

## Decision

Start with **Phase 1**. Without a stable local CLI and container, every other
avenue duplicates orchestration and cannot be tested consistently. The GitHub
App can remain available in parallel, but it should not be the only on-ramp.