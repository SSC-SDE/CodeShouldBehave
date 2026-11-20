# CI templates

Copy the file for your provider into the repository you want reviewed.

| Provider | Template | Destination |
|---|---|---|
| GitHub Actions | `github-actions.yml` | `.github/workflows/csb-review.yml` |
| GitHub Actions Pages (optional) | `github-actions-pages.yml` | `.github/workflows/csb-pages.yml` |
| GitLab CI | `gitlab-ci.yml` | merge into `.gitlab-ci.yml` |
| Bitbucket Pipelines | `bitbucket-pipelines.yml` | merge into `bitbucket-pipelines.yml` |

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
