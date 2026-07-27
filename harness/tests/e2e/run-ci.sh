#!/usr/bin/env bash
set -Eeuo pipefail

: "${III_BIN:?III_BIN must point to the pinned iii engine binary}"
: "${ANTHROPIC_API_KEY:?ANTHROPIC_API_KEY is required}"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
harness_root=$(cd -- "$script_dir/../.." && pwd)
repo_root=$(cd -- "$harness_root/.." && pwd)
artifacts_dir=${HARNESS_E2E_ARTIFACTS_DIR:-"$repo_root/target/harness-e2e"}
run_dir="$artifacts_dir/stack"
logs_dir="$artifacts_dir/logs"
iii_port=${HARNESS_E2E_PORT:-49134}
iii_url="ws://127.0.0.1:$iii_port"
subject=${HARNESS_E2E_SUBJECT:-"$script_dir/subjects/ci.json"}

mkdir -p "$run_dir" "$logs_dir"
export HARNESS_E2E_RUN_DIR="$run_dir"

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
  local attempts=240
  local response
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    response=$(
      "$III_BIN" trigger router::models::get \
        --port "$iii_port" \
        --json '{"provider":"anthropic","id":"claude-sonnet-4-6"}' 2>/dev/null || true
    )
    if jq -e '.model.id == "claude-sonnet-4-6"' <<<"$response" >/dev/null; then
      return 0
    fi
    sleep 0.5
  done
  echo "timed out waiting for the Anthropic E2E model" >&2
  return 1
}

start_process engine "$III_BIN" -c "$harness_root/engine.config.yaml"
wait_for_function engine::workers::list

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

start_process provider-anthropic \
  "$repo_root/provider-anthropic/target/release/provider-anthropic" \
  --url "$iii_url"
wait_for_function provider::anthropic::stream
wait_for_model

start_process context-manager \
  "$repo_root/context-manager/target/release/context-manager" \
  --url "$iii_url" \
  --config "$script_dir/stack-config/context-manager.yaml"
wait_for_function context::build

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

"$harness_root/target/release/harness-e2e" \
  --url "$iii_url" \
  --subject "$subject" \
  --output "$artifacts_dir/results"
