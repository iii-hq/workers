#!/usr/bin/env bash
set -euo pipefail

# End-to-end harness for the rbac-proxy worker. Boots a real iii engine, starts
# the rbac-proxy binary as a host process (with a --config seed wiring auth /
# middleware / hooks / expose to the harness support functions), then runs a
# Node harness that registers those support functions on the engine and drives
# a downstream worker THROUGH the proxy port — asserting the RBAC contract
# end-to-end.
#
# Mirrors database/tests/e2e/run-tests.sh, minus the docker-compose stack
# (rbac-proxy has no external dependencies — only the engine + configuration
# worker, which the engine enables by default).

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/rbac-proxy}"

ENGINE_PORT="${ENGINE_PORT:-49134}"
PROXY_PORT="${PROXY_PORT:-49271}"
ENGINE_WS="ws://127.0.0.1:${ENGINE_PORT}"
PROXY_WS="ws://127.0.0.1:${PROXY_PORT}"

REPORT_PATH="$ROOT_DIR/reports/report.json"
TS=$(date +%Y%m%d-%H%M%S)
ENGINE_LOG="$ROOT_DIR/reports/engine-$TS.log"
PROXY_LOG="$ROOT_DIR/reports/proxy-$TS.log"
HARNESS_LOG="$ROOT_DIR/reports/harness-$TS.log"
SEED_FILE="$ROOT_DIR/data/seed.yaml"
SENTINEL_TIMEOUT="${HARNESS_TIMEOUT:-120}"

KEEP=0
NO_BUILD=0
FILTER=""

for arg in "$@"; do
  case "$arg" in
    --keep)       KEEP=1 ;;
    --no-build)   NO_BUILD=1 ;;
    --filter=*)   FILTER="${arg#--filter=}" ;;
    -h|--help)
      cat <<EOF
Usage: $0 [--keep] [--no-build] [--filter=<case-name-substring>]

  --keep              Leave the engine + proxy running after the run.
  --no-build          Skip cargo build of the rbac-proxy worker.
  --filter=SUBSTR     Run only cases whose name contains SUBSTR.

Env overrides:
  WORKER_SRC          Path to the rbac-proxy crate (default: ../..).
  III_BIN             Path to the iii engine binary (default: \$(command -v iii)).
  WORKER_BIN_TARGET   Built worker binary (default: \$WORKER_SRC/target/release/rbac-proxy).
  ENGINE_PORT         Engine WS port (default: 49134).
  PROXY_PORT          Public RBAC port the proxy binds (default: 49271).
  HARNESS_TIMEOUT     Seconds to wait for the harness sentinel (default: 120).
EOF
      exit 0
      ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

