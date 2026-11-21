#!/usr/bin/env bash
# Keep adoption templates on the reviewed immutable image digest.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
digest='sha256:53e53a1f6374c58dacf90e1c52eca4130d1c82531f217edb56f31a43e507c075'
image="ghcr.io/ssc-sde/csb@${digest}"

fail() {
  echo "pin check failed: $*" >&2
  exit 1
}

grep -Fq "$image" "$root/action.yml" || fail "action.yml does not use $image"
grep -Fq "$image" "$root/templates/csb-ci/gitlab-ci.yml" || fail "gitlab template is unpinned"
grep -Fq "$image" "$root/templates/csb-ci/bitbucket-pipelines.yml" || fail "bitbucket template is unpinned"
grep -Fq "$image" "$root/templates/csb-ci/github-actions-pages.yml" || fail "pages template is unpinned"
grep -Fq "$digest" "$root/templates/csb-ci/README.md" || fail "template README is missing the digest"

if grep -R "ghcr.io/ssc-sde/csb:latest" "$root/action.yml" "$root/templates/csb-ci" >/dev/null; then
  fail "a moving :latest tag remains in an adoption template"
fi

echo "csb image pins are consistent: $digest"
