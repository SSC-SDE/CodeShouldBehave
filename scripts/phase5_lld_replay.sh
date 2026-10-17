#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "Usage: scripts/phase5_lld_replay.sh NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi
output=$1
if [ -e "$output" ]; then
    echo "Refusing to overwrite existing replay output" >&2
    exit 2
fi

replay_commit_before=$(git rev-parse HEAD)
if [ -n "$(git status --porcelain)" ]; then replay_dirty_before=true; else replay_dirty_before=false; fi
cargo +1.87.0 run --quiet --release --locked --features server --example phase5_lld_replay -- "$output"
{
    echo "schema=codeshouldbehave-phase5-lld-replay-metadata-v1"
    echo "repository_commit_at_build=$replay_commit_before"
    echo "working_tree_dirty_at_build=$replay_dirty_before"
    echo "repository_commit_at_finish=$(git rev-parse HEAD)"
    if [ -n "$(git status --porcelain)" ]; then echo "working_tree_dirty_at_finish=true"; else echo "working_tree_dirty_at_finish=false"; fi
    echo "rustc=$(rustc +1.87.0 --version)"
    echo "cargo=$(cargo +1.87.0 --version)"
    echo "git=$(git --version)"
    echo "host=$(uname -a)"
    echo "command=sh scripts/phase5_lld_replay.sh NEW_OUTPUT_DIRECTORY"
    echo "profile=real durable local HLD/LLD replay; no hosted/concurrent capacity claim"
    if command -v sysctl >/dev/null 2>&1; then sysctl hw.memsize hw.ncpu hw.machine 2>/dev/null || true; fi
} > "$output/metadata.txt"
echo "Additive HLD/LLD replay evidence written to $output"
