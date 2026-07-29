#!/usr/bin/env bash
set -Eeuo pipefail

: "${III_BIN:?III_BIN must point to the pinned iii engine binary}"
: "${HARNESS_E2E_SCENARIO:?HARNESS_E2E_SCENARIO is required}"
: "${HARNESS_E2E_MODEL:?HARNESS_E2E_MODEL is required}"
: "${HARNESS_E2E_PROVIDER:?HARNESS_E2E_PROVIDER is required}"
: "${HARNESS_E2E_JUDGE_MODEL:?HARNESS_E2E_JUDGE_MODEL is required}"
: "${HARNESS_E2E_JUDGE_PROVIDER:?HARNESS_E2E_JUDGE_PROVIDER is required}"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
harness_root=$(cd -- "$script_dir/../.." && pwd)
repo_root=$(cd -- "$harness_root/.." && pwd)
artifacts_dir=${HARNESS_E2E_ARTIFACTS_DIR:-"$repo_root/target/harness-e2e"}
e2e_bin=${HARNESS_E2E_BIN:-"$harness_root/target/release/harness-e2e"}
runs=${HARNESS_E2E_RUNS:-1}
run_dir="$artifacts_dir/stack"
logs_dir="$artifacts_dir/logs"
iii_port=${HARNESS_E2E_PORT:-49134}
[[ "$iii_port" =~ ^[0-9]+$ ]] && ((iii_port >= 1 && iii_port <= 65535)) || {
  echo "HARNESS_E2E_PORT must be an integer from 1 through 65535" >&2
  exit 1
}
iii_url="ws://127.0.0.1:$iii_port"
engine_config_template=${HARNESS_E2E_ENGINE_CONFIG:-"$script_dir/stack-config/engine.yaml"}
# iii-worker's lifecycle helpers mutate config.yaml in their project root.
# Use that same isolated path for the engine reload watcher.
engine_config="$run_dir/config.yaml"
database_config=${HARNESS_E2E_DATABASE_CONFIG:-"$script_dir/stack-config/database.yaml"}

mkdir -p "$run_dir" "$logs_dir" "$run_dir/database"
cp "$engine_config_template" "$engine_config"
# The engine expands environment placeholders itself, but iii-worker reads the
# same project file directly and expects the manager port to already be numeric.
sed -i "s/\${HARNESS_E2E_PORT}/$iii_port/g" "$engine_config"
export HARNESS_E2E_RUN_DIR="$run_dir"
export HARNESS_E2E_PORT="$iii_port"
# Built-in iii-worker daemons otherwise fall back to the fixed 49134 default,
# which breaks isolated runs that select a different port.
export III_ENGINE_URL="$iii_url"

