#!/bin/sh
set -eu

usage() {
  printf '%s\n' \
    'Usage: scripts/local_repo_review.sh REPOSITORY BASE PROPOSAL OUTPUT [CACHE_DIR]' \
    '' \
    'Builds a local, read-only PR review from committed Git objects.' \
    'BASE and PROPOSAL must already exist locally. The output file must not exist.'
}

if [ "$#" -lt 4 ] || [ "$#" -gt 5 ]; then
  usage >&2
  exit 2
fi

repository=$1
base=$2
proposal=$3
output=$4
cache_dir=${5:-/tmp/codeshouldbehave-local-review-cache}
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

if [ -e "$output" ] || [ -L "$output" ]; then
  printf 'Output already exists: %s\n' "$output" >&2
  exit 2
fi

git -C "$repository" rev-parse --is-inside-work-tree >/dev/null
base_commit=$(git -C "$repository" rev-parse --verify "$base^{commit}")
proposal_commit=$(git -C "$repository" rev-parse --verify "$proposal^{commit}")

printf 'Repository: %s\n' "$repository"
printf 'Baseline:   %s\n' "$base_commit"
printf 'Proposal:   %s\n' "$proposal_commit"

cargo build --release --manifest-path "$project_root/Cargo.toml"
"$project_root/target/release/flowdiff" review \
  --repo "$repository" \
  --base "$base_commit" \
  --head "$proposal_commit" \
  --merge-base \
  --cache-dir "$cache_dir" \
  --output "$output"

printf '\nReview generated. Serve only its containing directory on loopback, for example:\n'
printf 'python3 -m http.server 3194 --bind 127.0.0.1 --directory %s\n' "$(dirname -- "$output")"
