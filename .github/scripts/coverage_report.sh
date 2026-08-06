#!/usr/bin/env bash
# Merge LLVM coverage profiles from a dedicated profraw directory and emit a
# combined multi-binary report: html/, summary.json, and report.txt.
#
# Usage:
#   coverage_report.sh --profraw-dir DIR --output-dir DIR --title NAME \
#     --object BIN [--object BIN ...]
#
# Binaries must be built with:
#   RUSTFLAGS="-Cinstrument-coverage -Cllvm-args=-runtime-counter-relocation"
# and run with LLVM_PROFILE_FILE containing %c (continuous mode) so profiles
# survive SIGTERM/SIGKILL teardown. Requires the rustup llvm-tools component.
set -euo pipefail

IGNORE_REGEX='\.cargo/(registry|git)|/rustc/|harness/tests/'

profraw_dir=""
output_dir=""
title="coverage"
objects=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profraw-dir) profraw_dir="$2"; shift 2 ;;
    --output-dir) output_dir="$2"; shift 2 ;;
    --title) title="$2"; shift 2 ;;
    --object) objects+=("$2"); shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ -n "$profraw_dir" && -n "$output_dir" ]] || {
  echo "usage: $0 --profraw-dir DIR --output-dir DIR --title NAME --object BIN [--object BIN ...]" >&2
  exit 2
}

tool_dir="$(dirname "$(rustc --print target-libdir)")/bin"
for tool in llvm-profdata llvm-cov; do
  [[ -x "$tool_dir/$tool" ]] || {
    echo "missing $tool_dir/$tool; install with: rustup component add llvm-tools" >&2
    exit 2
  }
done

mkdir -p "$output_dir"

mapfile -t profraws < <(find "$profraw_dir" -name '*.profraw' -type f 2>/dev/null | sort)
if [[ ${#profraws[@]} -eq 0 ]]; then
  echo "no .profraw files under $profraw_dir; skipping report" >&2
  exit 0
fi

object_args=()
for object in "${objects[@]}"; do
  if [[ -x "$object" ]]; then
    object_args+=(-object "$object")
  else
    echo "warning: skipping missing object $object" >&2
  fi
done
[[ ${#object_args[@]} -gt 0 ]] || { echo "no usable --object binaries" >&2; exit 2; }

demangler_args=()
if command -v rustfilt >/dev/null 2>&1; then
  demangler_args=(-Xdemangler=rustfilt)
fi

"$tool_dir/llvm-profdata" merge -sparse "${profraws[@]}" -o "$output_dir/merged.profdata"

"$tool_dir/llvm-cov" show --format=html --output-dir="$output_dir/html" \
  -instr-profile="$output_dir/merged.profdata" \
  --ignore-filename-regex="$IGNORE_REGEX" \
  "${demangler_args[@]}" \
  "${object_args[@]}"

"$tool_dir/llvm-cov" report \
  -instr-profile="$output_dir/merged.profdata" \
  --ignore-filename-regex="$IGNORE_REGEX" \
  "${object_args[@]}" > "$output_dir/report.txt"

"$tool_dir/llvm-cov" export -summary-only \
  -instr-profile="$output_dir/merged.profdata" \
  --ignore-filename-regex="$IGNORE_REGEX" \
  "${object_args[@]}" \
  | jq --arg title "$title" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{title: $title, generated_at: $at, totals: .data[0].totals}' \
  > "$output_dir/summary.json"

echo "coverage report written to $output_dir"
tail -1 "$output_dir/report.txt"
