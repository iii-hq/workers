#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Path overrides (set in CI; defaults assume the harness lives at
# browser/tests/e2e/ inside the workers repo and the iii engine is
# on $PATH or at $HOME/.local/bin/iii — which is where the install script
# `curl -fsSL https://install.iii.dev/iii/main/install.sh | sh` puts it).
WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/browser}"

# Port isolation. This machine commonly runs a shared dev engine on the iii
# default port (49134) that already has a `browser` worker registered — some
# other build, from some other worktree. Triggering against it would silently
# test the wrong binary while reporting a pass. (The namespace rename to
# root `browser::*` ids do not collide with the PYTHON scrapling worker's
# `scrapling::*` ids, but "wrong build of the same
# worker" is the sharper risk and it is not fixed by namespacing.)
# The iii CLI has no `--port` flag
# and no `iii start` subcommand (checked `iii --help`); the override lives in
# config.yaml instead, on the builtin `iii-worker-manager` worker's `port`
# key (confirmed by reading the iii engine source,
# engine/src/workers/worker/mod.rs's `WorkerManagerConfig` — the same knob
# `sdk/fixtures/config-test.yaml` uses there). So: default to a DIFFERENT
# port than the iii default, and thread it through engine config, the
# worker's --url, and the harness's III_URL.
export E2E_PORT="${E2E_PORT:-49234}"

REPORT_PATH="$ROOT_DIR/reports/report.json"
TS=$(date +%Y%m%d-%H%M%S)
ENGINE_LOG="$ROOT_DIR/reports/engine-$TS.log"
WORKER_LOG="$ROOT_DIR/reports/browser-$TS.log"
HARNESS_LOG="$ROOT_DIR/reports/harness-$TS.log"
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
Usage: $0 [--keep] [--no-build] [--filter=<case-substring>]

  --keep      Leave the engine + worker running after the run (debugging).
  --no-build  Skip cargo build of the browser worker.
  --filter=X  Run only harness cases whose name contains X (substring match;
              there's no per-driver concept here).

Env overrides:
  E2E_PORT           Engine WebSocket port (default: 49234 — deliberately
                      NOT the iii default of 49134, to avoid colliding with a
                      locally running dev stack that may have another
                      browser worker registered under the same browser::* ids).
                      Set to another port if 49234 is also taken.
  WORKER_SRC          Path to the browser crate (default: ../..).
  III_BIN             Path to the iii engine binary (default: \$(command -v iii) or \$HOME/.local/bin/iii).
  WORKER_BIN_TARGET   Path to the built worker binary (default: \$WORKER_SRC/target/release/browser).
  HARNESS_TIMEOUT     Seconds to wait for the harness sentinel (default: 120).
EOF
      exit 0
      ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

ENGINE_PID=""
WORKER_PID=""
HARNESS_PID=""
cleanup() {
  local code=$?
  if [[ -n "$HARNESS_PID" ]] && kill -0 "$HARNESS_PID" 2>/dev/null; then
    kill "$HARNESS_PID" 2>/dev/null || true
    wait "$HARNESS_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP" -eq 0 && -n "$WORKER_PID" ]] && kill -0 "$WORKER_PID" 2>/dev/null; then
    kill "$WORKER_PID" 2>/dev/null || true
    wait "$WORKER_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP" -eq 0 && -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    kill "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

mkdir -p "$ROOT_DIR/reports"

# 1. Port preflight — first, before the ~17s cargo build, so a bound port
# fails fast instead of after paying for a build we'd have to abort anyway.
# NEVER touch whatever is already listening — it may be someone's dev engine
# (see the E2E_PORT comment above). If our target port is already bound,
# hard-fail with a clear remediation instead of guessing whose engine it is
# or trying to reuse it.
if (echo > "/dev/tcp/127.0.0.1/$E2E_PORT") 2>/dev/null; then
  echo "[run-tests] FATAL: port $E2E_PORT is already in use." >&2
  echo "[run-tests] This suite refuses to share a port with another running engine — it may" >&2
  echo "[run-tests] be a dev stack already running some other build of the browser worker," >&2
  echo "[run-tests] which would silently test the wrong binary and still report a pass." >&2
  echo "[run-tests] Stop whatever owns port $E2E_PORT, or re-run with E2E_PORT=<a-free-port>." >&2
  exit 1
fi

# 2. Build the worker (unless --no-build)
if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "[run-tests] cargo build --release --bin browser"
  (cd "$WORKER_SRC" && cargo build --release --bin browser)
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

# 4. Config-rewrite dodge. The iii engine is known to rewrite the config
# file it boots from (observed on database/tests/e2e/config.yaml, a tracked
# file, in this same repo). Boot from an untracked copy so a local run never
# dirties the tracked config.
RUNTIME_CONFIG="$ROOT_DIR/reports/config.runtime.yaml"
cp "$ROOT_DIR/config.yaml" "$RUNTIME_CONFIG"

