#!/usr/bin/env bash
# Local demo harness driver — registry-free.
#
# The migration workers aren't published
# to registry/index.json yet, so `iii worker add <name>` can't find them. But
# every worker is just an iii-sdk client that connects to the engine and
# registers its functions on the bus. We can run them all directly.
#
# Subcommands:
#   build     — `cargo build --release` for each of the 23 workers + harness
#   engine    — start `iii --use-default-config` in background, wait for :49134
#   start     — spawn each worker as a background process; PIDs in $DEMO_DIR/pids/
#   verify    — call harness::status and a couple of dep functions
#   logs <w>  — tail $DEMO_DIR/logs/<worker>.log
#   web       — install web deps if missing, start vite in tmux session
#   stop      — kill every PID in $DEMO_DIR/pids/ AND the engine + web
#   all       — build + engine + start + verify
#
# Usage:
#   ./scripts/demo.sh <cmd> [DEMO_DIR]
#
# DEMO_DIR defaults to ~/iii-harness-demo. Env requires `iii` engine
# reachable on ws://127.0.0.1:49134 (or set III_URL).

set -euo pipefail

CMD="${1:-}"
DEMO_DIR="${2:-$HOME/iii-harness-demo}"
WORKERS_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HARNESS_DIR="$WORKERS_REPO/harness"

WORKERS=(
  turn-orchestrator provider-router
  session-tree session-inbox
  models-catalog hook-fanout policy-denylist
  shell-bash shell-filesystem subagent
  provider-anthropic provider-openai
  auth-credentials llm-budget
  skills
)

ensure_dirs() {
  mkdir -p "$DEMO_DIR/pids" "$DEMO_DIR/logs"
}

# Most workers ship a binary named `iii-{worker}`; `skills` keeps the
# unprefixed `skills` name from before the convention. Centralize the lookup
# so the build/spawn paths stay consistent.
bin_name_for() {
  case "$1" in
    skills) echo "skills" ;;
    *) echo "iii-$1" ;;
  esac
}

cmd_build() {
  ensure_dirs
  echo "==> building harness + ${#WORKERS[@]} dep workers (release)..."
  for w in "${WORKERS[@]}" harness; do
    echo "    [build] $w"
    (cd "$WORKERS_REPO/$w" && cargo build --release --bin "$(bin_name_for "$w")" --quiet)
  done
  echo "==> all binaries built."
}

spawn_one() {
  local w="$1"
  local bin="$WORKERS_REPO/$w/target/release/$(bin_name_for "$w")"
  local pidfile="$DEMO_DIR/pids/$w.pid"
  local logfile="$DEMO_DIR/logs/$w.log"

  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "    [skip]  $w (already running, pid $(cat "$pidfile"))"
    return 0
  fi
  if [[ ! -x "$bin" ]]; then
    echo "    [error] $w binary missing — run \`./scripts/demo.sh build\` first"
    return 1
  fi

  : > "$logfile"
  nohup "$bin" >>"$logfile" 2>&1 &
  echo $! > "$pidfile"
  echo "    [start] $w pid=$! log=$logfile"
}

cmd_engine() {
  ensure_dirs
  command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
  local pidfile="$DEMO_DIR/engine.pid"
  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "==> engine already running (pid $(cat "$pidfile"))"
    return 0
  fi
  echo "==> starting iii engine..."
  (cd "$DEMO_DIR" && nohup iii --use-default-config > engine.log 2>&1 &
   echo $! > engine.pid)
  for i in 1 2 3 4 5 6 7 8 9 10; do
    sleep 1
    if iii --use-default-config trigger --function-id engine::queue::list_topics --timeout-ms 1000 >/dev/null 2>&1; then
      echo "==> engine ready after ${i}s (pid $(cat "$pidfile"))"
      return 0
    fi
  done
  echo "==> engine did not become ready in 10s; tail $DEMO_DIR/engine.log"
  exit 1
}

