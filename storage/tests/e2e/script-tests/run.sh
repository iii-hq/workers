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
# Bind the port in a background nc listener; cleanup with a trap.
( nc -l 49134 </dev/null >/dev/null 2>&1 ) &
NC_PID=$!
trap "kill $NC_PID 2>/dev/null || true" EXIT
sleep 0.5
out=$("$SCRIPT" --providers=local --no-build 2>&1) ; rc=$?
kill "$NC_PID" 2>/dev/null || true
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
  ( "$SCRIPT" --providers=local --no-build >/tmp/sigint-test.log 2>&1 ) &
  SCRIPT_PID=$!
  # Give the engine ~5s to start.
  sleep 5
  kill -INT "$SCRIPT_PID" 2>/dev/null || true
  wait "$SCRIPT_PID" 2>/dev/null || true
  sleep 2
  # Both processes should be gone.
  # Zombies (<defunct> / state Z) are "dead enough" for this test's
  # intent: the process has been killed, the kernel just hasn't reaped
  # the /proc entry yet (parent died alongside it, init reaps when
  # scheduled). `pgrep -f` still matches a zombie's comm, so we need to
  # filter on process state explicitly. `ps -o state=` prints the state
  # column; "Z" / "X" mean dead. (No `xargs -r` because BSD xargs on
  # macOS doesn't have it.)
  is_alive() {
    local pat=$1 pid state
    for pid in $(pgrep -f "$pat" 2>/dev/null); do
      state=$(ps -o state= -p "$pid" 2>/dev/null | tr -d ' ')
      [[ -n "$state" && "$state" != "Z" && "$state" != "X" ]] && return 0
    done
    return 1
  }
  if is_alive 'target/release/storage'; then
    echo "  FAIL: storage worker still running after SIGINT"
    FAIL=$((FAIL+1))
  else
    echo "  PASS: storage gone"
    PASS=$((PASS+1))
  fi
  if is_alive rustfs; then
    echo "  FAIL: rustfs still running after SIGINT"
    echo "  --- /tmp/sigint-test.log (last 100 lines) ---"
    tail -100 /tmp/sigint-test.log 2>/dev/null || echo "  (log not present)"
    echo "  --- live rustfs/storage/iii processes ---"
    ps -eo pid,ppid,pgid,sid,stat,comm,args 2>/dev/null | awk 'NR==1 || /rustfs|storage|iii/' | head -20
    echo "  --- end diagnostic ---"
    FAIL=$((FAIL+1))
  else
    echo "  PASS: rustfs gone"
    PASS=$((PASS+1))
  fi
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
