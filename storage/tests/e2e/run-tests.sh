#!/usr/bin/env bash
# E2E orchestrator for the storage worker, modelled on
# iii-database/tests/e2e/run-tests.sh. Builds the worker binary, starts the
# engine, launches the TS harness, greps for the HARNESS_DONE sentinel,
# pretty-prints the report, and propagates pass/fail as the exit code.
#
# Local backend only — no docker compose, no AWS/GCP/CF creds. The
# rustfs sidecar is spawned by storage itself when the engine starts.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Path overrides (set in CI; defaults assume the harness lives at
# storage/tests/e2e/ inside the workers repo and the iii engine is on
# $PATH or at $HOME/.local/bin/iii — which is where the install script
# `curl -fsSL https://install.iii.dev/iii/main/install.sh | sh` puts it).
WORKER_SRC="${WORKER_SRC:-$(cd "$ROOT_DIR/../.." && pwd)}"
III_BIN="${III_BIN:-$(command -v iii 2>/dev/null || echo "$HOME/.local/bin/iii")}"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$WORKER_SRC/target/release/storage}"
WORKER_BIN_LINK="${WORKER_BIN_LINK:-$HOME/.iii/workers/storage}"

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

for arg in "$@"; do
  case "$arg" in
    --keep)         KEEP=1 ;;
    --no-build)     NO_BUILD=1 ;;
    --no-pull)      NO_PULL=1 ;;
    --filter=*)     FILTER="${arg#--filter=}" ;;
    --providers=*)  PROVIDERS="${arg#--providers=}" ;;
    -h|--help)
      cat <<EOF
Usage: $0 [--providers=local|all|<csv>] [--keep] [--no-build] [--no-pull] [--filter=<substr>]

  --providers=local        (default) only exercise the local rustfs backend.
                           No docker required; preserves today's behavior.
  --providers=all          local + s3 + r2 via docker compose.
  --providers=local,s3     explicit subset (csv of: local, s3, r2).
  --keep                   Leave compose stack and tests/e2e/data/ in place.
  --no-build               Skip cargo build of the storage worker.
  --no-pull                Skip 'docker compose pull'; use cached images.
  --filter=SUBSTRING       Run only cases whose name contains SUBSTRING.

Env overrides:
  WORKER_SRC               Path to the storage worker crate (default: ../..).
  III_BIN                  Path to the iii engine binary.
  WORKER_BIN_TARGET        Path to the built worker binary.
  WORKER_BIN_LINK          Symlink the engine reads.
  HARNESS_TIMEOUT          Seconds to wait for the harness sentinel (default: 180).
  RUSTFS_BIN               Override path to rustfs (otherwise auto-fetched).
  RUSTFS_AUTO_FETCH        Set to 0 to disable rustfs auto-fetch.

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
  # Belt-and-suspenders: rustfs sometimes outlives the engine on SIGKILL paths.
  pkill -P $$ -f rustfs 2>/dev/null || true
  if [[ "$NEEDS_DOCKER" -eq 1 && "$KEEP" -eq 0 ]]; then
    (cd "$ROOT_DIR" && docker compose --profile cloud down -v --remove-orphans 2>/dev/null) || true
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

# 3b. Ensure rustfs is available. storage's local provider spawns rustfs
# as a sidecar; without it the worker exits with LOCAL_BACKEND_BIN_NOT_FOUND
# before registering RPCs, and the harness then times out waiting for
# functions that will never appear.
#
# Discovery order in storage/src/rustfs/spawn.rs::discover_binary:
#   1. $RUSTFS_BIN
#   2. <dir of storage binary>/rustfs   ← we drop the auto-fetched one here
#   3. rustfs on $PATH
#
# RUSTFS_VERSION pins which release to fetch. Set RUSTFS_AUTO_FETCH=0 to skip
# the download path entirely (e.g. air-gapped CI that pre-stages the binary).
RUSTFS_VERSION="${RUSTFS_VERSION:-1.0.0-beta.1}"
RUSTFS_AUTO_FETCH="${RUSTFS_AUTO_FETCH:-1}"
RUSTFS_LOCAL_PATH="$(dirname "$WORKER_BIN_TARGET")/rustfs"