ENGINE_PID=""
PROXY_PID=""
HARNESS_PID=""
# Kill a backgrounded process and its children. The `iii` engine forks a child
# that outlives its parent pid, so reaping children (`pkill -P`) is required to
# free :49134 for the next run.
kill_tree() {
  local pid="$1"
  [[ -z "$pid" ]] || ! kill -0 "$pid" 2>/dev/null && return 0
  pkill -P "$pid" 2>/dev/null || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
cleanup() {
  local code=$?
  kill_tree "$HARNESS_PID"
  kill_tree "$PROXY_PID"
  kill_tree "$ENGINE_PID"
  [[ "$KEEP" -eq 0 ]] && rm -f "$SEED_FILE"
  exit "$code"
}
trap cleanup EXIT INT TERM

mkdir -p "$ROOT_DIR/reports" "$ROOT_DIR/data"
# Run from the harness root so the engine's relative ./config.yaml and ./data/
# resolve here, and so a backgrounded engine is a direct child (kill_tree can
# reap it) rather than a grandchild of a `( cd … )` subshell.
cd "$ROOT_DIR"

# 1. Build the worker (unless --no-build).
if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "[run-tests] cargo build --release (rbac-proxy worker)"
  (cd "$WORKER_SRC" && cargo build --release --bin rbac-proxy)
fi
if [[ ! -x "$WORKER_BIN_TARGET" ]]; then
  echo "[run-tests] FATAL: worker binary missing at $WORKER_BIN_TARGET — run without --no-build" >&2
  exit 1
fi

# 2. Verify engine binary.
if [[ ! -x "$III_BIN" ]]; then
  echo "[run-tests] FATAL: iii engine binary missing at $III_BIN" >&2
  echo "[run-tests] install with: curl -fsSL https://install.iii.dev/iii/main/install.sh | sh" >&2
  exit 1
fi

# 3. Install harness deps if needed.
if [[ ! -d "$ROOT_DIR/workers/harness/node_modules" ]]; then
  echo "[run-tests] npm install (harness)"
  (cd "$ROOT_DIR/workers/harness" && npm install --silent)
fi

# 4. Start the engine.
echo "[run-tests] starting iii engine"
: > "$ENGINE_LOG"
"$III_BIN" --no-update-check -c ./config.yaml > "$ENGINE_LOG" 2>&1 &
ENGINE_PID=$!
echo "[run-tests] engine pid=$ENGINE_PID"

wait_for_port() {
  local port="$1" name="$2" pid="$3" log="$4" budget="${5:-30}"
  local deadline=$(( $(date +%s) + budget ))
  while :; do
    if (echo > "/dev/tcp/127.0.0.1/$port") 2>/dev/null; then return 0; fi
    if [[ -n "$pid" ]] && ! kill -0 "$pid" 2>/dev/null; then
      echo "[run-tests] FATAL: $name exited before binding :$port; tail of log:" >&2
      tail -40 "$log" >&2
      return 1
    fi
    if (( $(date +%s) > deadline )); then
      echo "[run-tests] FATAL: $name did not bind :$port within ${budget}s; tail of log:" >&2
      tail -40 "$log" >&2
      return 1
    fi
    sleep 0.5
  done
}

wait_for_port "$ENGINE_PORT" "engine" "$ENGINE_PID" "$ENGINE_LOG" 30
echo "[run-tests] engine listening on :$ENGINE_PORT"

# 5. Seed the proxy config (auth/middleware/hooks/expose -> harness support fns)
#    and start the rbac-proxy binary. The proxy installs this as the
#    `rbac-proxy` configuration entry's initial_value on first boot.
cat > "$SEED_FILE" <<EOF
host: 127.0.0.1
port: ${PROXY_PORT}
engine_url: ${ENGINE_WS}
expose_worker_internals: false
middleware_function_id: support::middleware
rbac:
  auth_function_id: support::auth
  on_function_registration_function_id: support::on-fn-reg
  expose_functions:
    - match("api::*")
    - match("engine::functions::*")
    - match("engine::workers::*")
EOF

echo "[run-tests] starting rbac-proxy worker"
: > "$PROXY_LOG"
"$WORKER_BIN_TARGET" --url "$ENGINE_WS" --config "$SEED_FILE" > "$PROXY_LOG" 2>&1 &
PROXY_PID=$!
echo "[run-tests] rbac-proxy pid=$PROXY_PID"

wait_for_port "$PROXY_PORT" "rbac-proxy" "$PROXY_PID" "$PROXY_LOG" 30
echo "[run-tests] rbac-proxy listening on :$PROXY_PORT"

# 6. Run the harness (registers support fns on the engine, drives a downstream
#    worker through the proxy). It streams per-case results and emits a
#    `HARNESS_DONE: PASS|FAIL n/m` sentinel.
echo "[run-tests] starting harness"
: > "$HARNESS_LOG"
HARNESS_ENV=(
  "III_URL=$ENGINE_WS"
  "PROXY_URL=$PROXY_WS"
  "HARNESS_REPORT_PATH=$REPORT_PATH"
)
[[ -n "$FILTER" ]] && HARNESS_ENV+=("HARNESS_FILTER=$FILTER")

( cd "$ROOT_DIR/workers/harness" && env "${HARNESS_ENV[@]}" npm run --silent dev ) > "$HARNESS_LOG" 2>&1 &
HARNESS_PID=$!
echo "[run-tests] harness pid=$HARNESS_PID"

# 7. Wait for the sentinel.
sentinel=""
deadline=$(( $(date +%s) + SENTINEL_TIMEOUT ))
while (( $(date +%s) < deadline )); do
  if grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG" >/dev/null 2>&1; then
    sentinel=$(grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG")
    break
  fi
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    echo "[run-tests] harness exited without sentinel; tail of harness log:" >&2
    tail -60 "$HARNESS_LOG" >&2
    exit 1
  fi
  sleep 1
done

if [[ -z "$sentinel" ]]; then
  echo "[run-tests] FATAL: harness did not emit sentinel within ${SENTINEL_TIMEOUT}s" >&2
  tail -60 "$HARNESS_LOG" >&2
  exit 1
fi

# 8. Print the per-case summary.
echo
echo "======================================================================="
echo "$sentinel"
if [[ -f "$REPORT_PATH" ]]; then
  python3 - "$REPORT_PATH" <<'PY' 2>/dev/null || cat "$REPORT_PATH"
import json, sys
data = json.load(open(sys.argv[1]))
for r in data["results"]:
    tag = "PASS" if r["status"] == "PASS" else "FAIL"
    err = (" — " + r.get("error", "")) if r["status"] == "FAIL" else ""
    print(f"  [{tag}] {r['case']}{err}")
PY
fi
echo "======================================================================="

case "$sentinel" in
  *PASS*) exit 0 ;;
  *)      exit 1 ;;
esac
