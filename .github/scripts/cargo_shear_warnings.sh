#!/usr/bin/env bash
# Report unused Rust dependencies as warnings, never as a failure.
#
# Usage (from the crate or workspace directory being checked):
#   cargo_shear_warnings.sh <path of this directory from the repository root>
#
# cargo-shear reads the sources without compiling. Each finding becomes a
# GitHub warning annotation on its manifest line; the argument prefixes the
# annotation path (cargo-shear reports it relative to the current directory)
# so the annotation lands on the right file. cargo-shear reports error-level
# findings (unused or misplaced dependencies) with exit 1, warning-level ones
# (unused optional or feature-only dependencies, ...) as ::warning with exit 0,
# and exits 2 when it cannot run. Every case ends as warnings, and this script
# exits 0.
#
# Fix a finding with `cargo shear --fix`. Keep an intentional dependency (one
# used only through a `#[path]` module that lives in another crate, or one
# declared only to pin a transitive crate's features) with:
#   [package.metadata.cargo-shear]
#   ignored = ["crate-name"]
set -uo pipefail

prefix="${1:-.}"
prefix="${prefix%/}"

status=0
output="$(cargo shear --locked --format=github 2>&1)" || status=$?
findings="$(grep -E '^::(error|warning|notice) ' <<< "$output" || true)"

if [[ -n "$findings" ]]; then
  sed -E -e "s#^::(error|warning|notice) file=#::\1 file=${prefix}/#" -e 's/^::error /::warning /' <<< "$findings"
  echo "::warning title=${prefix}: unused dependencies::cargo-shear reported dependency findings in ${prefix}. Run 'cargo shear --fix' there, or list an intentional one under [package.metadata.cargo-shear] ignored."
elif (( status != 0 )); then
  printf '%s\n' "$output"
  echo "::warning title=${prefix}: unused dependencies::cargo-shear could not run in ${prefix} (exit ${status}); see the log above."
else
  echo "${prefix}: no unused dependencies."
fi
exit 0
