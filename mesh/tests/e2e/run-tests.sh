#!/usr/bin/env bash
set -euo pipefail
TASK_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
: "${III_BIN:?set III_BIN to the real iii engine executable}"
export III_BIN
export MESH_TEST_PROXY_BIN=${MESH_TEST_PROXY_BIN:-$TASK_ROOT/rbac-proxy/target/debug/rbac-proxy}
export MESH_TEST_FP_BIN=${MESH_TEST_FP_BIN:-$TASK_ROOT/fp/target/debug/fp}
export MESH_TEST_HARNESS_BIN=${MESH_TEST_HARNESS_BIN:-$TASK_ROOT/harness/target/debug/harness}
export MESH_TEST_STATE_BIN=${MESH_TEST_STATE_BIN:-$TASK_ROOT/state/target/debug/state}
export MESH_TEST_QUEUE_BIN=${MESH_TEST_QUEUE_BIN:-$TASK_ROOT/queue/target/debug/queue}
export MESH_E2E_REPORT_DIR=${MESH_E2E_REPORT_DIR:-$TASK_ROOT/mesh/tests/e2e/reports}
mkdir -p "$MESH_E2E_REPORT_DIR"
chmod 700 "$MESH_E2E_REPORT_DIR"
export MESH_TEST_SESSION_BIN=${MESH_TEST_SESSION_BIN:-$TASK_ROOT/session-manager/target/debug/session-manager}
export CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local TS_NO_LOGS_NO_SUPPORT=true
for task_binary in "$III_BIN" "$MESH_TEST_PROXY_BIN" "$MESH_TEST_FP_BIN" "$MESH_TEST_HARNESS_BIN" "$MESH_TEST_STATE_BIN" "$MESH_TEST_QUEUE_BIN" "$MESH_TEST_SESSION_BIN"; do
  [[ -x "$task_binary" ]] || { echo "Missing executable: $task_binary" >&2; exit 1; }
done
cd "$TASK_ROOT/mesh"
go test -tags=e2e,ts_omit_peerapiserver ./tests/e2e -count=1 -timeout=15m -v "$@"