ensure_rustfs() {
  # Existing override / install wins — never re-download if the user already has one.
  if [[ -n "${RUSTFS_BIN:-}" && -x "$RUSTFS_BIN" ]]; then
    echo "[run-tests] using \$RUSTFS_BIN=$RUSTFS_BIN"
    return 0
  fi
  if command -v rustfs >/dev/null 2>&1; then
    echo "[run-tests] using rustfs from PATH: $(command -v rustfs)"
    return 0
  fi
  if [[ -x "$RUSTFS_LOCAL_PATH" ]]; then
    echo "[run-tests] using cached rustfs: $RUSTFS_LOCAL_PATH"
    return 0
  fi
  if [[ "$RUSTFS_AUTO_FETCH" != "1" ]]; then
    echo "[run-tests] FATAL: rustfs not found and RUSTFS_AUTO_FETCH=0" >&2
    echo "[run-tests]   set \$RUSTFS_BIN, place a binary at $RUSTFS_LOCAL_PATH, or install rustfs on \$PATH" >&2
    return 1
  fi

  # Detect platform. The release naming is rustfs-<os>-<arch>{-<libc>}-v<ver>.zip
  # where <libc> is gnu|musl on Linux only. We default to gnu — alpine/musl users
  # can pre-install rustfs and re-run with RUSTFS_AUTO_FETCH=0.
  local uname_s uname_m os arch asset
  uname_s="$(uname -s)"
  uname_m="$(uname -m)"
  case "$uname_s" in
    Darwin) os="macos" ;;
    Linux)  os="linux" ;;
    *)
      echo "[run-tests] FATAL: rustfs auto-fetch unsupported on $uname_s." >&2
      echo "[run-tests]   install manually from https://github.com/rustfs/rustfs/releases and set \$RUSTFS_BIN" >&2
      return 1 ;;
  esac
  case "$uname_m" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64)  arch="x86_64" ;;
    *)
      echo "[run-tests] FATAL: rustfs auto-fetch unsupported on arch $uname_m." >&2
      return 1 ;;
  esac
  if [[ "$os" == "linux" ]]; then
    asset="rustfs-linux-${arch}-gnu-v${RUSTFS_VERSION}.zip"
  else
    asset="rustfs-macos-${arch}-v${RUSTFS_VERSION}.zip"
  fi
  local base="https://github.com/rustfs/rustfs/releases/download/${RUSTFS_VERSION}"
  local url="${base}/${asset}"
  local sums_url="${base}/SHA256SUMS"

  echo "[run-tests] fetching rustfs ${RUSTFS_VERSION} (${os}/${arch}) — one-time, ~70-90MB compressed"
  local tmpdir
  tmpdir="$(mktemp -d -t rustfs-fetch.XXXXXX)"
  trap "rm -rf '$tmpdir'" RETURN
  if ! curl -fsSL --retry 3 -o "$tmpdir/$asset" "$url"; then
    echo "[run-tests] FATAL: download failed: $url" >&2
    return 1
  fi

  # Best-effort SHA256 verification. If the sums file is unreachable we WARN
  # but proceed — flaky CDN shouldn't block local dev. CI that wants to be
  # strict can pre-stage the binary or set RUSTFS_AUTO_FETCH=0.
  if curl -fsSL --retry 3 -o "$tmpdir/SHA256SUMS" "$sums_url" 2>/dev/null; then
    local expected actual sum_cmd
    expected="$(awk -v a="$asset" '$2==a {print $1}' "$tmpdir/SHA256SUMS")"
    if [[ -n "$expected" ]]; then
      if command -v sha256sum >/dev/null 2>&1; then sum_cmd="sha256sum"; else sum_cmd="shasum -a 256"; fi
      actual="$($sum_cmd "$tmpdir/$asset" | awk '{print $1}')"
      if [[ "$actual" != "$expected" ]]; then
        echo "[run-tests] FATAL: rustfs SHA256 mismatch (expected $expected, got $actual)" >&2
        return 1
      fi
      echo "[run-tests] sha256 verified ($expected)"
    else
      echo "[run-tests] WARN: $asset not listed in SHA256SUMS — skipping verification"
    fi
  else
    echo "[run-tests] WARN: SHA256SUMS unreachable — skipping verification"
  fi

  if ! command -v unzip >/dev/null 2>&1; then
    echo "[run-tests] FATAL: 'unzip' not on PATH; install it or pre-stage rustfs" >&2
    return 1
  fi
  unzip -q -o "$tmpdir/$asset" -d "$tmpdir"
  if [[ ! -f "$tmpdir/rustfs" ]]; then
    echo "[run-tests] FATAL: zip did not contain a top-level 'rustfs' binary" >&2
    ls "$tmpdir" >&2
    return 1
  fi

  mkdir -p "$(dirname "$RUSTFS_LOCAL_PATH")"
  chmod +x "$tmpdir/rustfs"
  mv -f "$tmpdir/rustfs" "$RUSTFS_LOCAL_PATH"
  # macOS quarantines downloaded binaries until first-launch confirm; strip
  # the attribute so storage can spawn it without a Gatekeeper prompt.
  if [[ "$os" == "macos" ]] && command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.quarantine "$RUSTFS_LOCAL_PATH" 2>/dev/null || true
  fi
  echo "[run-tests] installed rustfs → $RUSTFS_LOCAL_PATH"
}

