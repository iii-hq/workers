#!/usr/bin/env bash
# Pre-commit hook for the iii workers monorepo. Runs iii-skill-check on staged
# worker dirs before the commit lands.
#
# Install with:
#   ln -s ../../.github/scripts/skill-check-pre-commit.sh .git/hooks/pre-commit
#
# Skips the AI layer (slow, costs API tokens, and requires ANTHROPIC_API_KEY).
# CI runs the AI layer on every PR — see .github/workflows/skill-check.yml.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

# Find which staged top-level dirs contain an iii.worker.yaml. Portable loop
# (no `mapfile` — works on bash 3.2 / macOS).
staged_workers=""
prev_dir=""
while IFS= read -r path; do
    dir="${path%%/*}"
    [ "$dir" = "$prev_dir" ] && continue
    prev_dir="$dir"
    if [ -f "$dir/iii.worker.yaml" ] && [ -d "$dir/docs" ]; then
        staged_workers+="$dir"$'\n'
    fi
done < <(git diff --cached --name-only | sort -u)

if [ -z "$staged_workers" ]; then
    exit 0
fi

# Build the validator (cached after first run via cargo's incremental build).
cargo build --manifest-path iii-skill-check/Cargo.toml --quiet

bin="iii-skill-check/target/debug/iii-skill-check"
fail=0

while IFS= read -r worker; do
    [ -z "$worker" ] && continue
    echo "iii-skill-check: $worker"
    if ! "$bin" verify-rendered "$worker"; then
        fail=1
    fi
    if ! "$bin" verify "$worker" --layers structure,vale; then
        fail=1
    fi
done <<< "$staged_workers"

if [ "$fail" -ne 0 ]; then
    echo
    echo "iii-skill-check failed. Re-run on the affected worker:"
    echo "  iii-skill-check render --write <worker>     # if rendered files drifted"
    echo "  iii-skill-check verify <worker>             # to see all layer violations"
    exit 1
fi
