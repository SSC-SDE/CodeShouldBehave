#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "Usage: scripts/phase1_load_replay.sh NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi

output=$1
if [ -e "$output" ]; then
    echo "Refusing to overwrite existing output: $output" >&2
    exit 2
fi

cargo run --quiet --release --example phase1_load_replay -- "$output"
{
    echo "schema=codeshouldbehave-phase1-load-replay-metadata-v5"
    echo "repository_commit=$(git rev-parse HEAD)"
    echo "rustc=$(rustc --version)"
    echo "cargo=$(cargo --version)"
    echo "host=$(uname -a)"
    echo "command=scripts/phase1_load_replay.sh $output"
} > "$output/metadata.txt"

echo "Replay artifacts written to $output"
