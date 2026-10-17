#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 3 ]; then
    echo "Usage: scripts/phase1_baseline.sh OUTPUT_DIR [COMMIT] [REPOSITORY]" >&2
    exit 2
fi

output_dir=$1
baseline_commit=${2:-HEAD}
repository=${3:-.}

if [ -e "$output_dir" ]; then
    echo "Refusing to overwrite existing output: $output_dir" >&2
    exit 2
fi

mkdir -p "$output_dir"
cache_dir="$output_dir/cache"
resolved_commit=$(git -C "$repository" rev-parse --verify "$baseline_commit^{commit}")
git -C "$repository" ls-tree -r -l "$resolved_commit" > "$output_dir/tree.txt"

{
    echo "timestamp_utc=$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    echo "commit=$resolved_commit"
    echo "repository=$repository"
    echo "os=$(uname -srm)"
    echo "rustc=$(rustc --version)"
    echo "cargo=$(cargo --version)"
    echo "git=$(git --version)"
    echo "node=$(node --version)"
    echo "tracked_files=$(wc -l < "$output_dir/tree.txt" | tr -d ' ')"
    echo "tracked_bytes=$(awk '{ if ($4 != "-") total += $4 } END { print total + 0 }' "$output_dir/tree.txt")"
} > "$output_dir/environment.txt"

if [ "${CSB_SKIP_VALIDATION:-0}" != "1" ]; then
    cargo fmt --check > "$output_dir/fmt.log" 2>&1
    cargo test > "$output_dir/test.log" 2>&1
    cargo clippy --all-targets -- -D warnings > "$output_dir/clippy.log" 2>&1
    node --test tests/ui.test.cjs > "$output_dir/ui-test.log" 2>&1
    cargo build --release > "$output_dir/build.log" 2>&1
fi

if [ "$(uname -s)" = "Darwin" ]; then
    /usr/bin/time -l target/release/flowdiff index \
        --repo "$repository" --base "$resolved_commit" --cache-dir "$cache_dir" \
        --output "$output_dir/cold.html" \
        > "$output_dir/cold.stdout" 2> "$output_dir/cold.time" ||
        test -s "$output_dir/cold.html"
    /usr/bin/time -l target/release/flowdiff index \
        --repo "$repository" --base "$resolved_commit" --cache-dir "$cache_dir" \
        --output "$output_dir/warm.html" \
        > "$output_dir/warm.stdout" 2> "$output_dir/warm.time" ||
        test -s "$output_dir/warm.html"
else
    /usr/bin/time -v target/release/flowdiff index \
        --repo "$repository" --base "$resolved_commit" --cache-dir "$cache_dir" \
        --output "$output_dir/cold.html" \
        > "$output_dir/cold.stdout" 2> "$output_dir/cold.time"
    /usr/bin/time -v target/release/flowdiff index \
        --repo "$repository" --base "$resolved_commit" --cache-dir "$cache_dir" \
        --output "$output_dir/warm.html" \
        > "$output_dir/warm.stdout" 2> "$output_dir/warm.time"
fi

{
    wc -c target/release/flowdiff "$output_dir/cold.html" "$output_dir/warm.html"
    find "$cache_dir" -type f | wc -l
    du -sk "$cache_dir"
} > "$output_dir/sizes.txt"

echo "Phase 1 baseline captured in $output_dir"
echo "Read environment.txt, *.time, *.stdout, sizes.txt, and the validation logs."
