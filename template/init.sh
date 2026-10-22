#!/usr/bin/env bash
# Recreate template/NAME/ with `iii project init`, then start it with `iii compose --up`.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage: ./init.sh <template> [engine=<iii binary>]
       ./init.sh --list | --help

Start template/<template>/ as a brand-new `iii project init -t <template>`
project that runs this checkout's workers, then keep it up (iii compose --up).

Examples:
  ./init.sh quickstart    the cross-language quickstart tutorial
  ./init.sh harness       the ADE agent workspace; opens the setup wizard
  ./init.sh --list        the available templates, with descriptions
  ./init.sh harness engine=../../iii/target/debug/iii

Options:
  engine=<iii binary>     the iii that runs init and the whole stack
                          (default: iii on PATH); a path is relative to where
                          you run init.sh. Build one with `cargo build -p iii`
                          in the iii repo.

Every run is a first run:
  - template/<template>/ is replaced; its data/, .iii/, .env and edits are lost
  - a stack still running there is left alone; stop it with Ctrl+C first
  - an unknown name lists the templates; a failed init keeps the previous folder

Workers and ports:
  - package:// workers found in this checkout run from source (cargo run); after
    a code change, rebuild one with
    iii trigger compose::restart --json '{"container":"<name>"}'
  - ADE and http keep their default ports 3113 and 3111 when free, else take the
    next free one; the engine always gets a free port from 49334, so the root
    stack and other sandboxes keep running

After it starts:
  - the summary prints the ADE URL, the ports and the local workers
  - run `iii trigger ...` from template/<template>/ to reach this stack
  - Ctrl+C stops it; run init.sh again for another first run
  - template/README.md has the quickstart's local compose::add commands

Environment:
  TEMPLATES_REPO=<dir>      a local templates checkout (uncommitted edits too)
  III_TEMPLATE_URL_BRANCH   another branch of iii-hq/templates (default: main)
  III_TEMPLATE_URL          another templates repository

Requires iii, git, and python3 with ruamel.yaml (or uv).
HELP
}

usage_error() {
  printf '%s\nUsage: ./init.sh <template> [engine=<iii binary>]   (./init.sh --help for more)\n' "$1" >&2
  exit 2
}

(($# >= 1)) || { usage >&2; exit 2; }
list=
case "$1" in
  --help|-h) usage; exit 0 ;;
  --list) (($# == 1)) || usage_error "Unknown argument: $2 (--list takes none)"; list=1 ;;
  -*) usage_error "Unknown option: $1" ;;
esac
name=$1
shift
engine=iii
for arg; do
  case "$arg" in
    engine=) usage_error 'engine= needs an iii binary, for example engine=../../iii/target/debug/iii' ;;
    engine=*) engine=${arg#engine=} ;;
    *) usage_error "Unknown argument: $arg (only engine=<iii binary> may follow the template)" ;;
  esac
done
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
# Python reads the manifests and edits the Compose file; uv supplies ruamel.yaml when python3 lacks it.
python=(python3)
python3 -c 'import ruamel.yaml' >/dev/null 2>&1 || python=(uv run --quiet --script)
command -v "${python[0]}" >/dev/null || {
  echo 'python3 with ruamel.yaml, or uv, is required: install uv (https://docs.astral.sh/uv/) or run `python3 -m pip install ruamel.yaml==0.18.16`.' >&2
  exit 1
}

