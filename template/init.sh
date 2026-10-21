#!/usr/bin/env bash
# Recreate template/NAME/ with `iii project init`, then start it with `iii compose --up`.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage: ./init.sh <template>

Start template/<template>/ as a brand-new `iii project init -t <template>`
project that runs this checkout's workers, then keep it up (iii compose --up).

Examples:
  ./init.sh quickstart    the cross-language quickstart tutorial
  ./init.sh harness       the ADE agent workspace; opens the setup wizard
  ./init.sh nope          unknown names fail and list every template

Every run is a first run:
  - template/<template>/ is replaced: data/, .iii/, .env and Compose edits are lost
  - a stack still running there is left alone; stop it with Ctrl+C first
  - a failed init keeps the previous folder

Workers and ports:
  - package:// workers found in this checkout run from source (cargo run)
  - ADE and http keep their default ports 3113 and 3111 when free, else take the
    next free one; the engine always gets a free port from 49334, so the root
    stack and other sandboxes keep running; the run prints them

Environment:
  TEMPLATES_REPO=<dir>      a local templates checkout, uncommitted changes included
  III_TEMPLATE_URL_BRANCH   another branch of iii-hq/templates (default: main)
  III_TEMPLATE_URL          another templates repository

Requires iii, git, and python3 with ruamel.yaml (or uv).
HELP
}

(($# == 1)) || { usage >&2; exit 2; }
case "$1" in --help|-h) usage; exit 0 ;; esac
name=$1
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)

# Nothing is deleted until the name and the CLI are valid and init has succeeded.
if [[ ! "$name" =~ ^[a-z0-9][a-z0-9_-]*$ ]]; then
  echo 'Invalid template name: use lowercase letters, digits, underscores and hyphens.' >&2
  exit 2
fi
case "$name" in
  agents|config|data|scripts|skills|tests|upstream)
    echo "Reserved template name: $name" >&2; exit 2 ;;
esac
command -v iii >/dev/null || { echo 'iii is required.' >&2; exit 1; }
# Python only edits the Compose file; uv supplies ruamel.yaml when python3 lacks it.
python=(python3)
python3 -c 'import ruamel.yaml' >/dev/null 2>&1 || python=(uv run --quiet --script)
command -v "${python[0]}" >/dev/null || {
  echo 'python3 with ruamel.yaml, or uv, is required.' >&2; exit 1;
}

destination=$script_dir/$name
# Compose keeps its PID in engine.lock while it runs and empties it on exit.
for lock in "$destination"/.iii/compose/*/engine.lock; do
  pid=$(cat -- "$lock" 2>/dev/null) || continue
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    echo "$name is still running (pid $pid). Stop it with Ctrl+C first." >&2
    exit 1
  fi
done

stage=$(mktemp -d "$script_dir/.init-stage-XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
init=(iii project init -t "$name" -d "$stage/$name" --skip-iii)
[[ -z "${TEMPLATES_REPO:-}" ]] || init+=(--template-dir "$TEMPLATES_REPO/iii")
echo "Creating template/$name from ${TEMPLATES_REPO:-${III_TEMPLATE_URL:-iii-hq/templates}}..."
# The CLI's own next steps point at the staging folder; show its output only on failure.
if ! output=$("${init[@]}" </dev/null 2>&1); then
  printf '%s\n' "$output" >&2
  exit 1
fi
rm -rf -- "$destination"
mv -- "$stage/$name" "$destination"
rmdir -- "$stage"
trap - EXIT

# After the move: local worker paths are relative to the final folder.
"${python[@]}" "$script_dir/scripts/init_compose.py" "$destination"

cd -- "$destination"
exec iii compose --up
