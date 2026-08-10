#!/usr/bin/env bash
# End-to-end harness for the code-runner worker.
#
# Brings up a private engine on its own port, starts code-runner against it,
# and runs the self-asserting harness. Exits 0 on PASS, 1 on any FAIL.
#
# Covers BOTH engines through one worker, so a case that passes for node and
# fails for python is a case this suite is meant to catch.
#
#   ./run-tests.sh                  # everything
#   ./run-tests.sh --filter run     # only cases whose group/name matches
#   ./run-tests.sh --keep           # leave the engine + worker running
#
# Deliberately uses a random high port, never 49134: a developer's own stack
# usually holds that one, and a harness that hijacks it would take their
# workers down with it.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
HARNESS="$HERE/workers/harness"
LOGS="$HERE/reports"
mkdir -p "$LOGS"

FILTER=""
KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --filter) FILTER="${2:-}"; shift 2 ;;
    --filter=*) FILTER="${1#*=}"; shift ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

III_BIN="${III_BIN:-$(command -v iii || true)}"
if [ -z "$III_BIN" ] || [ ! -x "$III_BIN" ]; then
  echo "FAIL: no iii engine binary. Put it on PATH or set III_BIN=/path/to/iii" >&2
  exit 1
fi
command -v npm >/dev/null 2>&1 || { echo "FAIL: npm not found" >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "FAIL: cargo not found" >&2; exit 1; }

# A free port, verified free rather than assumed: the engine logs "address
# already in use" and then keeps running, so an occupied port produces a
# harness that talks to somebody else's engine and reports nonsense.
pick_port() {
  for _ in $(seq 1 40); do
    local p=$(( (RANDOM % 10000) + 40000 ))
    if ! (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; then echo "$p"; return 0; fi
    exec 3>&- 2>/dev/null || true
  done
  return 1
}
PORT="$(pick_port)" || { echo "FAIL: could not find a free port" >&2; exit 1; }
URL="ws://127.0.0.1:$PORT"
echo "==> engine port $PORT"

ENGINE_PID=""; WORKER_PID=""
cleanup() {
  local code=$?
  if [ "$KEEP" = "1" ]; then
    echo "==> --keep: engine pid ${ENGINE_PID:-none}, worker pid ${WORKER_PID:-none} on $URL"
    return
  fi
  # `|| true` on every guard AND every kill: `set -e` is on by the time this
  # trap runs (turned on right after the harness invocation, and never turned
  # back off), so a false `[ -n ... ]` test or a `kill -9` on a PID that
  # already exited in response to the plain `kill` above — both ordinary,
  # expected outcomes here — would otherwise abort this trap before it ever
  # reaches `exit $code`, reporting the WRONG exit status regardless of what
  # the suite actually did. `2>/dev/null` alone only silences the message,
  # not the exit status `set -e` reacts to.
  [ -n "$WORKER_PID" ] && kill "$WORKER_PID" 2>/dev/null || true
  [ -n "$ENGINE_PID" ] && kill "$ENGINE_PID" 2>/dev/null || true
  sleep 1
  [ -n "$WORKER_PID" ] && kill -9 "$WORKER_PID" 2>/dev/null || true
  [ -n "$ENGINE_PID" ] && kill -9 "$ENGINE_PID" 2>/dev/null || true
  exit $code
}
trap cleanup EXIT INT TERM

echo "==> building code-runner (release)"
( cd "$ROOT/code-runner" && cargo build --release --bin code-runner ) \
  > "$LOGS/build.log" 2>&1 || { echo "FAIL: build failed, see $LOGS/build.log" >&2; tail -30 "$LOGS/build.log"; exit 1; }

RUN_CFG="$LOGS/engine.config.yaml"
sed "s|\${III_E2E_PORT}|$PORT|" "$HERE/config.yaml" > "$RUN_CFG"

echo "==> starting engine"
( cd "$LOGS" && "$III_BIN" -c "$RUN_CFG" --no-update-check ) > "$LOGS/engine.log" 2>&1 &
ENGINE_PID=$!

for i in $(seq 1 60); do
  if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then exec 3>&- ; break; fi
  kill -0 "$ENGINE_PID" 2>/dev/null || { echo "FAIL: engine exited during startup" >&2; tail -30 "$LOGS/engine.log"; exit 1; }
  sleep 0.5
  [ "$i" = "60" ] && { echo "FAIL: engine never listened on $PORT" >&2; tail -30 "$LOGS/engine.log"; exit 1; }
done
echo "==> engine up"

echo "==> starting code-runner worker"
( cd "$ROOT/code-runner" && ./target/release/code-runner --url "$URL" --config "$HERE/code-runner.config.yaml" ) \
  > "$LOGS/code-runner.log" 2>&1 &
WORKER_PID=$!
sleep 1
kill -0 "$WORKER_PID" 2>/dev/null || { echo "FAIL: code-runner exited immediately" >&2; tail -30 "$LOGS/code-runner.log"; exit 1; }

if [ ! -d "$HARNESS/node_modules" ]; then
  echo "==> npm install"
  ( cd "$HARNESS" && npm install ) > "$LOGS/npm.log" 2>&1 \
    || { echo "FAIL: npm install failed, see $LOGS/npm.log" >&2; tail -20 "$LOGS/npm.log"; exit 1; }
fi

echo "==> running suite"
set +e
( cd "$HARNESS" && III_URL="$URL" HARNESS_FILTER="$FILTER" \
    HARNESS_REPORT_PATH="$LOGS/report.json" npm run --silent dev ) 2>&1 | tee "$LOGS/harness.log"
set -e
SUITE_LINE="$(grep -a 'HARNESS_DONE' "$LOGS/harness.log" | tail -1)"

echo
echo "================ code-runner e2e ================"
if [ -z "$SUITE_LINE" ]; then
  echo "FAIL: harness produced no result line (crashed?). See $LOGS/harness.log"
  exit 1
fi
echo "$SUITE_LINE"
echo "report: $LOGS/report.json"
echo "================================================="

case "$SUITE_LINE" in *PASS*) exit 0 ;; *) exit 1 ;; esac
