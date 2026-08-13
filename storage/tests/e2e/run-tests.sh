#!/usr/bin/env bash
# E2E orchestrator for the storage worker, modelled on
# database/tests/e2e/run-tests.sh. Builds the worker binary, starts the
# engine, launches the TS harness, greps for the HARNESS_DONE sentinel,
# pretty-prints the report, and propagates pass/fail as the exit code.
#
# Local backend only — no docker compose, no AWS/GCP/CF creds, and no
# external object-storage process.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
host_home=$HOME

# Path overrides (set in CI; defaults assume the harness lives at
# storage/tests/e2e/ inside the workers repo and the iii engine is on
# $PATH or at $HOME/.local/bin/iii — which is where the install script
# `curl -fsSL https://install.iii.dev/iii/main/install.sh | sh` puts it).
WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/storage}"
worker_bin_link_override=${WORKER_BIN_LINK:-}

REPORT_PATH="$ROOT_DIR/reports/report.json"
TS=$(date +%Y%m%d-%H%M%S)
ENGINE_LOG="$ROOT_DIR/reports/engine-$TS.log"
HARNESS_LOG="$ROOT_DIR/reports/harness-$TS.log"
SENTINEL_TIMEOUT="${HARNESS_TIMEOUT:-180}"

KEEP=0
NO_BUILD=0
NO_PULL=0
FILTER=""
PROVIDERS="local"
TRIGGER_PROVIDERS=""

for arg in "$@"; do
  case "$arg" in
    --keep)                 KEEP=1 ;;
    --no-build)             NO_BUILD=1 ;;
    --no-pull)              NO_PULL=1 ;;
    --filter=*)             FILTER="${arg#--filter=}" ;;
    --providers=*)          PROVIDERS="${arg#--providers=}" ;;
    --trigger-providers=*)  TRIGGER_PROVIDERS="${arg#--trigger-providers=}" ;;
    -h|--help)
      cat <<EOF
Usage: $0 [--providers=local|all|<csv>] [--trigger-providers=<csv>] [--keep] [--no-build] [--no-pull] [--filter=<substr>]

  --providers=local           (default) only exercise native local storage.
                              No docker required.
  --providers=all             local + s3 + r2 via docker compose.
  --providers=local,s3        explicit subset (csv of: local, s3, r2).
  --trigger-providers=<csv>   subset of --providers whose trigger plumbing is
                              wired here (default: same as --providers). Set to
                              local,s3 in CI so r2 trigger ERRORs (no Cloudflare
                              Queue) don't propagate as a non-zero exit.
  --keep                      Leave compose stack and tests/e2e/data/ in place.
  --no-build                  Skip cargo build of the storage worker.
  --no-pull                   Skip 'docker compose pull'; use cached images.
  --filter=SUBSTRING          Run only cases whose name contains SUBSTRING.

Env overrides:
  WORKER_SRC               Path to the storage worker crate (default: ../..).
  III_BIN                  Path to the iii engine binary.
  WORKER_BIN_TARGET        Path to the built worker binary.
  WORKER_BIN_LINK          Symlink the engine reads.
  HARNESS_TIMEOUT          Seconds to wait for the harness sentinel (default: 180).
Script-self-tests:
  ./script-tests/run.sh        Run bash tests of run-tests.sh itself.
                               Independent from the harness; CI runs both.
EOF
      exit 0
      ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Normalize --providers=all into the explicit csv.
if [[ "$PROVIDERS" == "all" ]]; then
  PROVIDERS="local,s3,r2"
fi

# Validate every comma-separated entry is one of the known set.
IFS=',' read -ra PROVIDER_LIST <<< "$PROVIDERS"
for p in "${PROVIDER_LIST[@]}"; do
  case "$p" in
    local|s3|r2) ;;
    *)
      echo "[run-tests] FATAL: unknown provider '$p' in --providers=$PROVIDERS" >&2
      echo "[run-tests]   valid: local, s3, r2 (or 'all')" >&2
      exit 3
      ;;
  esac
done

# Detect cloud-provider mode (anything other than local-only).
NEEDS_DOCKER=0
for p in "${PROVIDER_LIST[@]}"; do
  if [[ "$p" != "local" ]]; then NEEDS_DOCKER=1; break; fi
done

