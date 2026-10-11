#!/usr/bin/env bash
# Isolated bench stack on :49250 — never touches the live engine (:49134).
# Usage: RUN=/abs/dir/outside/any/repo ANTHROPIC_API_KEY=... context-clm/bench/boot.sh
set -euo pipefail
WT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)  # this checkout: harness, context-clm, arms
MAIN=${MAIN:-/home/anderson/project/iii/workers}          # unchanged workers' debug builds
RUN=${RUN:?set RUN to a directory outside any git repo, e.g. /home/anderson/context-clm-runs/bench-01}
case $RUN in /*) ;; *) echo "RUN must be an absolute path" >&2; exit 1 ;; esac
PORT=49250
III=${III_BIN:-$HOME/.local/bin/iii}
: "${ANTHROPIC_API_KEY:?export ANTHROPIC_API_KEY (provider-anthropic reads it)}"
export III_URL=ws://127.0.0.1:$PORT III_TELEMETRY_ENABLED=false NO_COLOR=1  # NO_COLOR: log scans match plain text (ruling C7)
unset III_COMPOSE_DIR III_COMPOSE_NAMESPACE III_NAMESPACE III_CONFIG_NAME III_COMPOSE_FILE III_ENGINE_PORT

if [ -s "$RUN/pids" ]; then echo "$RUN/pids exists: run down.sh first" >&2; exit 1; fi
mkdir -p "$RUN"/{bin,logs,workspace,configuration,sessions,queue,context-leases}
for w in state queue session-manager context-manager cron llm-router provider-anthropic ide; do
  [ -x "$MAIN/$w/target/debug/$w" ] || { echo "build $MAIN/$w first: cargo build --locked" >&2; exit 1; }
  cp "$MAIN/$w/target/debug/$w" "$RUN/bin/"
done
for b in harness/target/release/harness context-clm/target/release/context-clm; do
  [ -x "$WT/$b" ] || { echo "build $WT/$b first: cargo build --release --locked" >&2; exit 1; }
  cp "$WT/$b" "$RUN/bin/"
done
[ -d "$WT/context-clm/bench/arms/node_modules" ] || { echo "run: (cd $WT/context-clm/bench/arms && pnpm install --frozen-lockfile)" >&2; exit 1; }

cat >"$RUN/engine.yaml" <<EOF
workers:
  - name: iii-worker-manager
    config: { host: 127.0.0.1, port: $PORT }
  - name: configuration
    config: { adapter: { name: fs, config: { directory: $RUN/configuration } } }
EOF
printf 'adapter:\n  name: kv\n  config:\n    store_method: in_memory\n' >"$RUN/state.yaml"
printf 'adapter:\n  name: builtin\n  config:\n    store_method: file_based\n    file_path: %s/queue\n    save_interval_ms: 1000\n' "$RUN" >"$RUN/queue.yaml"
printf 'adapter:\n  name: fs\n  config:\n    data_dir: %s/sessions\n' "$RUN" >"$RUN/session-manager.yaml"
printf 'lease_dir: %s/context-leases\n' "$RUN" >"$RUN/context-manager.yaml"
# Execs inherit only PATH/HOME/LANG/LC_ALL/TERM (no API keys) and are Landlock-confined to their scope root plus
# temp, devices and the Rust toolchain caches; timeouts fit a cold cargo build.
cat >"$RUN/ide.yaml" <<EOF
working_dir: $RUN/workspace
max_timeout_ms: 600000
default_timeout_ms: 300000
fs:
  host_roots: [$RUN/workspace]
  exec_confinement: landlock
  exec_writable: [/tmp, /dev, ~/.cargo, ~/.rustup]
EOF
cat >"$RUN/harness.yaml" <<EOF
filesystem_boundary: workspace
default_filesystem_root: $RUN/workspace
EOF

: >"$RUN/pids"
# A boot that fails half-way stops what it already started.
trap '"$(dirname "$0")/down.sh"' ERR
start() { local n=$1; shift; (cd "$RUN" && exec "$@") >"$RUN/logs/$n.log" 2>&1 & echo $! >>"$RUN/pids"; }
waitfn() {
  for _ in $(seq 240); do
    "$III" trigger engine::functions::list --engine "$III_URL" --json '{"include_internal":true}' 2>/dev/null |
      jq -e --arg id "$1" '.functions[]? | select(.function_id == $id)' >/dev/null && return 0
    sleep 0.5
  done
  echo "timeout waiting for $1" >&2; return 1
}

start engine "$III" -c "$RUN/engine.yaml" --no-update-check
for _ in $(seq 120); do "$III" trigger engine::workers::list --engine "$III_URL" >/dev/null 2>&1 && break; sleep 0.5; done
start state "$RUN/bin/state" --url "$III_URL" --config "$RUN/state.yaml"; waitfn state::get
start queue "$RUN/bin/queue" --url "$III_URL" --config "$RUN/queue.yaml"; waitfn queue::define
start session-manager "$RUN/bin/session-manager" --url "$III_URL" --config "$RUN/session-manager.yaml"; waitfn session::messages
start llm-router "$RUN/bin/llm-router" --url "$III_URL"; waitfn router::models::get
start provider-anthropic "$RUN/bin/provider-anthropic" --url "$III_URL"; waitfn provider::anthropic::stream
for _ in $(seq 240); do
  "$III" trigger router::models::get --engine "$III_URL" --json '{"provider":"anthropic","id":"claude-sonnet-5-5"}' 2>/dev/null |
    jq -e '.model.id' >/dev/null && break; sleep 0.5
done
start context-manager "$RUN/bin/context-manager" --url "$III_URL" --config "$RUN/context-manager.yaml"; waitfn context::assemble
start cron "$RUN/bin/cron" --url "$III_URL"
start ide "$RUN/bin/ide" --url "$III_URL" --config "$RUN/ide.yaml"; waitfn shell::exec
start context-clm "$RUN/bin/context-clm" --url "$III_URL"; waitfn context-clm::assemble
start context-arms env ARMS_DIR="$RUN/arms" node "$WT/context-clm/bench/arms/src/index.ts"
waitfn context-sum::assemble; waitfn context-base::assemble
start harness "$RUN/bin/harness" --url "$III_URL" --config "$RUN/harness.yaml"; waitfn harness::send
echo "bench stack up on :$PORT (run dir $RUN)"
