#!/usr/bin/env bash
# Download a template folder verbatim. Do not configure or run the project.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage: ./sync.sh [--template NAME] [--ref REF] [--repo URL_OR_PATH] [--dry-run]

Download all contents of iii/NAME/ into the folder NAME beside sync.sh.
The default template is harness. Downloaded folders are ignored by Git.
Existing files with matching paths are overwritten; local-only files are kept.
No project structure or runtime-state checks, configuration changes or scripts.

  --template NAME Template folder under iii/ (default: harness)
  --ref REF       Branch, tag, commit, or pull request ref (default: main)
  --repo SOURCE   Git URL or local repository (default: iii-hq/templates)
  --dry-run       List files without creating or changing the destination
  --help          Show this help

Requires Bash, Git and Python 3.11+ (standard library only).
Automatically tries python3, then python, using the first compatible interpreter.
Run from any working directory; destinations are always beside sync.sh.
HELP
}

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo=https://github.com/iii-hq/templates.git
ref=main
template=harness
# Bash 3.2 treats empty arrays as unset under set -u; keep a required argument.
options=(--root "$script_dir")
while (($#)); do
  case "$1" in
    --ref|--repo|--template)
      (($# >= 2)) && [[ -n "$2" && "$2" != -* ]] || {
        echo "Missing value for $1" >&2; exit 2;
      }
      case "$1" in
        --ref) ref=$2 ;;
        --repo) repo=$2 ;;
        --template) template=$2 ;;
      esac
      shift 2 ;;
    --dry-run) options+=("$1"); shift ;;
    # Accepted for compatibility; neither flag is needed or has any effect.
    --stack-stopped|--force) shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# Restrict the destination to a sandbox beside sync.sh, not the tooling itself.
if [[ ! "$template" =~ ^[a-z0-9][a-z0-9_-]*$ ]]; then
  echo 'Invalid template name: use lowercase letters, digits, underscores and hyphens.' >&2
  exit 2
fi
case "$template" in
  agents|config|data|scripts|skills|tests|upstream)
    echo "Reserved template name: $template" >&2; exit 2 ;;
esac
options+=(--template "$template")

importer="$script_dir/scripts/sync_template.py"
if [[ ! -f "$importer" || ! -r "$importer" ]]; then
  echo "Sync importer is missing or unreadable: $importer" >&2
  echo 'Restore template/scripts/sync_template.py from this checkout; keep sync.sh and scripts/ together.' >&2
  exit 1
fi
command -v git >/dev/null || { echo 'Git is required.' >&2; exit 1; }
python_bin=
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1 && \
      "$candidate" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
    python_bin=$candidate
    break
  fi
done
if [[ -z "$python_bin" ]]; then
  echo 'Python 3.11+ is required. Neither python3 nor python is available with a compatible version.' >&2
  exit 1
fi

# This lock only prevents concurrent downloads; it is not a project-state check.
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
scratch=$(mktemp -d "${TMPDIR:-/tmp}/template-sync.XXXXXX")

git init --quiet "$scratch/repo"
GIT_TERMINAL_PROMPT=0 git -C "$scratch/repo" -c protocol.ext.allow=never \
  fetch --quiet --depth=1 --no-tags -- "$repo" "$ref"
commit=$(git -C "$scratch/repo" rev-parse --verify 'FETCH_HEAD^{commit}')
"$python_bin" "$importer" --checkout "$scratch/repo" --commit "$commit" "${options[@]}"