cmd_start() {
  ensure_dirs
  command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
  echo "==> engine reachability check..."
  if ! iii --use-default-config trigger --function-id engine::queue::list_topics --timeout-ms 1000 >/dev/null 2>&1; then
    echo "    engine not reachable — run \`./scripts/demo.sh engine\` first (or use \`all\`)"; exit 1
  fi
  echo "==> spawning ${#WORKERS[@]} dep workers..."
  for w in "${WORKERS[@]}"; do
    spawn_one "$w"
    sleep 0.1
  done
  echo "==> spawning harness..."
  spawn_one harness
  echo "==> waiting 2s for registrations to settle..."
  sleep 2
  echo "==> done. PIDs in $DEMO_DIR/pids/  Logs in $DEMO_DIR/logs/"
}

cmd_verify() {
  command -v iii >/dev/null || { echo "iii CLI not found"; exit 1; }
  echo "==> harness::status"
  iii --use-default-config trigger --function-id harness::status
  echo
  echo "==> models::list (proves models-catalog connected)"
  iii --use-default-config trigger --function-id models::list || true
}

cmd_logs() {
  local w="${2:-harness}"
  exec tail -f "$DEMO_DIR/logs/$w.log"
}

cmd_web() {
  command -v npm >/dev/null || { echo "npm not found; install Node.js first"; exit 1; }
  command -v tmux >/dev/null || { echo "tmux not found; install tmux"; exit 1; }
  local web_dir="$WORKERS_REPO/harness/web"
  if [[ ! -d "$web_dir/node_modules" ]]; then
    echo "==> installing web deps..."
    (cd "$web_dir" && npm install --no-fund --no-audit)
  fi
  if tmux has-session -t harness-web 2>/dev/null; then
    echo "==> harness-web tmux session already running. Attach with: tmux attach -t harness-web"
    return 0
  fi
  echo "==> starting vite (proxies /bridge → http://127.0.0.1:3111)"
  tmux new-session -d -s harness-web -c "$web_dir" "npm run dev"
  echo "==> http://localhost:5173    (logs: tmux attach -t harness-web)"
}

cmd_stop() {
  if [[ ! -d "$DEMO_DIR/pids" ]]; then
    echo "no pid dir at $DEMO_DIR/pids; nothing to stop"
    return 0
  fi
  echo "==> stopping spawned processes..."
  for pidfile in "$DEMO_DIR/pids"/*.pid; do
    [[ -f "$pidfile" ]] || continue
    local w pid
    w="$(basename "$pidfile" .pid)"
    pid="$(cat "$pidfile")"
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null && echo "    [stop]  $w pid=$pid"
    else
      echo "    [gone]  $w pid=$pid"
    fi
    rm -f "$pidfile"
  done
  if [[ -f "$DEMO_DIR/engine.pid" ]]; then
    local epid
    epid="$(cat "$DEMO_DIR/engine.pid")"
    if kill -0 "$epid" 2>/dev/null; then
      kill -TERM "$epid" 2>/dev/null && echo "    [stop]  engine pid=$epid"
    fi
    rm -f "$DEMO_DIR/engine.pid"
  fi
  if command -v tmux >/dev/null && tmux has-session -t harness-web 2>/dev/null; then
    tmux kill-session -t harness-web && echo "    [stop]  harness-web tmux"
  fi
  echo "==> done."
}

cmd_all() {
  cmd_build
  cmd_engine
  cmd_start
  echo
  cmd_verify
}

case "$CMD" in
  build)  cmd_build ;;
  engine) cmd_engine ;;
  start)  cmd_start ;;
  verify) cmd_verify ;;
  logs)   cmd_logs "$@" ;;
  web)    cmd_web ;;
  stop)   cmd_stop ;;
  all)    cmd_all ;;
  *)      sed -n '2,22p' "$0"; exit 2 ;;
esac
