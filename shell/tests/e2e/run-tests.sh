#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Central configuration (v0.4.0+): the engine launches the shell worker with
# --config <temp.yaml> derived from the `shell` config block in config.yaml.
# The shell worker registers that as the SEED with the built-in `configuration`
# worker (configuration::register) and then reads it back (configuration::get).
# The engine's file-backed configuration worker persists entries under
# ./config, so this script clears that generated directory before each run and
# passes the engine a throwaway copy of config.yaml. The checked-in config file
# remains the authoritative source and is not rewritten by the engine.

# Path overrides (defaults assume the harness lives at shell/tests/e2e/ inside
# the workers repo and the iii engine is on $PATH or at $HOME/.local/bin/iii —
# which is where the install script
# `curl -fsSL https://install.iii.dev/iii/main/install.sh | sh` puts it).
WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/shell}"
# The engine resolves binaries by registered worker name (`shell` per
# iii.worker.yaml), not by cargo bin name. If the engine actually looks up
# by binary name, override with WORKER_BIN_LINK=$HOME/.iii/workers/shell.
WORKER_BIN_LINK="${WORKER_BIN_LINK:-$HOME/.iii/workers/shell}"

REPORT_PATH="$ROOT_DIR/reports/report.json"
TS=$(date +%Y%m%d-%H%M%S)
ENGINE_LOG="$ROOT_DIR/reports/engine-$TS.log"
HARNESS_LOG="$ROOT_DIR/reports/harness-$TS.log"
ENGINE_CONFIG="$ROOT_DIR/reports/config-$TS.yaml"
CONFIG_STATE_DIR="$ROOT_DIR/config"
SENTINEL_TIMEOUT="${HARNESS_TIMEOUT:-90}"

KEEP=0
NO_BUILD=0

for arg in "$@"; do
  case "$arg" in
    --keep)     KEEP=1 ;;
    --no-build) NO_BUILD=1 ;;
    -h|--help)
      cat <<EOF
Usage: $0 [--keep] [--no-build]

  --keep     Leave the engine running after the run (for debugging).
  --no-build Skip cargo build of the shell worker.

Env overrides:
  WORKER_SRC          Path to the shell worker crate (default: ../..).
  III_BIN             Path to the iii engine binary.
  WORKER_BIN_TARGET   Path to the built worker binary.
  WORKER_BIN_LINK     Path to the symlink the engine reads.
  HARNESS_TIMEOUT     Seconds to wait for the harness sentinel (default: 90).
                      Bump this on a fresh dev machine: cases-exec-sandbox.ts
                      drives the real iii-sandbox worker, and the first
                      sandbox::create cold-pulls the python image. Try
                      HARNESS_TIMEOUT=300 if you see "did not emit sentinel
                      within 90s" on initial runs.
EOF
      exit 0
      ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

