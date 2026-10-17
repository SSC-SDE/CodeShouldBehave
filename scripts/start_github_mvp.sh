#!/bin/sh
set -eu
umask 077
repository=${1:-.}
port=${CSB_PORT:-3195}
output=${CSB_MVP_OUTPUT:-/tmp/codeshouldbehave-github-mvp}
native_profile=${CSB_NATIVE_PROFILE:-release}
case $native_profile in
  debug|release) ;;
  *) printf 'CSB_NATIVE_PROFILE must be debug or release\n' >&2; exit 2 ;;
esac
runtime=${TMPDIR:-/tmp}/codeshouldbehave-github-mvp
mkdir -p "$runtime" "$output"
pid_file=$runtime/server.pid
log_file=$runtime/server.log
if [ -f "$pid_file" ] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then
  printf 'CodeShouldBehave is already running at http://127.0.0.1:%s/\n' "$port"
  exit 0
fi
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
set -- build --locked --features server --bin csb-onboarding --bin flowdiff --manifest-path "$project_root/Cargo.toml"
if [ "$native_profile" = release ]; then set -- "$@" --release; fi
cargo "$@"
set -- --repo "$repository" --output "$output" --port "$port" --native-profile "$native_profile" --auth-profile "${CSB_AUTH_PROFILE:-github-app}"
case ${CSB_NEW_ONBOARDING:-1} in
  1) ;;
  0) set -- "$@" --disable-new-onboarding ;;
  *) printf 'CSB_NEW_ONBOARDING must be 0 or 1\n' >&2; exit 1 ;;
esac
case ${CSB_NEW_HLD:-1} in
  1) ;;
  0) set -- "$@" --disable-new-hld ;;
  *) printf 'CSB_NEW_HLD must be 0 or 1\n' >&2; exit 1 ;;
esac
case ${CSB_NEW_LLD:-1} in
  1) ;;
  0) set -- "$@" --disable-new-lld ;;
  *) printf 'CSB_NEW_LLD must be 0 or 1\n' >&2; exit 1 ;;
esac
nohup python3 "$(dirname "$0")/github_mvp_server.py" "$@" >"$log_file" 2>&1 &
pid=$!
printf '%s\n' "$pid" > "$pid_file"
sleep 1
kill -0 "$pid" 2>/dev/null || { cat "$log_file" >&2; exit 1; }
url=http://127.0.0.1:$port/
printf 'CodeShouldBehave started: %s\n' "$url"
if command -v open >/dev/null 2>&1; then open "$url"; fi
