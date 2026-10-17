#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "Usage: scripts/generate_phase1_corpora.sh NEW_OUTPUT_ROOT" >&2
    exit 2
fi

root=$1
if [ -e "$root" ]; then
    echo "Refusing to overwrite existing output: $root" >&2
    exit 2
fi
mkdir -p "$root"

create_corpus() {
    name=$1
    files=$2
    repository="$root/$name"
    mkdir -p "$repository/src"
    git -C "$repository" init -q
    git -C "$repository" config user.name "CodeShouldBehave fixture"
    git -C "$repository" config user.email "fixture@example.invalid"
    index=1
    while [ "$index" -le "$files" ]; do
        previous=$((index - 1))
        if [ "$index" -eq 1 ]; then
            printf 'pub fn service_%04d(input: u64) -> u64 { input + 1 }\n' "$index" \
                > "$repository/src/service_$(printf '%04d' "$index").rs"
        else
            printf 'pub fn service_%04d(input: u64) -> u64 { service_%04d(input) + 1 }\n' \
                "$index" "$previous" \
                > "$repository/src/service_$(printf '%04d' "$index").rs"
        fi
        index=$((index + 1))
    done
    printf '# %s deterministic Rust corpus\n\nGenerated for local capacity baselines.\n' "$name" \
        > "$repository/README.md"
    git -C "$repository" add README.md src
    git -C "$repository" -c core.hooksPath=/dev/null commit -qm "create $name corpus"
    git -C "$repository" rev-parse HEAD > "$repository/COMMIT"
}

create_corpus small 25
create_corpus medium 250
create_corpus large 1500

echo "Generated deterministic corpora under $root"