ENGINE_PID=""
HARNESS_PID=""
cleanup() {
  local code=$?
  if [[ -n "$HARNESS_PID" ]] && kill -0 "$HARNESS_PID" 2>/dev/null; then
    kill "$HARNESS_PID" 2>/dev/null || true
    wait "$HARNESS_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP" -eq 0 ]] && [[ -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    kill "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

# Reap orphaned processes from previous runs. If a prior engine
# crashed (e.g., port-conflict panic) the trap above never fired, so
# its workers stayed alive and re-register against the next engine —
# which then routes calls to the orphan instead of the freshly-spawned
# worker, producing baffling test failures (wrong denylist, wrong
# sandbox.enabled). Kill them before starting clean.
reap_orphans() {
  local port_pid
  port_pid=$(lsof -tiTCP:49134 -sTCP:LISTEN 2>/dev/null || true)
  if [[ -n "$port_pid" ]]; then
    echo "[run-tests] reaping stale engine on port 49134 (pid=$port_pid)"
    kill -9 $port_pid 2>/dev/null || true
  fi
  # Stale shell + iii-worker sandbox-daemon survivors from previous
  # shell test runs. Match narrowly on the test-config path
  # signature so we don't touch unrelated iii processes the user has
  # going. -f matches against the full command line.
  pkill -f "shell --config /var/folders" 2>/dev/null || true
  pkill -f "iii-worker sandbox-daemon --config /var/folders" 2>/dev/null || true
  sleep 0.5
}
reap_orphans

mkdir -p "$ROOT_DIR/reports" "$ROOT_DIR/data" "$(dirname "$WORKER_BIN_LINK")"

# 1. Build the worker (unless --no-build)
if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "[run-tests] cargo build --release (shell worker)"
  (cd "$WORKER_SRC" && cargo build --release --bin shell)
fi
if [[ ! -x "$WORKER_BIN_TARGET" ]]; then
  echo "[run-tests] FATAL: worker binary missing at $WORKER_BIN_TARGET — run without --no-build" >&2
  exit 1
fi

# 2. Symlink at $WORKER_BIN_LINK
if [[ ! -L "$WORKER_BIN_LINK" || "$(readlink "$WORKER_BIN_LINK")" != "$WORKER_BIN_TARGET" ]]; then
  ln -sfn "$WORKER_BIN_TARGET" "$WORKER_BIN_LINK"
  echo "[run-tests] symlink: $WORKER_BIN_LINK -> $WORKER_BIN_TARGET"
fi

# 3. Verify engine binary
if [[ ! -x "$III_BIN" ]]; then
  echo "[run-tests] FATAL: iii engine binary missing at $III_BIN" >&2
  echo "[run-tests] install with: curl -fsSL https://install.iii.dev/iii/main/install.sh | sh" >&2
  exit 1
fi

# 4. Install harness deps if needed
if [[ ! -d "$ROOT_DIR/workers/harness/node_modules" ]]; then
  echo "[run-tests] npm ci (harness)"
  (cd "$ROOT_DIR/workers/harness" && npm ci --silent)
fi

# 5. Set env vars used by the env-scrubbing/passthrough cases. HARNESS_TEST_VAR
# is in env.allow — should round-trip. HARNESS_NOT_ALLOWED is not — should be
# scrubbed before reaching the spawned `env` command.
export HARNESS_TEST_VAR="harness-allowed-value"
export HARNESS_NOT_ALLOWED="harness-blocked-value"

# 6. Start the engine
echo "[run-tests] starting iii engine"
: > "$ENGINE_LOG"
: > "$HARNESS_LOG"
rm -rf "$CONFIG_STATE_DIR"
cp "$ROOT_DIR/config.yaml" "$ENGINE_CONFIG"

( cd "$ROOT_DIR" && "$III_BIN" --no-update-check -c "$ENGINE_CONFIG" ) > "$ENGINE_LOG" 2>&1 &
ENGINE_PID=$!
echo "[run-tests] engine pid=$ENGINE_PID"

# 7. Wait for the engine to accept TCP on its WebSocket port (49134).
deadline=$(( $(date +%s) + 30 ))
while :; do
  if (echo > /dev/tcp/127.0.0.1/49134) 2>/dev/null; then
    break
  fi
  if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
    echo "[run-tests] FATAL: engine exited before binding port; tail of engine log:" >&2
    tail -40 "$ENGINE_LOG" >&2
    exit 1
  fi
  if (( $(date +%s) > deadline )); then
    echo "[run-tests] FATAL: engine did not bind port 49134 within 30s; tail of engine log:" >&2
    tail -40 "$ENGINE_LOG" >&2
    exit 1
  fi
  sleep 0.5
done
echo "[run-tests] engine listening"

# 8. Launch the harness as a host node process
echo "[run-tests] starting harness"
( cd "$ROOT_DIR/workers/harness" && \
  env III_URL=ws://127.0.0.1:49134 \
      HARNESS_REPORT_PATH="$REPORT_PATH" \
      HARNESS_TEST_VAR="$HARNESS_TEST_VAR" \
      HARNESS_NOT_ALLOWED="$HARNESS_NOT_ALLOWED" \
      npm run --silent dev ) > "$HARNESS_LOG" 2>&1 &
HARNESS_PID=$!
echo "[run-tests] harness pid=$HARNESS_PID"

# 9. Wait for sentinel line
sentinel=""
deadline=$(( $(date +%s) + SENTINEL_TIMEOUT ))
while (( $(date +%s) < deadline )); do
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    if grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG" >/dev/null 2>&1; then
      sentinel=$(grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG")
      break
    fi
    echo "[run-tests] harness exited without sentinel; tail of harness log:" >&2
    tail -40 "$HARNESS_LOG" >&2
    exit 1
  fi
  if grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG" >/dev/null 2>&1; then
    sentinel=$(grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG")
    break
  fi
  sleep 1
done

if [[ -z "$sentinel" ]]; then
  echo "[run-tests] FATAL: harness did not emit sentinel within ${SENTINEL_TIMEOUT}s" >&2
  echo "[run-tests] tail of harness log:" >&2
  tail -40 "$HARNESS_LOG" >&2
  exit 1
fi

echo
echo "======================================================================="
echo "$sentinel"
if [[ -f "$REPORT_PATH" ]]; then
  python3 - "$REPORT_PATH" <<'PY' 2>/dev/null || cat "$REPORT_PATH"
import json, sys
data = json.load(open(sys.argv[1]))
for r in data["results"]:
    tag = "PASS" if r["status"] == "PASS" else "FAIL"
    err = (" — " + r.get("error","")) if r["status"] == "FAIL" else ""
    print(f"  [{tag}] {r['case']}{err}")
PY
fi
echo "======================================================================="

case "$sentinel" in
  *PASS*) exit 0 ;;
  *)      exit 1 ;;
esac