ENGINE_PID=""
HARNESS_PID=""
COMPOSE_STARTED=0
RUN_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/storage-e2e.XXXXXX")
export HOME="$RUN_ROOT/home"
export XDG_CONFIG_HOME="$HOME/.config"
export CARGO_HOME=${CARGO_HOME:-"$host_home/.cargo"}
export RUSTUP_HOME=${RUSTUP_HOME:-"$host_home/.rustup"}
WORKER_BIN_LINK=${worker_bin_link_override:-"$HOME/.iii/workers/storage"}
if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
  export COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-"storage-e2e-$$"}
fi
# shellcheck disable=SC2317 # Invoked by EXIT/INT/TERM traps.
cleanup() {
  local code=$?
  trap - EXIT INT TERM

  if [[ -n "$HARNESS_PID" ]] && kill -0 "$HARNESS_PID" 2>/dev/null; then
    kill "$HARNESS_PID" 2>/dev/null || true
    wait "$HARNESS_PID" 2>/dev/null || true
  fi
  if [[ -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    kill -- "-$ENGINE_PID" 2>/dev/null || kill "$ENGINE_PID" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$ENGINE_PID" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL -- "-$ENGINE_PID" 2>/dev/null || kill -KILL "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi

  if [[ "$COMPOSE_STARTED" -eq 1 && "$KEEP" -eq 0 ]]; then
    (cd "$ROOT_DIR" && docker compose --profile cloud down -v --remove-orphans 2>/dev/null) || true
  fi
  if [[ "$KEEP" -eq 0 ]]; then
    rm -rf "$RUN_ROOT"
  else
    echo "[run-tests] preserved isolated project at $RUN_ROOT"
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

# Pre-flight checks. Distinguish:
#   - infrastructure (exit 3): docker missing, port in use
#   - environment    (exit 2): handled later inside the harness via ERROR
#   - regression     (exit 1): handled later via FAIL

require_free_port() {
  local port=$1 name=$2
  if (echo > "/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    echo "[run-tests] FATAL: port $port already in use ($name)" >&2
    echo "[run-tests]   another iii engine, MinIO, or fake-gcs may be running; stop it first" >&2
    exit 3
  fi
}

require_free_port 49134 "iii engine websocket"

if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
  if ! command -v docker >/dev/null 2>&1; then
    echo "[run-tests] FATAL: docker not on PATH; install it or use --providers=local" >&2
    exit 3
  fi
  if ! docker compose version >/dev/null 2>&1; then
    echo "[run-tests] FATAL: 'docker compose' (v2) not available; install Docker Desktop or compose-plugin" >&2
    exit 3
  fi
  require_free_port 9000 "MinIO S3 API"
  require_free_port 9001 "MinIO console"
fi

mkdir -p "$ROOT_DIR/reports" "$ROOT_DIR/data" "$(dirname "$WORKER_BIN_LINK")"

# 1. Ensure binary symlink at $WORKER_BIN_LINK
if [[ ! -L "$WORKER_BIN_LINK" || "$(readlink "$WORKER_BIN_LINK")" != "$WORKER_BIN_TARGET" ]]; then
  ln -sfn "$WORKER_BIN_TARGET" "$WORKER_BIN_LINK"
  echo "[run-tests] symlink: $WORKER_BIN_LINK -> $WORKER_BIN_TARGET"
fi

# 2. Build the worker (unless --no-build)
if [[ "$NO_BUILD" -eq 0 ]]; then
  echo "[run-tests] cargo build --release (storage worker)"
  (cd "$WORKER_SRC" && cargo build --release --bin storage)
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

# 4. Create the isolated native-local data tree.
mkdir -p "$RUN_ROOT/data/storage"

# 4b. Bring up docker compose stack (if needed) and bootstrap buckets.
if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
  echo "[run-tests] starting docker compose stack (project=$COMPOSE_PROJECT_NAME, profile=cloud)"
  if [[ "$NO_PULL" -eq 0 ]]; then
    (cd "$ROOT_DIR" && docker compose --profile cloud pull --quiet) || true
  fi
  COMPOSE_STARTED=1
  (cd "$ROOT_DIR" && docker compose --profile cloud up -d)

  # Wait for health.
  health_deadline=$(( $(date +%s) + 60 ))
  while :; do
    # Services with no HEALTHCHECK defined emit an empty {{.Health}} field;
    # treat empty as "no opinion" (don't block waiting on services that
    # never report health). Only flag services that explicitly reported a
    # non-healthy status.
    unhealthy=$(cd "$ROOT_DIR" && \
      docker compose --profile cloud ps --format '{{.Service}} {{.Health}}' \
      | awk 'NF >= 2 && $2 != "healthy" {print $1}')
    if [[ -z "$unhealthy" ]]; then break; fi
    if (( $(date +%s) > health_deadline )); then
      echo "[run-tests] FATAL: compose services not healthy within 60s:" >&2
      echo "$unhealthy" >&2
      echo "[run-tests] === per-service logs (last 30 lines) ===" >&2
      for svc in $unhealthy; do
        echo "--- $svc ---" >&2
        (cd "$ROOT_DIR" && docker compose logs --tail=30 "$svc") >&2
      done
      exit 2
    fi
    sleep 1
  done
  echo "[run-tests] compose services healthy"

  # Bootstrap buckets, in parallel — they're independent.
  bootstrap_pids=()
  MINIO_INIT_DONE=""
  for p in "${PROVIDER_LIST[@]}"; do
    case "$p" in
      s3|r2)
        # MinIO init is shared between s3 and r2; only run once.
        if [[ -z "$MINIO_INIT_DONE" ]]; then
          MINIO_INIT_DONE=1
          "$ROOT_DIR/fixtures/minio-init.sh" &
          bootstrap_pids+=($!)
        fi
        ;;
    esac
  done
  for pid in "${bootstrap_pids[@]}"; do
    if ! wait "$pid"; then
      echo "[run-tests] FATAL: bucket bootstrap failed (pid=$pid)" >&2
      exit 2
    fi
  done
fi

# 5. Install harness deps if needed
if [[ ! -d "$ROOT_DIR/workers/harness/node_modules" ]]; then
  echo "[run-tests] npm install (harness)"
  (cd "$ROOT_DIR/workers/harness" && npm install --silent)
fi

# 6. Start the engine (default config: ./config.yaml)
: > "$ENGINE_LOG"
: > "$HARNESS_LOG"

ENGINE_CONFIG_SOURCE="$ROOT_DIR/config.yaml"
if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
  ENGINE_CONFIG_SOURCE="$ROOT_DIR/config.all.yaml"
fi
ENGINE_CONFIG="$RUN_ROOT/config.yaml"
cp "$ENGINE_CONFIG_SOURCE" "$ENGINE_CONFIG"

# When the s3 provider is in scope, the storage worker spawns an SQS poller
# (storage/src/main.rs:296-308) that uses the AWS SDK default credential
# chain to sign requests against ElasticMQ. ElasticMQ accepts any signature
# but the SDK still demands non-empty creds + a region. Override via env so
# we don't leak the developer's real `aws sso` session into the test, and
# so tests pass on a machine with no AWS config at all.
NEEDS_S3=0
for p in "${PROVIDER_LIST[@]}"; do
  if [[ "$p" == "s3" ]]; then NEEDS_S3=1; break; fi
done
if [[ "$NEEDS_S3" -eq 1 ]]; then
  export AWS_ACCESS_KEY_ID="elasticmq"
  export AWS_SECRET_ACCESS_KEY="elasticmq"
  export AWS_REGION="us-east-1"
  # Critical: the AWS Rust SDK does NOT derive its endpoint from QueueUrl.
  # Without an override it sends ReceiveMessage to sqs.us-east-1.amazonaws.com,
  # which rejects our placeholder creds as InvalidClientTokenId (surfaces in
  # the worker as the opaque "service error"). Pinning the SQS endpoint
  # routes traffic to the local ElasticMQ container instead.
  export AWS_ENDPOINT_URL_SQS="http://127.0.0.1:9324"
  # Defensive: a developer with a global AWS_ENDPOINT_URL pinned to real AWS
  # would override the per-service var above. Drop it.
  unset AWS_ENDPOINT_URL
fi

echo "[run-tests] starting iii engine (config=$ENGINE_CONFIG)"
unset ANTHROPIC_API_KEY OPENAI_API_KEY ZAI_API_KEY
if command -v setsid >/dev/null 2>&1; then
  (cd "$RUN_ROOT" && exec setsid "$III_BIN" --no-update-check -c "$ENGINE_CONFIG") >"$ENGINE_LOG" 2>&1 &
else
  (cd "$RUN_ROOT" && exec "$III_BIN" --no-update-check -c "$ENGINE_CONFIG") >"$ENGINE_LOG" 2>&1 &
fi
ENGINE_PID=$!
echo "[run-tests] engine pid=$ENGINE_PID"

# 7. Wait for the engine to accept TCP on its WebSocket port (49134).
# Probing the port directly instead of grepping for an engine log line
# decouples this script from the engine's logging format.
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
HARNESS_ENV=()
if [[ -n "$FILTER" ]]; then
  HARNESS_ENV+=("HARNESS_FILTER=$FILTER")
fi
HARNESS_ENV+=("III_URL=ws://127.0.0.1:49134")
HARNESS_ENV+=("HARNESS_REPORT_PATH=$REPORT_PATH")
HARNESS_ENV+=("HARNESS_PROVIDERS=$PROVIDERS")
# Forward HARNESS_TRIGGER_PROVIDERS — either from --trigger-providers= flag
# or inherited from caller's environment. Both empty → harness defaults to
# fan-out across every provider in HARNESS_PROVIDERS.
if [[ -n "$TRIGGER_PROVIDERS" ]]; then
  HARNESS_ENV+=("HARNESS_TRIGGER_PROVIDERS=$TRIGGER_PROVIDERS")
elif [[ -n "${HARNESS_TRIGGER_PROVIDERS:-}" ]]; then
  HARNESS_ENV+=("HARNESS_TRIGGER_PROVIDERS=$HARNESS_TRIGGER_PROVIDERS")
fi

( cd "$ROOT_DIR/workers/harness" && env "${HARNESS_ENV[@]}" npm run --silent dev ) > "$HARNESS_LOG" 2>&1 &
HARNESS_PID=$!
echo "[run-tests] harness pid=$HARNESS_PID"

# 9. Wait for sentinel line
sentinel=""
deadline=$(( $(date +%s) + SENTINEL_TIMEOUT ))
while (( $(date +%s) < deadline )); do
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    if grep -m1 -E '^HARNESS_DONE: (PASS|FAIL|ERROR) [0-9]+/[0-9]+( errors=[0-9]+)?$' "$HARNESS_LOG" >/dev/null 2>&1; then
      sentinel=$(grep -m1 -E '^HARNESS_DONE: (PASS|FAIL|ERROR) [0-9]+/[0-9]+( errors=[0-9]+)?$' "$HARNESS_LOG")
      break
    fi
    echo "[run-tests] harness exited without sentinel; tail of harness log:" >&2
    tail -40 "$HARNESS_LOG" >&2
    exit 1
  fi
  if grep -m1 -E '^HARNESS_DONE: (PASS|FAIL|ERROR) [0-9]+/[0-9]+( errors=[0-9]+)?$' "$HARNESS_LOG" >/dev/null 2>&1; then
    sentinel=$(grep -m1 -E '^HARNESS_DONE: (PASS|FAIL|ERROR) [0-9]+/[0-9]+( errors=[0-9]+)?$' "$HARNESS_LOG")
    break
  fi
  sleep 1
done

if [[ -z "$sentinel" ]]; then
  echo "[run-tests] FATAL: harness did not emit sentinel within ${SENTINEL_TIMEOUT}s" >&2
  echo "[run-tests] === tail of harness log ===" >&2
  tail -40 "$HARNESS_LOG" >&2
  echo "[run-tests] === tail of engine log ===" >&2
  tail -40 "$ENGINE_LOG" >&2
  if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
    echo "[run-tests] === docker compose ps ===" >&2
    (cd "$ROOT_DIR" && docker compose --profile cloud ps) >&2 || true
    echo "[run-tests] === minio logs (last 20 lines) ===" >&2
    (cd "$ROOT_DIR" && docker compose logs --tail=20 minio) >&2 || true
  fi
  exit 2
fi

# 10. Print summary
echo
echo "======================================================================="
echo "$sentinel"
if [[ -f "$REPORT_PATH" ]]; then
  python3 - "$REPORT_PATH" <<'PY' 2>/dev/null || cat "$REPORT_PATH"
import json, sys
data = json.load(open(sys.argv[1]))
for r in data["results"]:
    # Render the actual status — collapsing ERROR (probe-skipped) into FAIL
    # is misleading when triaging failures. Keep them distinct.
    tag = r["status"]
    err = (" — " + r.get("error","")) if r["status"] in ("FAIL", "ERROR") else ""
    print(f"  [{tag}] {r['case']}{err}")
PY
fi
echo "======================================================================="

case "$sentinel" in
  *PASS*)  exit 0 ;;
  *ERROR*) exit 2 ;;
  *)       exit 1 ;;
esac