pids=()

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ((${#pids[@]} > 0)); then
    kill "${pids[@]}" 2>/dev/null || true
    wait "${pids[@]}" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

start_process() {
  local name=$1
  shift
  (
    cd "$run_dir"
    exec "$@"
  ) >"$logs_dir/$name.log" 2>&1 &
  pids+=("$!")
}

wait_for_function() {
  local function_id=$1
  local attempts=120
  local response
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    response=$(
      "$III_BIN" trigger engine::functions::list \
        --port "$iii_port" \
        --json '{"include_internal":true}' 2>/dev/null || true
    )
    if jq -e --arg id "$function_id" \
      '.functions[]? | select(.function_id == $id)' \
      <<<"$response" >/dev/null; then
      return 0
    fi
    sleep 0.5
  done
  echo "timed out waiting for function $function_id" >&2
  return 1
}

wait_for_trigger_type() {
  local trigger_type=$1
  local attempts=120
  local response
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    response=$(
      "$III_BIN" trigger engine::triggers::list \
        --port "$iii_port" \
        --json '{"include_internal":true}' 2>/dev/null || true
    )
    if jq -e --arg id "$trigger_type" \
      '.triggers[]? | select(.id == $id)' \
      <<<"$response" >/dev/null; then
      return 0
    fi
    sleep 0.5
  done
  echo "timed out waiting for trigger type $trigger_type" >&2
  return 1
}

wait_for_model() {
  local provider=$1
  local model=$2
  local attempts=240
  local response
  local request
  request=$(jq -cn --arg provider "$provider" --arg id "$model" \
    '{provider:$provider,id:$id}')
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    response=$(
      "$III_BIN" trigger router::models::get \
        --port "$iii_port" \
        --json "$request" 2>/dev/null || true
    )
    if jq -e --arg id "$model" '.model.id == $id' <<<"$response" >/dev/null; then
      return 0
    fi
    sleep 0.5
  done
  echo "timed out waiting for E2E model $provider/$model" >&2
  return 1
}

start_provider() {
  local provider=$1
  [[ "$provider" =~ ^[A-Za-z0-9_-]+$ ]] || {
    echo "unsupported provider id: $provider" >&2
    return 1
  }
  local worker="provider-$provider"
  local binary="$repo_root/$worker/target/release/$worker"
  [[ -x "$binary" ]] || {
    echo "provider binary is not executable: $binary" >&2
    return 1
  }
  start_process "$worker" "$binary" --url "$iii_url"
  wait_for_function "provider::$provider::stream"
}

start_process engine "$III_BIN" -c "$engine_config"
wait_for_function engine::workers::list
wait_for_function worker::add

start_process database \
  "$repo_root/database/target/release/database" \
  --url "$iii_url" \
  --config "$database_config"
wait_for_function database::query
wait_for_trigger_type database::row-changed

start_process state \
  "$repo_root/state/target/release/state" \
  --url "$iii_url" \
  --config "$script_dir/stack-config/state.yaml"
wait_for_function state::get

start_process queue \
  "$repo_root/queue/target/release/queue" \
  --url "$iii_url" \
  --config "$script_dir/stack-config/queue.yaml"
wait_for_function queue::define

start_process session-manager \
  "$repo_root/session-manager/target/release/session-manager" \
  --url "$iii_url" \
  --config "$script_dir/stack-config/session-manager.yaml"
wait_for_function session::messages

start_process llm-router \
  "$repo_root/llm-router/target/release/llm-router" \
  --url "$iii_url"
wait_for_function router::models::get

declare -A started_providers=()
for provider in "$HARNESS_E2E_PROVIDER" "$HARNESS_E2E_JUDGE_PROVIDER"; do
  if [[ -z "${started_providers[$provider]:-}" ]]; then
    start_provider "$provider"
    started_providers[$provider]=1
  fi
done
wait_for_model "$HARNESS_E2E_PROVIDER" "$HARNESS_E2E_MODEL"
wait_for_model "$HARNESS_E2E_JUDGE_PROVIDER" "$HARNESS_E2E_JUDGE_MODEL"

start_process context-manager \
  "$repo_root/context-manager/target/release/context-manager" \
  --url "$iii_url" \
  --config "$script_dir/stack-config/context-manager.yaml"
wait_for_function context::assemble

start_process iii-directory \
  "$repo_root/iii-directory/target/release/iii-directory" \
  --url "$iii_url"
wait_for_function directory::skills::list

start_process cron \
  "$repo_root/cron/target/release/cron" \
  --url "$iii_url"
wait_for_trigger_type cron

start_process harness \
  "$harness_root/target/release/harness" \
  --url "$iii_url"
wait_for_function harness::send

"$e2e_bin" run \
  --url "$iii_url" \
  --model "$HARNESS_E2E_MODEL" \
  --provider "$HARNESS_E2E_PROVIDER" \
  --judge-model "$HARNESS_E2E_JUDGE_MODEL" \
  --judge-provider "$HARNESS_E2E_JUDGE_PROVIDER" \
  --output "$artifacts_dir/results" \
  --scenario "$HARNESS_E2E_SCENARIO" \
  --runs "$runs"
