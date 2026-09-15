#!/usr/bin/env bash
# Fetch upstream instructions without replacing the local-source Compose stack.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage: ./sync.sh [--ref REF] [--repo URL_OR_PATH] [--dry-run | --stack-stopped] [--force]

Fetch agents and skills from iii-hq/templates (main by default). Record the
resolved commit in upstream/sync.json; keep any upstream configuration under
upstream/config/ for review only. Local config/, Compose, environment files
and runtime data are never replaced. No upstream scripts are run.

  --ref REF       Branch, tag, commit, or pull request ref (default: main)
  --repo SOURCE   Git URL or local repository (default: iii-hq/templates)
  --dry-run       Fetch and validate, but do not replace generated files
  --stack-stopped Confirm the stack/readers are stopped before replacing files
                  (required for writes; not an automatic runtime-state check)
  --force         Discard edits in agents/, skills/ and upstream/ only
                  (the normal mode refuses to overwrite edits)
  --help          Show this help

Requires Bash, Git and Python 3.11+ (standard library only).
Run from any working directory.
HELP
}

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo=https://github.com/iii-hq/templates.git
ref=main
# Bash 3.2 treats empty arrays as unset under set -u; keep a required argument.
options=(--destination "$script_dir")
preview=false
stack_stopped=false
while (($#)); do
  case "$1" in
    --ref|--repo)
      (($# >= 2)) && [[ -n "$2" && "$2" != -* ]] || {
        echo "Missing value for $1" >&2; exit 2;
      }
      if [[ "$1" == --ref ]]; then ref=$2; else repo=$2; fi
      shift 2 ;;
    --dry-run) preview=true; options+=("$1"); shift ;;
    --stack-stopped) stack_stopped=true; options+=("$1"); shift ;;
    --force) options+=("$1"); shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# Reject writes before fetching unless the operator confirms readers are stopped.
if [[ "$preview" == false && "$stack_stopped" == false ]]; then
  echo 'Stop the template stack first, then run ./sync.sh --stack-stopped (or use --dry-run).' >&2
  exit 2
fi

command -v git >/dev/null || { echo 'Git is required.' >&2; exit 1; }
python3 -c 'import sys; assert sys.version_info >= (3, 11)' 2>/dev/null || {
  echo 'Python 3.11+ is required.' >&2; exit 1;
}

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
python3 "$script_dir/scripts/sync_template.py" \
  --checkout "$scratch/repo" --commit "$commit" --repo "$repo" --ref "$ref" \
  "${options[@]}"
