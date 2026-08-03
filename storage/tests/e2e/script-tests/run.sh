#!/usr/bin/env bash
# Tests for run-tests.sh itself. Each test runs a controlled invocation
# and asserts on exit code + log content. Independent from the main
# harness — these tests should pass in CI even when --providers=all is
# unavailable (e.g. on a developer laptop without docker).
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/run-tests.sh"
WORKER_BIN_TARGET="${WORKER_BIN_TARGET:-$ROOT_DIR/../../target/release/storage}"
PASS=0
FAIL=0

assert_eq() {
  local actual=$1 expected=$2 label=$3
  if [[ "$actual" == "$expected" ]]; then
    echo "  PASS: $label"
    PASS=$((PASS+1))
  else
    echo "  FAIL: $label — expected '$expected', got '$actual'"
    FAIL=$((FAIL+1))
  fi
}

assert_contains() {
  local haystack=$1 needle=$2 label=$3
  if [[ "$haystack" == *"$needle"* ]]; then
    echo "  PASS: $label"
    PASS=$((PASS+1))
  else
    echo "  FAIL: $label — output did not contain '$needle'"
    echo "    haystack (first 500 chars): ${haystack:0:500}"
    FAIL=$((FAIL+1))
  fi
}

# ---- arg-parsing tests (always run) ----

echo "[script-tests] unknown --providers value exits 3"
out=$("$SCRIPT" --providers=foo --no-build 2>&1) ; rc=$?
assert_eq "$rc" "3" "exit code"
assert_contains "$out" "unknown provider 'foo'" "error message"

echo "[script-tests] --providers=gcs is rejected (descoped)"
out=$("$SCRIPT" --providers=gcs --no-build 2>&1) ; rc=$?
assert_eq "$rc" "3" "exit code"
assert_contains "$out" "unknown provider 'gcs'" "error message"

echo "[script-tests] unknown arg exits 2"
out=$("$SCRIPT" --bogus 2>&1) ; rc=$?
assert_eq "$rc" "2" "exit code"
assert_contains "$out" "unknown arg: --bogus" "error message"

echo "[script-tests] -h prints help and exits 0"
out=$("$SCRIPT" -h 2>&1) ; rc=$?
assert_eq "$rc" "0" "exit code"
assert_contains "$out" "Usage:" "help banner"
assert_contains "$out" "--providers" "documents --providers"

# ---- port-conflict test (always run; doesn't need worker binary) ----

echo "[script-tests] port 49134 in use → exit 3"
# Bind the port in a background listener; cleanup with a trap.
python3 -c '
import socket
import time

with socket.socket() as listener:
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 49134))
    listener.listen()
    time.sleep(60)
' &
NC_PID=$!
trap 'kill "$NC_PID" 2>/dev/null || true' EXIT
for _ in {1..50}; do
  (echo > /dev/tcp/127.0.0.1/49134) 2>/dev/null && break
  sleep 0.1
done
out=$("$SCRIPT" --providers=local --no-build 2>&1) ; rc=$?
kill "$NC_PID" 2>/dev/null || true
wait "$NC_PID" 2>/dev/null || true
trap - EXIT
assert_eq "$rc" "3" "exit code"
assert_contains "$out" "port 49134 already in use" "error message"

# ---- worker-binary-required tests (skipped if binary not built) ----

if [[ ! -x "$WORKER_BIN_TARGET" ]]; then
  echo "[script-tests] SKIP: worker binary not built at $WORKER_BIN_TARGET"
  echo "[script-tests]   skipping --filter and SIGINT tests"
else
  echo "[script-tests] --filter matches zero cases → FAIL"
  out=$("$SCRIPT" --providers=local --no-build --filter=this-string-matches-no-case 2>&1) ; rc=$?
  assert_eq "$rc" "1" "exit code"
  assert_contains "$out" "matched no cases" "FAIL message"

  echo "[script-tests] SIGINT mid-run kills engine + rustfs"
  sigint_root=$(mktemp -d "${TMPDIR:-/tmp}/storage-e2e-sigint.XXXXXX")
  sigint_log="$sigint_root/run.log"
  rustfs_pid_file="$sigint_root/rustfs.pid"
  ( STORAGE_E2E_RUSTFS_PID_FILE="$rustfs_pid_file" \
      "$SCRIPT" --providers=local --no-build >"$sigint_log" 2>&1 ) &
  SCRIPT_PID=$!
  deadline=$(( $(date +%s) + 60 ))
  while [[ ! -s "$rustfs_pid_file" ]] && kill -0 "$SCRIPT_PID" 2>/dev/null; do
    if (( $(date +%s) > deadline )); then break; fi
    sleep 0.2
  done
  rustfs_pid=$(cat "$rustfs_pid_file" 2>/dev/null || true)
  kill -INT "$SCRIPT_PID" 2>/dev/null || true
  wait "$SCRIPT_PID" 2>/dev/null || true
  if (echo > /dev/tcp/127.0.0.1/49134) 2>/dev/null; then
    echo "  FAIL: engine port still open after SIGINT"
    FAIL=$((FAIL+1))
  else
    echo "  PASS: engine port released"
    PASS=$((PASS+1))
  fi
  state=$(ps -o state= -p "$rustfs_pid" 2>/dev/null | tr -d ' ')
  if [[ -n "$rustfs_pid" && -n "$state" && "$state" != "Z" && "$state" != "X" ]]; then
    echo "  FAIL: rustfs still running after SIGINT"
    tail -100 "$sigint_log" 2>/dev/null || true
    FAIL=$((FAIL+1))
  else
    echo "  PASS: owned rustfs process gone"
    PASS=$((PASS+1))
  fi
  rm -rf "$sigint_root"
fi

# ---- result ----

echo
if [[ "$FAIL" -eq 0 ]]; then
  echo "SCRIPT_TESTS_DONE: PASS $PASS/$((PASS+FAIL))"
  exit 0
else
  echo "SCRIPT_TESTS_DONE: FAIL $PASS/$((PASS+FAIL))"
  exit 1
fi
