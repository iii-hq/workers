#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Path overrides (set in CI; defaults assume the harness lives at
# database/tests/e2e/ inside the workers repo and the iii engine is on
# $PATH or at $HOME/.local/bin/iii — which is where the install script
# `curl -fsSL https://install.iii.dev/iii/main/install.sh | sh` puts it).
WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/iii-database}"
WORKER_BIN_LINK="${WORKER_BIN_LINK:-$HOME/.iii/workers/iii-database}"

REPORT_PATH="$ROOT_DIR/reports/report.json"
TS=$(date +%Y%m%d-%H%M%S)
ENGINE_LOG="$ROOT_DIR/reports/engine-$TS.log"
HARNESS_LOG="$ROOT_DIR/reports/harness-$TS.log"
SENTINEL_TIMEOUT="${HARNESS_TIMEOUT:-180}"
HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-60}"

KEEP=0
NO_BUILD=0
FILTER=""

for arg in "$@"; do
  case "$arg" in
    --keep)      KEEP=1 ;;
    --no-build)  NO_BUILD=1 ;;
    --filter=*)  FILTER="${arg#--filter=}" ;;
    -h|--help)
      cat <<EOF
Usage: $0 [--keep] [--no-build] [--filter=<sqlite_db|pg_db|mysql_db>]

  --keep        Leave docker compose stack running after the run.
  --no-build    Skip cargo build of the iii-database worker.
  --filter=KEY  Run only one driver (default: all 3).

Env overrides:
  WORKER_SRC          Path to the database worker crate (default: ../..).
  III_BIN             Path to the iii engine binary (default: \$(command -v iii) or \$HOME/.local/bin/iii).
  WORKER_BIN_TARGET   Path to the built worker binary (default: \$WORKER_SRC/target/release/iii-database).
  WORKER_BIN_LINK     Path to the symlink the engine reads (default: \$HOME/.iii/workers/iii-database).
  HARNESS_TIMEOUT     Seconds to wait for the harness sentinel (default: 180).
  HEALTH_TIMEOUT      Seconds to wait for postgres/mysql healthchecks (default: 60).
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
  if [[ -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    kill "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP" -eq 0 ]]; then
    (cd "$ROOT_DIR" && docker compose down -v >/dev/null 2>&1) || true
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

mkdir -p "$ROOT_DIR/reports" "$ROOT_DIR/data" "$(dirname "$WORKER_BIN_LINK")"

# 1. Ensure binary symlink at $WORKER_BIN_LINK
if [[ ! -L "$WORKER_BIN_LINK" || "$(readlink "$WORKER_BIN_LINK")" != "$WORKER_BIN_TARGET" ]]; then
  ln -sfn "$WORKER_BIN_TARGET" "$WORKER_BIN_LINK"
  echo "[run-tests] symlink: $WORKER_BIN_LINK -> $WORKER_BIN_TARGET"
fi

# 2. Build the worker (unless --no-build)
if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "[run-tests] cargo build --release (iii-database worker)"
  (cd "$WORKER_SRC" && cargo build --release --bin iii-database)
fi
if [[ ! -x "$WORKER_BIN_TARGET" ]]; then
  echo "[run-tests] FATAL: worker binary missing at $WORKER_BIN_TARGET — run without --no-build" >&2
  exit 1
fi

# 3. Verify engine binary
if [[ ! -x "$III_BIN" ]]; then
  echo "[run-tests] FATAL: iii engine binary missing at $III_BIN" >&2
  echo "[run-tests] install with: curl -fsSL https://install.iii.dev/iii/main/install.sh | sh" >&2
  exit 1
fi

# 4. Bring up postgres + mysql and wait for healthchecks via compose's --wait
# (compose v2 native; exits non-zero if any service fails to become healthy
# within HEALTH_TIMEOUT). Beats parsing `compose ps --format json` with regex.
echo "[run-tests] docker compose up -d --wait (timeout=${HEALTH_TIMEOUT}s)"
if ! (cd "$ROOT_DIR" && docker compose up -d --wait --wait-timeout "$HEALTH_TIMEOUT"); then
  echo "[run-tests] FATAL: services did not become healthy within ${HEALTH_TIMEOUT}s" >&2
  (cd "$ROOT_DIR" && docker compose logs --tail 40) >&2
  exit 1
fi
echo "[run-tests] both services healthy"

# 6. Reset SQLite file
rm -f "$ROOT_DIR/data/test.sqlite"

# 7. Install harness deps if needed
if [[ ! -d "$ROOT_DIR/workers/harness/node_modules" ]]; then
  echo "[run-tests] npm install (harness)"
  (cd "$ROOT_DIR/workers/harness" && npm install --silent)
fi

# 8. Start the engine (default config: ./config.yaml)
echo "[run-tests] starting iii engine"
: > "$ENGINE_LOG"
: > "$HARNESS_LOG"

( cd "$ROOT_DIR" && "$III_BIN" --no-update-check -c ./config.yaml ) > "$ENGINE_LOG" 2>&1 &
ENGINE_PID=$!
echo "[run-tests] engine pid=$ENGINE_PID"

# 9. Wait for the engine to accept TCP on its WebSocket port (49134).
# Probing the port directly instead of grepping for an engine log line
# decouples this script from the engine's logging format — a quiet log
# refactor upstream used to silently break us as a 30s timeout.
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

# 10. Launch the harness as a host node process
echo "[run-tests] starting harness"
HARNESS_ENV=()
if [[ -n "$FILTER" ]]; then
  HARNESS_ENV+=("HARNESS_FILTER=$FILTER")
fi
HARNESS_ENV+=("III_URL=ws://127.0.0.1:49134")
HARNESS_ENV+=("HARNESS_REPORT_PATH=$REPORT_PATH")

( cd "$ROOT_DIR/workers/harness" && env "${HARNESS_ENV[@]}" npm run --silent dev ) > "$HARNESS_LOG" 2>&1 &
HARNESS_PID=$!
echo "[run-tests] harness pid=$HARNESS_PID"

# 11. Wait for sentinel line
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

# 12. Print summary
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
    print(f"  [{tag}] {r['driver']:10s} {r['case']}{err}")
PY
fi
echo "======================================================================="

case "$sentinel" in
  *PASS*) exit 0 ;;
  *)      exit 1 ;;
esac
