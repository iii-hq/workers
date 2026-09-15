#!/usr/bin/env bash
# Fetch upstream instructions without replacing the local-source Compose stack.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage: ./sync.sh [--ref REF] [--repo URL_OR_PATH] [--dry-run] [--force]

Fetch iii/harness from iii-hq/templates (main by default), synchronize agents,
skills and configuration seeds, and record the resolved commit in upstream/.
Local config-overrides are merged last. The local Compose, environment files
and runtime data are never imported or replaced. No upstream scripts are run.

  --ref REF       Branch, tag, commit, or refs/pull/84/head (default: main)
  --repo SOURCE   Git URL or local repository (default: iii-hq/templates)
  --dry-run       Fetch and validate, but do not replace generated files
  --force         Discard edits inside generated agents/, skills/, config/ and
                  upstream/ only (the normal mode refuses to overwrite edits)
  --help          Show this help

Requires Bash, Git and Python 3.11+ with PyYAML 6.0.3, or uv to supply Python
and the pinned dependency automatically. Run from any working directory.
HELP
}

repo=https://github.com/iii-hq/templates.git
ref=main
options=()
while (($#)); do
  case "$1" in
    --ref|--repo)
      (($# >= 2)) && [[ -n "$2" && "$2" != -* ]] || {
        echo "Missing value for $1" >&2; exit 2;
      }
      if [[ "$1" == --ref ]]; then ref=$2; else repo=$2; fi
      shift 2 ;;
    --dry-run|--force) options+=("$1"); shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
command -v git >/dev/null || { echo 'Git is required.' >&2; exit 1; }
if command -v python3 >/dev/null && python3 -c 'import sys, yaml; assert sys.version_info >= (3, 11)' 2>/dev/null; then
  runner=(python3)
elif command -v uv >/dev/null; then
  runner=(uv run --script)
else
  echo 'Install uv, or Python 3.11+ and PyYAML==6.0.3, then run sync.sh again.' >&2
  exit 1
fi

# Prevent concurrent replacements; a crashed process leaves an explicit lock
# to inspect rather than allowing another invocation to race partial output.
lock="$script_dir/.sync.lock"
mkdir -- "$lock" 2>/dev/null || {
  echo "Sync already running (or stale lock): $lock" >&2; exit 1;
}
scratch=
cleanup() {
  [[ -z "$scratch" ]] || rm -rf -- "$scratch"
  rmdir -- "$lock"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
scratch=$(mktemp -d "${TMPDIR:-/tmp}/harness-template-sync.XXXXXX")

git init --quiet "$scratch/repo"
# A fresh repository avoids stale tracking refs and never checks out/runs
# upstream code. Fetch failure leaves the template untouched.
GIT_TERMINAL_PROMPT=0 git -C "$scratch/repo" -c protocol.ext.allow=never \
  fetch --quiet --depth=1 --no-tags -- "$repo" "$ref"
commit=$(git -C "$scratch/repo" rev-parse --verify 'FETCH_HEAD^{commit}')
"${runner[@]}" "$script_dir/scripts/sync_template.py" \
  --checkout "$scratch/repo" --commit "$commit" --repo "$repo" --ref "$ref" \
  --destination "$script_dir" "${options[@]}"
