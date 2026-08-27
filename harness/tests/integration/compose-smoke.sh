#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../../.." && pwd)
PROFILE=${INTEGRATION_PROFILE:-release}
III_BIN=${III_BIN:-${1:-}}
COMPOSE_TEMPLATE="$SCRIPT_DIR/compose-smoke.yaml"
SMOKE_ROOT=${COMPOSE_SMOKE_ROOT:-$REPO_ROOT/target/integration-compose-smoke}
COMPOSE_FILE="$SMOKE_ROOT/worker-compose.yaml"
PYTHON=${PYTHON:-python3}
NAMESPACE=integration-compose-smoke

[[ -n "$III_BIN" && -x "$III_BIN" ]] || {
  echo "III_BIN must point to an executable iii release"
  exit 3
}

export QUEUE_BIN=${QUEUE_BIN:-$REPO_ROOT/queue/target/$PROFILE/queue}
export III_DIRECTORY_BIN=${III_DIRECTORY_BIN:-$REPO_ROOT/iii-directory/target/$PROFILE/iii-directory}
export SESSION_MANAGER_BIN=${SESSION_MANAGER_BIN:-$REPO_ROOT/session-manager/target/$PROFILE/session-manager}
export CONTEXT_MANAGER_BIN=${CONTEXT_MANAGER_BIN:-$REPO_ROOT/context-manager/target/$PROFILE/context-manager}
export STATE_BIN=${STATE_BIN:-$REPO_ROOT/state/target/$PROFILE/state}
export DATABASE_BIN=${DATABASE_BIN:-$REPO_ROOT/database/target/$PROFILE/database}
export QUEUE_WORKER_DIR="$REPO_ROOT/queue"
export III_DIRECTORY_WORKER_DIR="$REPO_ROOT/iii-directory"
export SESSION_MANAGER_WORKER_DIR="$REPO_ROOT/session-manager"
export CONTEXT_MANAGER_WORKER_DIR="$REPO_ROOT/context-manager"
export STATE_WORKER_DIR="$REPO_ROOT/state"
export DATABASE_WORKER_DIR="$REPO_ROOT/database"

for binary in \
  "$QUEUE_BIN" \
  "$III_DIRECTORY_BIN" \
  "$SESSION_MANAGER_BIN" \
  "$CONTEXT_MANAGER_BIN" \
  "$STATE_BIN" \
  "$DATABASE_BIN"; do
  [[ -x "$binary" ]] || { echo "missing Compose smoke binary: $binary"; exit 3; }
done

mkdir -p \
  "$SMOKE_ROOT/config" \
  "$SMOKE_ROOT/skills" \
  "$SMOKE_ROOT/local-skills" \
  "$SMOKE_ROOT/agents" \
  "$SMOKE_ROOT/global-agents" \
  "$SMOKE_ROOT/agents-skills" \
  "$SMOKE_ROOT/global-agents-skills" \
  "$SMOKE_ROOT/sessions" \
  "$SMOKE_ROOT/leases" \
  "$SMOKE_ROOT/state"

port=$(
  "$PYTHON" -c \
    'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
)
export COMPOSE_ENGINE_PORT=$port
export COMPOSE_ENGINE_URL="ws://127.0.0.1:$port"
export COMPOSE_CONFIG_DIR="$SMOKE_ROOT/config"
export COMPOSE_QUEUE_PATH="$SMOKE_ROOT/queue.json"
export COMPOSE_SKILLS_DIR="$SMOKE_ROOT/skills"
export COMPOSE_LOCAL_SKILLS_DIR="$SMOKE_ROOT/local-skills"
export COMPOSE_AGENTS_DIR="$SMOKE_ROOT/agents"
export COMPOSE_GLOBAL_AGENTS_DIR="$SMOKE_ROOT/global-agents"
export COMPOSE_AGENTS_SKILLS_DIR="$SMOKE_ROOT/agents-skills"
export COMPOSE_GLOBAL_AGENTS_SKILLS_DIR="$SMOKE_ROOT/global-agents-skills"
export COMPOSE_SESSION_DIR="$SMOKE_ROOT/sessions"
export COMPOSE_LEASES_DIR="$SMOKE_ROOT/leases"
export COMPOSE_DATABASE_PATH="$SMOKE_ROOT/database.sqlite"
export III_COMPOSE_STATE_DIR="$SMOKE_ROOT/state"
export COMPOSE_LLVM_PROFILE_FILE=${COMPOSE_LLVM_PROFILE_FILE:-}
export COMPOSE_TEMPLATE COMPOSE_FILE

"$PYTHON" <<'PY'
import os
import re
from pathlib import Path