ensure_rustfs

# Resolve the absolute rustfs path the engine should use and export it.
# discover_binary() in storage tries $RUSTFS_BIN first, then a sibling of
# the storage executable, then $PATH. The "sibling" path is fragile here
# because the engine spawns storage via the symlink at $WORKER_BIN_LINK
# (e.g. ~/.iii/workers/storage), and on macOS std::env::current_exe()
# returns the symlink path — so the sibling lookup would find ~/.iii/workers/
# rather than target/release/. Setting RUSTFS_BIN sidesteps that entirely
# and the engine forwards the env to the worker process.
if [[ -n "${RUSTFS_BIN:-}" && -x "$RUSTFS_BIN" ]]; then
  RESOLVED_RUSTFS="$RUSTFS_BIN"
elif [[ -x "$RUSTFS_LOCAL_PATH" ]]; then
  RESOLVED_RUSTFS="$RUSTFS_LOCAL_PATH"
elif command -v rustfs >/dev/null 2>&1; then
  RESOLVED_RUSTFS="$(command -v rustfs)"
fi
if [[ -n "${RESOLVED_RUSTFS:-}" ]]; then
  export RUSTFS_BIN="$RESOLVED_RUSTFS"
  echo "[run-tests] RUSTFS_BIN=$RUSTFS_BIN"
fi

# 4. Reset rustfs data tree. rustfs is stateful on disk; leftover objects
# from a previous run would make the schema-reset case incomplete and
# could let the silence assertion observe in-flight events from the
# old generation.
echo "[run-tests] resetting tests/e2e/data/storage"
rm -rf "$ROOT_DIR/data/storage"
mkdir -p "$ROOT_DIR/data/storage"

# 4b. Bring up docker compose stack (if needed) and bootstrap buckets.
if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
  echo "[run-tests] starting docker compose stack (profile=cloud)"
  if [[ "$NO_PULL" -eq 0 ]]; then
    (cd "$ROOT_DIR" && docker compose --profile cloud pull --quiet) || true
  fi
  (cd "$ROOT_DIR" && docker compose --profile cloud up -d)

  # Wait for health.
  health_deadline=$(( $(date +%s) + 60 ))
  while :; do
    unhealthy=$(cd "$ROOT_DIR" && \
      docker compose --profile cloud ps --format '{{.Service}} {{.Health}}' \
      | awk '$2 != "healthy" {print $1}')
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

ENGINE_CONFIG="./config.yaml"
if [[ "$NEEDS_DOCKER" -eq 1 ]]; then
  ENGINE_CONFIG="./config.all.yaml"
fi

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
  # Defensive: a developer with AWS_ENDPOINT_URL pinned to real AWS would
  # silently break SQS polling. The queue URL in config.all.yaml already
  # encodes the ElasticMQ host:port; clear any global override.
  unset AWS_ENDPOINT_URL
fi

echo "[run-tests] starting iii engine (config=$ENGINE_CONFIG)"
( cd "$ROOT_DIR" && "$III_BIN" --no-update-check -c "$ENGINE_CONFIG" ) > "$ENGINE_LOG" 2>&1 &
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
    echo "[run-tests] === per-container logs (last 20 lines) ===" >&2
    for svc in minio; do
      echo "--- $svc ---" >&2
      (cd "$ROOT_DIR" && docker compose logs --tail=20 "$svc") >&2 || true
    done
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
    tag = "PASS" if r["status"] == "PASS" else "FAIL"
    err = (" — " + r.get("error","")) if r["status"] == "FAIL" else ""
    print(f"  [{tag}] {r['case']}{err}")
PY
fi
echo "======================================================================="

# 11. Optional data preservation
if [[ "$KEEP" -eq 0 ]]; then
  rm -rf "$ROOT_DIR/data/storage"
fi

case "$sentinel" in
  *PASS*)  exit 0 ;;
  *ERROR*) exit 2 ;;
  *)       exit 1 ;;
esac