# Browser fetchers are certified against the frozen Chrome build. Reuse a
# caller-provided executable, otherwise verify the local artifact cache and
# fetch it only when absent.
if [[ -z "${SCRAPLING_CHROMIUM_EXECUTABLE:-}" ]]; then
  CHROMIUM_OUTPUT=""
  if ! CHROMIUM_OUTPUT=$("$WORKER_SRC/scripts/fetch_chromium_artifacts.sh" verify 2>/dev/null); then
    CHROMIUM_OUTPUT=$("$WORKER_SRC/scripts/fetch_chromium_artifacts.sh" fetch)
  fi
  SCRAPLING_CHROMIUM_EXECUTABLE=$(awk -F= '/^SCRAPLING_CHROMIUM_EXECUTABLE=/{print substr($0, index($0,"=")+1)}' <<<"$CHROMIUM_OUTPUT")
fi
if [[ ! -x "$SCRAPLING_CHROMIUM_EXECUTABLE" ]]; then
  echo "[run-tests] FATAL: frozen Chromium executable missing at $SCRAPLING_CHROMIUM_EXECUTABLE" >&2
  exit 1
fi

BROWSER_CONFIG="$ROOT_DIR/reports/browser.runtime.json"
python3 - "$BROWSER_CONFIG" "$SCRAPLING_CHROMIUM_EXECUTABLE" "$ROOT_DIR/reports/elements.db" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "scrapling": {
        "security_mode": "safe",
        "chromium_executable": sys.argv[2],
        "allow_loopback": True,
        "adaptive_storage_path": sys.argv[3],
    }
}))
PY

# The engine's builtin `configuration` worker also persists any config
# resolved through its fs adapter to ./config/<id>.yaml relative to CWD
# (observed for `iii-observability`) — reset it so a stale value from a
# previous run can never shadow what THIS run's config.yaml asks for.
rm -rf "$ROOT_DIR/config"

# 5. Install harness deps if needed
if [[ ! -d "$ROOT_DIR/workers/harness/node_modules" ]]; then
  echo "[run-tests] npm install (harness)"
  (cd "$ROOT_DIR/workers/harness" && npm install --silent)
fi

# 6. Start the engine (from the untracked runtime config copy; see step 4).
echo "[run-tests] starting iii engine on port $E2E_PORT"
: > "$ENGINE_LOG"
: > "$HARNESS_LOG"

( cd "$ROOT_DIR" && "$III_BIN" --no-update-check -c "$RUNTIME_CONFIG" ) > "$ENGINE_LOG" 2>&1 &
ENGINE_PID=$!
echo "[run-tests] engine pid=$ENGINE_PID"

# 7. Wait for the engine to accept TCP on $E2E_PORT. Probing the port
# directly instead of grepping for an engine log line decouples this script
# from the engine's logging format.
deadline=$(( $(date +%s) + 30 ))
while :; do
  if (echo > "/dev/tcp/127.0.0.1/$E2E_PORT") 2>/dev/null; then
    break
  fi
  if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
    echo "[run-tests] FATAL: engine exited before binding port; tail of engine log:" >&2
    tail -40 "$ENGINE_LOG" >&2
    exit 1
  fi
  if (( $(date +%s) > deadline )); then
    echo "[run-tests] FATAL: engine did not bind port $E2E_PORT within 30s; tail of engine log:" >&2
    tail -40 "$ENGINE_LOG" >&2
    exit 1
  fi
  sleep 0.5
done
echo "[run-tests] engine listening on $E2E_PORT"

# 8. Start the browser worker as a host process (not engine-managed).
echo "[run-tests] starting browser worker"
: > "$WORKER_LOG"
( cd "$ROOT_DIR" && "$WORKER_BIN_TARGET" --config "$BROWSER_CONFIG" --url "ws://127.0.0.1:$E2E_PORT" ) > "$WORKER_LOG" 2>&1 &
WORKER_PID=$!
echo "[run-tests] browser worker pid=$WORKER_PID"

# 9. Wait for browser::css to succeed (worker startup + function registration).
deadline=$(( $(date +%s) + 30 ))
while :; do
  if "$III_BIN" trigger --port "$E2E_PORT" browser::css html='<p>x</p>' query='p' >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$WORKER_PID" 2>/dev/null; then
    echo "[run-tests] FATAL: browser worker exited before becoming ready; tail of worker log:" >&2
    tail -40 "$WORKER_LOG" >&2
    exit 1
  fi
  if (( $(date +%s) > deadline )); then
    echo "[run-tests] FATAL: browser worker did not respond within 30s; tail of worker log:" >&2
    tail -40 "$WORKER_LOG" >&2
    exit 1
  fi
  sleep 0.5
done
echo "[run-tests] browser worker ready"

# 10. Launch the harness as a host node process
echo "[run-tests] starting harness"
HARNESS_ENV=()
if [[ -n "$FILTER" ]]; then
  HARNESS_ENV+=("HARNESS_FILTER=$FILTER")
fi
HARNESS_ENV+=("III_URL=ws://127.0.0.1:$E2E_PORT")
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
    print(f"  [{tag}] {r['case']}{err}")
PY
fi
echo "======================================================================="

case "$sentinel" in
  *PASS*) exit 0 ;;
  *)      exit 1 ;;
esac
