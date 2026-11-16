# CI templates

Copy the file for your provider into the repository you want reviewed.

| Provider | Template | Destination |
|---|---|---|
| GitHub Actions | `github-actions.yml` | `.github/workflows/csb-review.yml` |
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
