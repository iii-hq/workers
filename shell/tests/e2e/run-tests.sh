#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
host_home=$HOME

# Central configuration (v0.4.0+): the engine launches the shell worker with
# --config <temp.yaml> derived from the `shell` config block in config.yaml.
# The shell worker registers that as the SEED with the built-in `configuration`
# worker (configuration::register) and then reads it back (configuration::get).
# The engine's file-backed configuration worker persists entries under its
# current directory, so the launcher runs it from a throwaway project and
# passes it a copy of config.yaml. The checkout is never used as runtime state.

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
worker_bin_link_override=${WORKER_BIN_LINK:-}

TS=$(date +%Y%m%d-%H%M%S)
SENTINEL_TIMEOUT="${HARNESS_TIMEOUT:-90}"

KEEP=0
NO_BUILD=0
SUITE=default

for arg in "$@"; do
  case "$arg" in
    --keep)     KEEP=1 ;;
    --no-build) NO_BUILD=1 ;;
    --suite=default|--suite=jailed) SUITE="${arg#--suite=}" ;;
    -h|--help)
      cat <<EOF
Usage: $0 [--keep] [--no-build] [--suite=default|jailed]

  --keep     Leave the engine running after the run (for debugging).
  --no-build Skip cargo build of the shell worker.
  --suite    Select the default or jailed regression suite.

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

if [[ "$SUITE" == jailed ]]; then
  CONFIG_FILE=config-jailed.yaml
  REPORT_PATH="$ROOT_DIR/reports/report-jailed.json"
  ENGINE_LOG="$ROOT_DIR/reports/engine-jailed-$TS.log"
  HARNESS_LOG="$ROOT_DIR/reports/harness-jailed-$TS.log"
  jail_root_override=${JAIL_ROOT:-}
else
  CONFIG_FILE=config.yaml
  REPORT_PATH="$ROOT_DIR/reports/report.json"
  ENGINE_LOG="$ROOT_DIR/reports/engine-$TS.log"
  HARNESS_LOG="$ROOT_DIR/reports/harness-$TS.log"
fi

RUN_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/shell-e2e.XXXXXX")
ENGINE_CONFIG="$RUN_ROOT/config.yaml"
export HOME="$RUN_ROOT/home"
export XDG_CONFIG_HOME="$HOME/.config"
export CARGO_HOME=${CARGO_HOME:-"$host_home/.cargo"}
export RUSTUP_HOME=${RUSTUP_HOME:-"$host_home/.rustup"}
WORKER_BIN_LINK=${worker_bin_link_override:-"$HOME/.iii/workers/shell"}
if [[ "$SUITE" == jailed ]]; then
  JAIL_ROOT=${jail_root_override:-"$RUN_ROOT/jail"}
fi

ENGINE_PID=""
HARNESS_PID=""
# shellcheck disable=SC2317 # Invoked by EXIT/INT/TERM traps.
cleanup() {
  local code=$?
  trap - EXIT INT TERM
  if [[ -n "$HARNESS_PID" ]] && kill -0 "$HARNESS_PID" 2>/dev/null; then
    kill "$HARNESS_PID" 2>/dev/null || true
    wait "$HARNESS_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP" -eq 0 ]] && [[ -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    kill -- "-$ENGINE_PID" 2>/dev/null || kill "$ENGINE_PID" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$ENGINE_PID" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL -- "-$ENGINE_PID" 2>/dev/null || kill -KILL "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  [[ "$KEEP" -eq 1 ]] || rm -rf "$RUN_ROOT"
  exit "$code"
}
trap cleanup EXIT INT TERM

if (echo > /dev/tcp/127.0.0.1/49134) 2>/dev/null; then
  echo "[run-tests] FATAL: port 49134 is already in use; stop the existing engine first" >&2
  exit 3
fi

mkdir -p "$ROOT_DIR/reports" "$RUN_ROOT/data" "$(dirname "$WORKER_BIN_LINK")"
[[ "$SUITE" == jailed ]] && mkdir -p "$JAIL_ROOT"

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
unset ANTHROPIC_API_KEY OPENAI_API_KEY ZAI_API_KEY

# 6. Start the engine
echo "[run-tests] starting iii engine"
: > "$ENGINE_LOG"
: > "$HARNESS_LOG"
cp "$ROOT_DIR/$CONFIG_FILE" "$ENGINE_CONFIG"
if [[ "$SUITE" == jailed ]]; then
  escaped_jail_root=${JAIL_ROOT//|/\\|}
  sed -i "s|/private/tmp/iii-shell-jailed-root|$escaped_jail_root|g" "$ENGINE_CONFIG"
fi

if command -v setsid >/dev/null 2>&1; then
  (cd "$RUN_ROOT" && exec setsid "$III_BIN" --no-update-check -c "$ENGINE_CONFIG") >"$ENGINE_LOG" 2>&1 &
else
  (cd "$RUN_ROOT" && exec "$III_BIN" --no-update-check -c "$ENGINE_CONFIG") >"$ENGINE_LOG" 2>&1 &
fi
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

# The jailed lane also exercises the live BDD contract before its focused
# harness regression cases.
if [[ "$SUITE" == jailed ]]; then
  (cd "$WORKER_SRC" && \
    env III_ENGINE_WS_URL=ws://127.0.0.1:49134 \
    cargo test --test bdd -- --tags @live)
fi

# 8. Launch the harness as a host node process
echo "[run-tests] starting harness"
HARNESS_ENV=(
  "III_URL=ws://127.0.0.1:49134"
  "HARNESS_REPORT_PATH=$REPORT_PATH"
  "HARNESS_TEST_VAR=$HARNESS_TEST_VAR"
  "HARNESS_NOT_ALLOWED=$HARNESS_NOT_ALLOWED"
)
[[ "$SUITE" == jailed ]] && HARNESS_ENV+=("HARNESS_SUITE=jailed")
[[ "$SUITE" == jailed ]] && HARNESS_ENV+=("JAIL_ROOT=$JAIL_ROOT")
(cd "$ROOT_DIR/workers/harness" && env "${HARNESS_ENV[@]}" npm run --silent dev) \
  >"$HARNESS_LOG" 2>&1 &
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