template = Path(os.environ["COMPOSE_TEMPLATE"]).read_text()
names = (
    "COMPOSE_ENGINE_PORT",
    "COMPOSE_ENGINE_URL",
    "COMPOSE_CONFIG_DIR",
    "COMPOSE_QUEUE_PATH",
    "COMPOSE_SKILLS_DIR",
    "COMPOSE_LOCAL_SKILLS_DIR",
    "COMPOSE_AGENTS_DIR",
    "COMPOSE_GLOBAL_AGENTS_DIR",
    "COMPOSE_AGENTS_SKILLS_DIR",
    "COMPOSE_GLOBAL_AGENTS_SKILLS_DIR",
    "COMPOSE_SESSION_DIR",
    "COMPOSE_LEASES_DIR",
    "COMPOSE_DATABASE_PATH",
    "COMPOSE_LLVM_PROFILE_FILE",
    "QUEUE_BIN",
    "III_DIRECTORY_BIN",
    "SESSION_MANAGER_BIN",
    "CONTEXT_MANAGER_BIN",
    "STATE_BIN",
    "DATABASE_BIN",
    "QUEUE_WORKER_DIR",
    "III_DIRECTORY_WORKER_DIR",
    "SESSION_MANAGER_WORKER_DIR",
    "CONTEXT_MANAGER_WORKER_DIR",
    "STATE_WORKER_DIR",
    "DATABASE_WORKER_DIR",
)
for name in names:
    template = template.replace(f"@{name}@", os.environ[name])
if re.search(r"@[A-Z0-9_]+@", template):
    raise SystemExit("compose smoke template contains an unresolved placeholder")
Path(os.environ["COMPOSE_FILE"]).write_text(template)
PY

compose_log="$SMOKE_ROOT/compose.log"
compose_pid=

cleanup() {
  exit_code=$?
  trap - EXIT
  set +e
  if [[ -n "$compose_pid" ]] && kill -0 "$compose_pid" 2>/dev/null; then
    kill -TERM "$compose_pid" 2>/dev/null
    for _ in {1..60}; do
      kill -0 "$compose_pid" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "$compose_pid" 2>/dev/null; then
      kill -KILL "$compose_pid" 2>/dev/null
    fi
    wait "$compose_pid" 2>/dev/null
  fi
  if ((exit_code != 0)); then
    echo "iii compose smoke failed; captured output follows"
    sed -n '1,240p' "$compose_log" 2>/dev/null
  fi
  exit "$exit_code"
}
trap cleanup EXIT

III_TELEMETRY_ENABLED=false "$III_BIN" compose --up --file "$COMPOSE_FILE" \
  >"$compose_log" 2>&1 &
compose_pid=$!

status_file="$SMOKE_ROOT/compose-status.json"
deadline=$((SECONDS + 45))
while true; do
  if timeout --signal=KILL 5s "$III_BIN" trigger compose::status \
    --port "$port" \
    --namespace "$NAMESPACE" \
    --json "{\"file\":\"$COMPOSE_FILE\"}" >"$status_file" 2>/dev/null \
    && jq -e '
      ([.containers[].container] | sort) ==
        ["context-manager", "database", "iii-directory", "queue", "session-manager", "state"]
      and all(.containers[]; .state == "ready" and .owned == true)
    ' "$status_file" >/dev/null; then
    break
  fi
  if ! kill -0 "$compose_pid" 2>/dev/null; then
    echo "iii compose exited before the stack became ready"
    exit 1
  fi
  if ((SECONDS >= deadline)); then
    echo "iii compose did not make every worker ready before the deadline"
    exit 1
  fi
  sleep 1
done

timeout --signal=KILL 5s "$III_BIN" trigger state::set \
  --port "$port" \
  --namespace "$NAMESPACE" \
  --json '{"scope":"compose-smoke","key":"ready","value":true}' \
  >"$SMOKE_ROOT/state-set.json"
timeout --signal=KILL 5s "$III_BIN" trigger state::get \
  --port "$port" \
  --namespace "$NAMESPACE" \
  --json '{"scope":"compose-smoke","key":"ready"}' \
  >"$SMOKE_ROOT/state-get.json"
jq -e '. == true' "$SMOKE_ROOT/state-get.json" >/dev/null

timeout --signal=KILL 10s "$III_BIN" trigger compose::down \
  --port "$port" \
  --namespace "$NAMESPACE" \
  --json "{\"file\":\"$COMPOSE_FILE\"}" \
  >"$SMOKE_ROOT/compose-down.json"
jq -e '
  .status == "ok"
  and ([.containers[].container] | sort) ==
    ["context-manager", "database", "iii-directory", "queue", "session-manager", "state"]
  and all(.containers[]; .state == "stopped")
' "$SMOKE_ROOT/compose-down.json" >/dev/null

if kill -0 "$compose_pid" 2>/dev/null; then
  kill -TERM "$compose_pid"
fi
for _ in {1..60}; do
  kill -0 "$compose_pid" 2>/dev/null || break
  sleep 0.25
done
if kill -0 "$compose_pid" 2>/dev/null; then
  echo "iii compose did not stop after SIGTERM"
  exit 1
fi
wait "$compose_pid" || true
compose_pid=
trap - EXIT

echo "iii compose started, verified, and stopped all integration workers"