# One fetch the way `iii project init` fetches (a shallow clone of III_TEMPLATE_URL
# at III_TEMPLATE_URL_BRANCH); the CLI then reads that folder through --template-dir.
fetch_templates() {
  stage=$(mktemp -d "$script_dir/.init-stage-XXXXXX")
  trap 'rm -rf -- "$stage"' EXIT
  if [[ -n "${TEMPLATES_REPO:-}" ]]; then
    templates=$TEMPLATES_REPO/iii
    echo "Using templates from $templates" >&2
    return
  fi
  # shortcut: these defaults copy scaffolder-core's fetcher.rs; update them if the CLI's change.
  local url=${III_TEMPLATE_URL:-https://github.com/iii-hq/templates.git}
  local branch=${III_TEMPLATE_URL_BRANCH:-main}
  echo "Fetching templates from $url ($branch)..." >&2
  GIT_TERMINAL_PROMPT=0 git clone --quiet --depth 1 --branch "$branch" --single-branch -- \
    "$url" "$stage/templates" </dev/null || {
    echo "Could not clone $url at $branch. Check the network, III_TEMPLATE_URL and III_TEMPLATE_URL_BRANCH, or set TEMPLATES_REPO to a local templates checkout." >&2
    exit 1
  }
  templates=$stage/templates/iii
}

if [[ -n "$list" ]]; then
  fetch_templates
  "${python[@]}" "$script_dir/scripts/init_templates.py" "$templates"
  exit
fi

# Nothing is deleted until the name and the CLI are valid and init has succeeded.
[[ "$name" =~ ^[a-z0-9][a-z0-9_-]*$ ]] ||
  usage_error "Invalid template name '$name': use lowercase letters, digits, underscores and hyphens."
case "$name" in
  agents|config|data|scripts|skills|tests|upstream)
    usage_error "Reserved template name '$name': template/$name/ holds the root stack's files, so init.sh never replaces it." ;;
esac
# A builtin or function (engine=echo) has no path, so it is refused like a missing binary.
if ! path=$(command -v -- "$engine") || [[ "$path" != */* ]]; then
  if [[ "$engine" == iii ]]; then
    echo 'iii is not on PATH. Install the iii CLI, or pass engine=<iii binary>.' >&2
  else
    echo "engine=$engine is not an executable from $PWD. Build iii with \`cargo build -p iii\` in the iii repo, or leave engine= out to use iii from PATH." >&2
  fi
  exit 1
fi
# Absolute, because compose runs from the project folder; a bare name found
# through an absolute PATH entry stays as typed.
[[ "$path" == /* && "$engine" != */* ]] ||
  engine=$(cd -- "$(dirname -- "$path")" && pwd -P)/$(basename -- "$path")

destination=$script_dir/$name
# Compose keeps its PID in engine.lock while it runs and empties it on exit.
for lock in "$destination"/.iii/compose/*/engine.lock; do
  pid=$(cat -- "$lock" 2>/dev/null) || continue
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    echo "$name is still running (pid $pid). Stop it with Ctrl+C first." >&2
    exit 1
  fi
done

fetch_templates
# An unknown name lists the templates and exits 2.
"${python[@]}" "$script_dir/scripts/init_templates.py" "$templates" "$name"
# Until the swap below, any failure or Ctrl+C keeps the previous folder.
trap 'rm -rf -- "$stage"; echo "template/$name was left unchanged." >&2' EXIT
init=("$engine" project init -t "$name" -d "$stage/$name" --skip-iii --template-dir "$templates")
echo "Creating template/$name with $engine..."
# The CLI's own next steps point at the staging folder; show its output only on failure.
if ! output=$("${init[@]}" </dev/null 2>&1); then
  printf '%s\n' "$output" >&2
  exit 1
fi
# Never swap in an empty result: the old folder is deleted before the move.
[[ -d "$stage/$name" ]] || { echo "$engine project init exited 0 but created no project; is engine= an iii binary?" >&2; exit 1; }
"${python[@]}" "$script_dir/scripts/init_compose.py" "$stage/$name" "$destination" "$engine"
trap 'rm -rf -- "$stage"' EXIT
rm -rf -- "$destination"
mv -- "$stage/$name" "$destination"
rm -rf -- "$stage"
trap - EXIT

cd -- "$destination"
# Compose starts the engine from its own executable, so this binary runs the whole stack.
exec "$engine" compose --up
