#!/usr/bin/env bash
# Runs the e2e harness against a given engine binary and worker binary on a
# port of your choosing, without docker: SQLite always, Postgres when
# TEST_POSTGRES_URL points at a reachable server (MySQL cases need
# TEST_MYSQL_URL). Used for the observe transition (0.24 / 0.25 engines).
#
#   run-local.sh <engine-bin> <database-bin> [sqlite_db|pg_db|mysql_db] [full|no-bypass|bypass-only]
#
# Env: PORT (default 49360), OUT (default ./reports/local-<ts>).
set -uo pipefail

ENGINE_BIN="$1"; DB_BIN="$2"; FILTER="${3:-sqlite_db}"; MODE="${4:-full}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${PORT:-49360}"
URL="ws://127.0.0.1:$PORT"
OUT="${OUT:-$ROOT_DIR/reports/local-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$OUT" "$ROOT_DIR/data"
rm -rf "$OUT/configuration" "$ROOT_DIR"/data/iii.db* "$ROOT_DIR/data/configuration"

cat > "$OUT/engine.yaml" <<EOF
workers:
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: $OUT/configuration
      ttl_seconds: 0
  - name: iii-worker-manager
    config:
      port: $PORT
EOF

PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
  sleep 0.5
  for p in "${PIDS[@]}"; do kill -9 "$p" 2>/dev/null; done
}
trap cleanup EXIT

"$ENGINE_BIN" --no-update-check -c "$OUT/engine.yaml" > "$OUT/engine.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 60); do (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null && break; sleep 0.5; done

[[ -d "$ROOT_DIR/workers/harness/node_modules" ]] || (cd "$ROOT_DIR/workers/harness" && npm install --silent)
( cd "$ROOT_DIR/workers/harness" && III_URL=$URL npx tsx src/seed-configuration.ts ) > "$OUT/seed.log" 2>&1 \
  || { echo "seed failed"; tail -20 "$OUT/seed.log"; exit 1; }

( cd "$ROOT_DIR" && exec "$DB_BIN" --url "$URL" ) > "$OUT/database.log" 2>&1 &
PIDS+=($!)
ok=0
for _ in $(seq 1 60); do
  if "$ENGINE_BIN" trigger --engine "$URL" database::query db=sqlite_db sql='SELECT 1' > /dev/null 2>&1; then ok=1; break; fi
  sleep 0.5
done
[[ $ok == 1 ]] || { echo "database worker not ready"; tail -30 "$OUT/database.log"; exit 1; }

( cd "$ROOT_DIR/workers/harness" && HARNESS_FILTER=$FILTER III_URL=$URL HARNESS_REPORT_PATH=$OUT/report.json \
    HARNESS_MODE=$MODE timeout 600 npx tsx src/worker.ts ) > "$OUT/harness.log" 2>&1
grep -E '^\[harness\] (PASS|FAIL)|HARNESS_DONE' "$OUT/harness.log" | sed 's/\x1b\[[0-9;]*m//g' > "$OUT/summary.txt"
grep HARNESS_DONE "$OUT/summary.txt"
grep '^\[harness\] FAIL' "$OUT/summary.txt" | cut -c1-400
grep -q 'HARNESS_DONE: PASS' "$OUT/summary.txt"
