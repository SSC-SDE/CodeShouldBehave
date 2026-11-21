# CI templates

Copy the file for your provider into the repository you want reviewed.

| Provider | Template | Destination |
|---|---|---|
| GitHub Actions | `github-actions.yml` | `.github/workflows/csb-review.yml` |
| GitHub Actions Pages (legacy) | `github-actions-pages.yml` | Deprecated: the review action publishes Pages directly |
| GitLab CI | `gitlab-ci.yml` | merge into `.gitlab-ci.yml` |
| Bitbucket Pipelines | `bitbucket-pipelines.yml` | merge into `bitbucket-pipelines.yml` |

The GitHub template uses the first-party composite action
`SSC-SDE/CodeShouldBehave@v1`. It runs the pinned image, verifies the report,
publishes the interactive report to the repository's Pages site, posts one
pull-request comment and uploads the immutable artifact.

Enable Pages once before the first run: **Settings → Pages → Build and
deployment → Source: GitHub Actions**. The action deploys the report and links
it at the top of the PR comment. If Pages is unavailable, the comment links the
workflow artifact. Pages keeps only the latest deployment; the artifact is the
immutable copy.

`v1` is a convenience tag that tracks compatible action releases. For an
immutable production pin, use the current v1 commit SHA:
`702f8f3a78333be1acc37eb06b103c95fc5e8d25`. The exact action release is also
available as `@v0.7.0`. Do not use `@main`.

All non-action templates pin the `v0.6.0` image by immutable digest:
`ghcr.io/ssc-sde/csb@sha256:53e53a1f6374c58dacf90e1c52eca4130d1c82531f217edb56f31a43e507c075`.
Do not use `:latest` in an adoption template. Bump the digest only after a
provider-confirmed release, and review the diff before merging.

All templates run `csb review` against exact commit IDs supplied by the CI
provider. They upload the interactive review as an artifact and, on GitHub,
post or update a single pull-request summary comment.

`--allow-dirty` is required in CI because the runner may hold build outputs or
tool caches outside the reviewed commits. Analysis still reads committed Git
objects only.

The container image must be published first. See
[`docs/CLI_USAGE.md`](../../docs/CLI_USAGE.md).

## Publishing a browsable report

The GitHub Actions pair deploys each report to the adopter's own GitHub Pages.
Nothing is committed to the repository; the Pages artifact is built from the CI
run's temporary output directory.

1. Copy both `github-actions.yml` → `.github/workflows/csb-review.yml` and
   `github-actions-pages.yml` → `.github/workflows/csb-pages.yml`.
2. Settings → Pages → Build and deployment → Source: **GitHub Actions**.
3. Settings → Secrets and variables → Actions → Variables: add `CSB_PAGES` with
   value `true`.
4. Run the review workflow on a same-repository pull request.

The review workflow verifies the report and uploads it. The companion workflow
is triggered when that run completes, re-verifies every hash, and deploys the
artifact. It runs on the default branch because GitHub Pages commonly protects
the `github-pages` environment to that branch; a direct pull-request deployment
would be rejected by that policy. The summary comment gains a link to the
deployed report.

Pages hosts only the latest deployment, so a new pull request replaces the
previous report; download the 30-day artifact when an immutable copy is
required. Fork pull requests are not deployed because their workflow token is
read-only — they still receive the summary comment and artifact. The companion
workflow deploys an artifact produced by the pull-request workflow; enable it
only if repository collaborators are trusted to change workflow files.

The template runs `--design auto`, so the report includes HLD/LLD derived from
the committed Cargo workspace. Those component boundaries are **provisional**:
they are workspace-derived, not human-confirmed, and the report labels them
accordingly. Use `--design-context` with a confirmed accepted-context file when
authoritative architecture is required.
