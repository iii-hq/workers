#!/usr/bin/env bash
set -euo pipefail

log_file=${1:?usage: replay-terminal.sh <terminal-log>}
delay=${TERMINAL_REPLAY_DELAY:-0.04}
max_lines=${TERMINAL_REPLAY_MAX_LINES:-240}

[[ -f "$log_file" ]] || {
  echo "terminal log not found: $log_file" >&2
  exit 1
}
[[ "$delay" =~ ^(0|[0-9]+)(\.[0-9]+)?$ ]] || {
  echo "TERMINAL_REPLAY_DELAY must be a non-negative number" >&2
  exit 2
}
if [[ ! "$max_lines" =~ ^[0-9]+$ ]]; then
  echo "TERMINAL_REPLAY_MAX_LINES must be an integer greater than one" >&2
  exit 2
fi
max_lines=$((10#$max_lines))
if ((max_lines <= 1)); then
  echo "TERMINAL_REPLAY_MAX_LINES must be an integer greater than one" >&2
  exit 2
fi

# Normalize carriage-return progress output and remove terminal control
# sequences before replaying it into a fresh terminal.
mapfile -t lines < <(
  tr '\r' '\n' <"$log_file" |
    sed -E $'s/\033\\[[0-9;?]*[ -\\/]*[@-~]//g'
)

line_count=${#lines[@]}
if ((line_count <= max_lines)); then
  replay=("${lines[@]}")
else
  head_lines=$((max_lines / 3))
  tail_lines=$((max_lines - head_lines - 1))
  omitted=$((line_count - head_lines - tail_lines))
  replay=(
    "${lines[@]:0:head_lines}"
    "... ${omitted} lines omitted from the recording ..."
    "${lines[@]:line_count-tail_lines:tail_lines}"
  )
fi

for line in "${replay[@]}"; do
  printf '%s\n' "$line"
  sleep "$delay"
done

printf '\nTERMINAL REPLAY COMPLETE\n'
