#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "Usage: scripts/phase1_acceptance_replay.sh NEW_OUTPUT_DIRECTORY BROWSER_REPORT" >&2
    exit 2
fi

output=$1
browser_report=$2
if [ -e "$output" ]; then
    echo "Refusing to overwrite existing output: $output" >&2
    exit 2
fi
if [ ! -f "$browser_report" ]; then
    echo "Browser report does not exist: $browser_report" >&2
    exit 2
fi

mkdir "$output"
for run in 1 2 3; do
    scripts/phase1_load_replay.sh "$output/run-$run"
done
python3 scripts/summarize_phase1_replays.py "$output" "$browser_report"
