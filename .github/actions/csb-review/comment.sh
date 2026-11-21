#!/usr/bin/env bash
# Post or update one summary comment. Failures are non-fatal in the action.
set -euo pipefail

summary="${INPUT_SUMMARY:-}"
event_path="${GITHUB_EVENT_PATH:-}"
[ -s "$summary" ] || exit 0
[ -n "$event_path" ] && [ -f "$event_path" ] || exit 0
command -v gh >/dev/null 2>&1 || { echo "::warning::gh is unavailable; skipping comment"; exit 0; }
command -v jq >/dev/null 2>&1 || { echo "::warning::jq is unavailable; skipping comment"; exit 0; }

pr="$(jq -r '.pull_request.number // empty' "$event_path")"
[ -n "$pr" ] || exit 0
[ -n "${GH_TOKEN:-}" ] || { echo "::warning::GH_TOKEN is unavailable; skipping comment"; exit 0; }

runner_temp="${RUNNER_TEMP:-/tmp}"
marker='<!-- csb-review -->'
body_file="$runner_temp/csb-comment.md"
payload_file="$runner_temp/csb-comment.json"
{
  printf '%s\n' "$marker"
  head -c 60000 "$summary"
} > "$body_file"
jq -n --rawfile body "$body_file" '{body: $body}' > "$payload_file"

existing="$(
  gh api --paginate "repos/${GITHUB_REPOSITORY}/issues/${pr}/comments?per_page=100" 2>/dev/null \
    | jq -r --arg marker "$marker" '.[]? | select(.body | contains($marker)) | .id' \
    | head -n 1 || true
)"

if [ -n "$existing" ]; then
  gh api -X PATCH "repos/${GITHUB_REPOSITORY}/issues/comments/${existing}" \
    --input "$payload_file" >/dev/null
else
  gh api -X POST "repos/${GITHUB_REPOSITORY}/issues/${pr}/comments" \
    --input "$payload_file" >/dev/null
fi
echo "CodeShouldBehave summary comment updated"
