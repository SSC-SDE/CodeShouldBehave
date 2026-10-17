#!/bin/sh
set -eu

usage() {
  cat <<'EOF'
Usage: scripts/github_mvp.sh REPOSITORY PR_NUMBER OUTPUT_DIRECTORY [--no-serve]

Creates a real CodeShouldBehave review for one GitHub pull request using the
GitHub CLI's existing authenticated session. Credentials remain in `gh`'s
credential store and are never passed to CodeShouldBehave or the browser.

Environment:
  CSB_PORT          Loopback port (default: 3195)
  CSB_FLOWDIFF_BIN  Prebuilt flowdiff binary (otherwise cargo builds release)
  CSB_CACHE_DIR     Trusted local cache (default: OUTPUT_DIRECTORY/.cache)
EOF
}

if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
  usage >&2
  exit 2
fi

repository=$1
pr_number=$2
output_directory=$3
serve=true
if [ "${4:-}" = "--no-serve" ]; then
  serve=false
elif [ "$#" -eq 4 ]; then
  usage >&2
  exit 2
fi

case "$pr_number" in
  ''|*[!0-9]*) printf 'PR_NUMBER must be a positive integer.\n' >&2; exit 2 ;;
  0) printf 'PR_NUMBER must be a positive integer.\n' >&2; exit 2 ;;
esac
case "${CSB_PORT:-3195}" in
  ''|*[!0-9]*) printf 'CSB_PORT must be numeric.\n' >&2; exit 2 ;;
esac

for command in git gh; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'Required command is unavailable: %s\n' "$command" >&2
    exit 2
  }
done

git -C "$repository" rev-parse --is-inside-work-tree >/dev/null
repository=$(cd "$repository" && pwd -P)
gh auth status --hostname github.com >/dev/null

slug=$(cd "$repository" && gh repo view --json nameWithOwner --jq .nameWithOwner)
base_branch=$(gh pr view "$pr_number" --repo "$slug" --json baseRefName --jq .baseRefName)
base_oid=$(gh pr view "$pr_number" --repo "$slug" --json baseRefOid --jq .baseRefOid)
head_oid=$(gh pr view "$pr_number" --repo "$slug" --json headRefOid --jq .headRefOid)
pr_url=$(gh pr view "$pr_number" --repo "$slug" --json url --jq .url)

case "$slug" in */*) ;; *) printf 'GitHub returned an invalid repository identity.\n' >&2; exit 1 ;; esac
git check-ref-format --branch "$base_branch" >/dev/null 2>&1 || {
  printf 'GitHub returned an unsafe base branch.\n' >&2
  exit 1
}
case "$base_oid:$head_oid" in
  *[!0-9a-fA-F:]*|*:*:*) printf 'GitHub returned an invalid pull request revision.\n' >&2; exit 1 ;;
esac
[ "${#base_oid}" -eq 40 ] && [ "${#head_oid}" -eq 40 ] || {
  printf 'GitHub returned an invalid pull request revision.\n' >&2
  exit 1
}

mkdir -p "$output_directory"
output_directory=$(cd "$output_directory" && pwd -P)
report="$output_directory/pr-$pr_number.html"
if [ -e "$report" ] || [ -L "$report" ]; then
  printf 'Review already exists: %s\n' "$report" >&2
  exit 2
fi

temporary_base_ref="refs/codeshouldbehave/pr-$pr_number-base-$$"
temporary_head_ref="refs/codeshouldbehave/pr-$pr_number-head-$$"
cleanup() {
  git -C "$repository" update-ref -d "$temporary_base_ref" >/dev/null 2>&1 || true
  git -C "$repository" update-ref -d "$temporary_head_ref" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

printf 'Authenticating: GitHub CLI credential store\n'
printf 'Repository:     %s\n' "$slug"
printf 'Pull request:   %s\n' "$pr_url"
printf 'Target branch:  %s\n' "$base_branch"

git -C "$repository" fetch --no-tags origin "$base_oid:$temporary_base_ref"
git -C "$repository" fetch --no-tags origin "pull/$pr_number/head:$temporary_head_ref"
base_commit=$(git -C "$repository" rev-parse --verify "$temporary_base_ref^{commit}")
proposal_commit=$(git -C "$repository" rev-parse --verify "$temporary_head_ref^{commit}")
[ "$base_commit" = "$base_oid" ] && [ "$proposal_commit" = "$head_oid" ] || {
  printf 'Fetched revisions do not match GitHub pull request metadata.\n' >&2
  exit 1
}

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_root=$(CDPATH= cd -- "$script_directory/.." && pwd)
if [ -n "${CSB_FLOWDIFF_BIN:-}" ]; then
  flowdiff_binary=$CSB_FLOWDIFF_BIN
else
  cargo build --release --features server --manifest-path "$project_root/Cargo.toml"
  flowdiff_binary="$project_root/target/release/flowdiff"
fi

cache_directory=${CSB_CACHE_DIR:-$output_directory/.cache}
"$flowdiff_binary" review \
  --repo "$repository" \
  --github-repository "$slug" \
  --base "$base_commit" \
  --head "$proposal_commit" \
  --merge-base \
  --cache-dir "$cache_directory" \
  --artifact "$output_directory/pr-$pr_number.json" \
  --output "$report"

cleanup
trap - EXIT HUP INT TERM

printf '\nReview ready: %s\n' "$report"
if [ "$serve" = false ]; then
  exit 0
fi

command -v python3 >/dev/null 2>&1 || {
  printf 'python3 is required to serve the browser review.\n' >&2
  exit 2
}
port=${CSB_PORT:-3195}
printf 'Browser URL: http://127.0.0.1:%s/pr-%s.html\n' "$port" "$pr_number"
printf 'Press Ctrl-C to stop the loopback server.\n'
exec python3 -m http.server "$port" --bind 127.0.0.1 --directory "$output_directory"
