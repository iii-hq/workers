#!/usr/bin/env bash
# Variant of run-tests.sh that boots the engine against
# `config-jailed.yaml` (with `fs.host_root` set) and runs the
# jailed-only vuln-repro suite (S-C1). Reuses the same engine startup
# logic; the differences are: (1) a different config, (2) HARNESS_SUITE=jailed
# so runner.ts picks the matching case set, (3) a separate report path
# so a default run doesn't clobber it.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Central configuration (v0.4.0+): the engine launches the shell worker with
# --config <temp.yaml> derived from the `shell` config block in config-jailed.yaml.
# The shell worker registers that as the SEED with the built-in `configuration`
# worker (configuration::register) and then reads it back (configuration::get).
# The configuration worker is fresh per engine process, so the config-jailed.yaml
# block is authoritative each run — no separate seed step is needed.

WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/shell}"
WORKER_BIN_LINK="${WORKER_BIN_LINK:-$HOME/.iii/workers/shell}"
JAIL_ROOT="${JAIL_ROOT:-/private/tmp/iii-shell-jailed-root}"

REPORT_PATH="$ROOT_DIR/reports/report-jailed.json"
TS=$(date +%Y%m%d-%H%M%S)
ENGINE_LOG="$ROOT_DIR/reports/engine-jailed-$TS.log"
HARNESS_LOG="$ROOT_DIR/reports/harness-jailed-$TS.log"
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

Runs the jailed vuln-repro suite (config-jailed.yaml) — currently just
S-C1 (symlink-parent jail escape).

  --keep     Leave the engine running after the run.
  --no-build Skip cargo build of the shell worker.
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

mkdir -p "$ROOT_DIR/reports" "$ROOT_DIR/data" "$(dirname "$WORKER_BIN_LINK")" "$JAIL_ROOT"

if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "[run-tests-jailed] cargo build --release (shell worker)"
  (cd "$WORKER_SRC" && cargo build --release --bin shell)
fi
if [[ ! -x "$WORKER_BIN_TARGET" ]]; then
  echo "[run-tests-jailed] FATAL: worker binary missing at $WORKER_BIN_TARGET" >&2
  exit 1
fi

if [[ ! -L "$WORKER_BIN_LINK" || "$(readlink "$WORKER_BIN_LINK")" != "$WORKER_BIN_TARGET" ]]; then
  ln -sfn "$WORKER_BIN_TARGET" "$WORKER_BIN_LINK"
fi

if [[ ! -x "$III_BIN" ]]; then
  echo "[run-tests-jailed] FATAL: iii engine binary missing at $III_BIN" >&2
  exit 1
fi

if [[ ! -d "$ROOT_DIR/workers/harness/node_modules" ]]; then
  echo "[run-tests-jailed] npm install (harness)"
  (cd "$ROOT_DIR/workers/harness" && npm install --silent)
fi

export HARNESS_TEST_VAR="harness-allowed-value"

echo "[run-tests-jailed] starting iii engine (jailed config: host_root=$JAIL_ROOT)"
: > "$ENGINE_LOG"
: > "$HARNESS_LOG"

( cd "$ROOT_DIR" && "$III_BIN" --no-update-check -c ./config-jailed.yaml ) > "$ENGINE_LOG" 2>&1 &
ENGINE_PID=$!
echo "[run-tests-jailed] engine pid=$ENGINE_PID"

deadline=$(( $(date +%s) + 30 ))
while :; do
  if (echo > /dev/tcp/127.0.0.1/49134) 2>/dev/null; then
    break
  fi
  if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
    echo "[run-tests-jailed] FATAL: engine exited before binding port; tail of engine log:" >&2
    tail -40 "$ENGINE_LOG" >&2
    exit 1
  fi
  if (( $(date +%s) > deadline )); then
    echo "[run-tests-jailed] FATAL: engine did not bind port 49134 within 30s; tail of engine log:" >&2
    tail -40 "$ENGINE_LOG" >&2
    exit 1
  fi
  sleep 0.5
done
echo "[run-tests-jailed] engine listening"

echo "[run-tests-jailed] running coder::* live BDD smoke"
( cd "$WORKER_SRC" && \
  env III_ENGINE_WS_URL=ws://127.0.0.1:49134 \
      cargo test --test bdd -- --tags @live )

echo "[run-tests-jailed] starting harness (HARNESS_SUITE=jailed)"
( cd "$ROOT_DIR/workers/harness" && \
  env III_URL=ws://127.0.0.1:49134 \
      HARNESS_REPORT_PATH="$REPORT_PATH" \
      HARNESS_SUITE=jailed \
      HARNESS_TEST_VAR="$HARNESS_TEST_VAR" \
      npm run --silent dev ) > "$HARNESS_LOG" 2>&1 &
HARNESS_PID=$!

sentinel=""
deadline=$(( $(date +%s) + SENTINEL_TIMEOUT ))
while (( $(date +%s) < deadline )); do
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    if grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG" >/dev/null 2>&1; then
      sentinel=$(grep -m1 -E '^HARNESS_DONE: (PASS|FAIL) [0-9]+/[0-9]+$' "$HARNESS_LOG")
      break
    fi
    echo "[run-tests-jailed] harness exited without sentinel; tail:" >&2
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
  echo "[run-tests-jailed] FATAL: harness did not emit sentinel within ${SENTINEL_TIMEOUT}s" >&2
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
