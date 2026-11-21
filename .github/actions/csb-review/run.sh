#!/usr/bin/env bash
# Review and verify a pull request with a pinned CodeShouldBehave image.
# The composite action's final step decides whether the job fails.
set -u

repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
runner_temp="${RUNNER_TEMP:-/tmp}"
event_path="${GITHUB_EVENT_PATH:-}"
image="${INPUT_IMAGE:-}"
design="${INPUT_DESIGN:-auto}"
out_dir="${INPUT_OUT_DIR:-}"
base="${INPUT_BASE:-}"
head="${INPUT_HEAD:-}"

fail() {
  echo "::error::$*"
  exit 1
}

command -v docker >/dev/null 2>&1 || fail "Docker is required for this action; use an ubuntu-latest runner"
command -v jq >/dev/null 2>&1 || fail "jq is required for this action"
[ -n "$image" ] || fail "image input is empty"
[ -d "$repo_root/.git" ] || fail "checkout the repository with fetch-depth: 0 before this action"

if [ -z "$base" ] || [ -z "$head" ]; then
  [ -n "$event_path" ] && [ -f "$event_path" ] || fail "no explicit base/head and no GitHub event payload"
  [ -n "$base" ] || base="$(jq -r '.pull_request.base.sha // empty' "$event_path")"
  [ -n "$head" ] || head="$(jq -r '.pull_request.head.sha // empty' "$event_path")"
fi
[[ "$base" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]] || fail "base must be an exact commit SHA"
[[ "$head" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]] || fail "head must be an exact commit SHA"

if [ -z "$out_dir" ]; then
  out_dir="$runner_temp/csb-review"
fi
if [ -e "$out_dir" ] && [ -n "$(ls -A "$out_dir" 2>/dev/null)" ]; then
  fail "out-dir must be empty: $out_dir"
fi
mkdir -p "$out_dir"

summary="$runner_temp/csb-summary.md"
stderr_file="$runner_temp/csb-review.stderr"
verify_file="$runner_temp/csb-verify.txt"
: > "$summary"
: > "$stderr_file"
: > "$verify_file"

set +e
docker run --rm \
  -v "$repo_root:/repo:ro" \
  -v "$out_dir:/out" \
  "$image" review \
    --repo /repo \
    --base "$base" \
    --head "$head" \
    --out-dir /out \
    --allow-dirty \
    --design "$design" \
    --summary markdown > "$summary" 2> "$stderr_file"
review_exit=$?

verify_exit=1
if [ -f "$out_dir/manifest.json" ]; then
  docker run --rm \
    -v "$out_dir:/review:ro" \
    "$image" verify --dir /review > "$verify_file" 2>&1
  verify_exit=$?
fi
set -e

coverage="failed"
if [ "$review_exit" -eq 0 ] && [ "$verify_exit" -eq 0 ]; then
  coverage="complete"
elif [ "$review_exit" -eq 1 ] && [ "$verify_exit" -eq 0 ]; then
  coverage="incomplete"
fi

{
  echo "out-dir=$out_dir"
  echo "summary-path=$summary"
  echo "review-exit=$review_exit"
  echo "verify-exit=$verify_exit"
  echo "coverage=$coverage"
} >> "${GITHUB_OUTPUT:-/dev/stdout}"

echo "CodeShouldBehave coverage: $coverage (review=$review_exit, verify=$verify_exit)"
if [ -s "$stderr_file" ]; then
  cat "$stderr_file" >&2
fi
if [ -s "$verify_file" ]; then
  cat "$verify_file"
fi

# The final composite step maps these codes to a job result. Do not fail here so
# the artifact and summary comment still run.
exit 0
