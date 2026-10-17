#!/bin/sh
set -eu

runtime=${TMPDIR:-/tmp}/codeshouldbehave-github-mvp
pid_file=$runtime/server.pid

if [ ! -f "$pid_file" ]; then
  printf 'CodeShouldBehave is not running.\n'
  exit 0
fi

pid=$(cat "$pid_file")
case "$pid" in ''|*[!0-9]*) printf 'Invalid CodeShouldBehave process record.\n' >&2; exit 1 ;; esac

if kill -0 "$pid" 2>/dev/null; then
  kill "$pid"
  printf 'CodeShouldBehave stopped.\n'
else
  printf 'CodeShouldBehave was already stopped.\n'
fi
rm -f "$pid_file"
