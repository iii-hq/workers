#!/usr/bin/env bash
set -Eeuo pipefail

: "${HARNESS_E2E_EXECUTION_CONTRACT:?HARNESS_E2E_EXECUTION_CONTRACT is required}"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../../.." && pwd)
contract_tool="$repo_root/.github/scripts/harness_e2e_shadow_contract.py"
artifact_dir=${HARNESS_E2E_ARTIFACTS_DIR:-"$repo_root/target/harness-e2e-shadow"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
engine_port=${HARNESS_E2E_ENGINE_PORT:-49134}
wait_seconds=${HARNESS_E2E_WAIT_SECONDS:-180}
run_timeout_seconds=${HARNESS_E2E_RUN_TIMEOUT_SECONDS:-10800}

case "$artifact_dir" in
  "$repo_root"/target/*) ;;
  *) echo "HARNESS_E2E_ARTIFACTS_DIR must be below $repo_root/target" >&2; exit 2 ;;
esac
mkdir -p "$artifact_dir"
artifact_dir=$(cd "$artifact_dir" && pwd)
contract_path="$artifact_dir/execution-contract.json"
printf '%s\n' "$HARNESS_E2E_EXECUTION_CONTRACT" >"$contract_path"
python3 "$contract_tool" validate --contract "$contract_path" >/dev/null

contract_schema=$(jq -r '.schema_version' "$contract_path")
if [[ "$contract_schema" == 2 ]]; then
  stack_versions=$(jq -c '.target.stack.resolved_versions' "$contract_path")
  stack_digest=$(jq -r '.target.stack.resolution_sha256' "$contract_path")
  runtime_versions=$(jq -c '.runtime.stack_versions' "$contract_path")
  cli_version=$(jq -r '.runtime.cli.version' "$contract_path")
  cli_channel=""
else
  stack_versions=$(jq -c '.target.stack_versions' "$contract_path")
  stack_digest=$(jq -r '.target.stack_digest' "$contract_path")
  runtime_versions='{}'
  cli_version=""
  cli_channel=${III_CLI_CHANNEL:-latest}
  case "$cli_channel" in latest | next) ;; *) echo "III_CLI_CHANNEL must be latest or next" >&2; exit 2 ;; esac
fi
runner_worker=$(jq -r '.runner.registry_worker' "$contract_path")
runner_ref=$(jq -r '.runner.registry_ref' "$contract_path")
subject_provider=$(jq -r '.plan.definition.subject.provider' "$contract_path")
judge_provider=$(jq -r '.plan.definition.judge.provider' "$contract_path")
seed=$(jq -r '.plan.definition.seed' "$contract_path")

run_root=$(mktemp -d "${TMPDIR:-/tmp}/harness-e2e-shadow.XXXXXX")
project_dir="$run_root/project"
project_config="$project_dir/iii.config.yaml"
e2e_home="$run_root/home"
mkdir -p "$project_dir" "$e2e_home" "$artifact_dir/logs" "$artifact_dir/stack"

export HOME="$e2e_home"
export XDG_CONFIG_HOME="$e2e_home/.config"
export PATH="$e2e_home/.local/bin:$e2e_home/.iii/bin:$PATH"
export HARNESS_E2E_STACK_MODE=registry
export HARNESS_E2E_STACK_VERSIONS="$stack_versions"
export HARNESS_E2E_STACK_DIGEST="$stack_digest"
export HARNESS_E2E_DATA_DIR="$run_root/e2e-data"
export HARNESS_E2E_RUN_DIR="$project_dir"
export HARNESS_E2E_LANE=release-control-shadow

iii_bin=""
engine_pid=""
failure_phase=bootstrap
failure_reason=""

log() {
  printf '\n[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

fail() {
  failure_reason=$1
  printf '[FAIL] %s\n' "$failure_reason" >&2
  return 1
}

snapshot_stack() {
  for file in iii.config.yaml iii.lock workers.json; do
    [[ -f "$project_dir/$file" ]] && cp "$project_dir/$file" "$artifact_dir/stack/$file"
  done
  if [[ -f "$project_dir/iii.lock" ]]; then
    sha256sum "$project_dir/iii.lock" | awk '{print "sha256:" $1}' >"$artifact_dir/stack/iii-lock.sha256"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM ERR
  set +e
  snapshot_stack
  if [[ -n "$engine_pid" ]] && kill -0 "$engine_pid" 2>/dev/null; then
    kill -- "-$engine_pid" 2>/dev/null || kill "$engine_pid" 2>/dev/null || true
    wait "$engine_pid" 2>/dev/null || true
  fi
  if ((status != 0)); then
    [[ -n "$failure_reason" ]] || failure_reason="shadow execution failed during $failure_phase (exit $status)"
    jq -n --arg phase "$failure_phase" --arg error "$failure_reason" --argjson exit_code "$status" \
      '{schema:"e2e-shadow-failure/v1",phase:$phase,outcome:"infra_failed",error:$error,exit_code:$exit_code}' \
      >"$artifact_dir/failure.json"
  fi
  rm -rf "$run_root"
  exit "$status"
}

on_error() {
  local status=$? line=$1
  [[ -n "$failure_reason" ]] || failure_reason="command failed during $failure_phase at line $line (exit $status)"
  return "$status"
}

trap 'on_error "$LINENO"' ERR
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

wait_for_engine() {
  local response
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    kill -0 "$engine_pid" 2>/dev/null || fail "iii engine exited before becoming ready"
    response=$("$iii_bin" trigger engine::workers::list --port "$engine_port" --json '{}' 2>/dev/null || true)
    jq -e '.workers != null' <<<"$response" >/dev/null 2>&1 && return 0
    sleep 1
  done
  fail "iii engine did not become ready within ${wait_seconds}s"
}

wait_for_functions() {
  local required response missing
  required=$(printf '%s\n' "$@" | jq -R . | jq -s .)
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>/dev/null || true)
    missing=$(jq -r --argjson required "$required" \
      '(.functions // [] | map(.function_id)) as $available | ($required - $available) | join(" ")' \
      <<<"$response" 2>/dev/null || printf 'function discovery failed')
    [[ -z "$missing" ]] && return 0
    sleep 1
  done
  fail "required functions did not register: $missing"
}

add_with_retry() {
  local label=$1
  shift
  for attempt in 1 2 3; do
    if (cd "$project_dir" && timeout --signal=TERM --kill-after=15s 600 "$iii_bin" worker add -y "$@") \
      2>&1 | tee -a "$artifact_dir/logs/$label.log"; then
      return 0
    fi
    log "worker add ($label) failed on attempt $attempt"
    sleep 15
  done
  return 1
}

install_exact_stack() {
  local label=$1
  local versions_json=$2
  local -a pins=()
  mapfile -t pins < <(jq -r 'to_entries | sort_by(.key)[] | "\(.key)@\(.value)"' <<<"$versions_json")
  ((${#pins[@]} > 0)) || return 0
  log "Installing ${#pins[@]} exact workers ($label)"
  add_with_retry "$label" "${pins[@]}" --force
}

if [[ "$contract_schema" == 2 ]]; then
  log "Installing exact iii CLI $cli_version"
else
  log "Installing iii CLI from $cli_channel"
fi
curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 "$install_url" -o "$run_root/install.sh"
if [[ "$contract_schema" == 2 ]]; then
  VERSION="$cli_version" sh "$run_root/install.sh" 2>&1 | tee "$artifact_dir/logs/install.log"
elif [[ "$cli_channel" == next ]]; then
  sh "$run_root/install.sh" --next 2>&1 | tee "$artifact_dir/logs/install.log"
else
  sh "$run_root/install.sh" 2>&1 | tee "$artifact_dir/logs/install.log"
fi
iii_bin=$(command -v iii)
observed_cli_version=$("$iii_bin" --version 2>&1)
printf '%s\n' "$observed_cli_version" >"$artifact_dir/iii-version.txt"
if [[ "$contract_schema" == 2 && "$observed_cli_version" != *"$cli_version"* ]]; then
  fail "iii CLI version mismatch: expected $cli_version, observed $observed_cli_version"
fi
export HARNESS_E2E_ENGINE_REVISION="$observed_cli_version"

printf 'workers: []\n' >"$project_config"
(cd "$project_dir" && exec setsid "$iii_bin" -c iii.config.yaml --no-update-check) \
  >"$artifact_dir/logs/engine.log" 2>&1 &
engine_pid=$!
wait_for_engine

failure_phase=registry
if [[ "$contract_schema" == 2 ]]; then
  # The contract may distinguish runtime and target pins. Resolve their union
  # once before the runner and once after it, rather than waiting for every
  # individual worker to report ready in four serial loops.
  exact_stack_versions=$(jq -cn \
    --argjson runtime "$runtime_versions" \
    --argjson target "$stack_versions" \
    '$runtime + $target')
  install_exact_stack stack-bootstrap "$exact_stack_versions"
else
  support=("database@latest" "storage@latest" "fp@latest" "web@latest")
  declare -A providers=()
  for provider in "$subject_provider" "$judge_provider"; do
    [[ -n "${providers[$provider]:-}" ]] && continue
    support+=("provider-$provider@latest")
    providers[$provider]=1
  done
  log "Installing E2E support workers"
  add_with_retry support "${support[@]}"
fi

if [[ "$contract_schema" != 2 ]]; then
  while IFS=$'\t' read -r worker version; do
    log "Installing exact target stack: $worker@$version"
    add_with_retry "stack-$worker" "$worker@$version" --force
  done < <(jq -r 'to_entries | sort_by(.key)[] | [.key,.value] | @tsv' <<<"$stack_versions")
fi

log "Installing ephemeral runner: $runner_worker@$runner_ref"
add_with_retry runner "$runner_worker@$runner_ref" --force

# A runner dependency may resolve a different version of a runtime or target
# worker. Reapply every exact pin after the runner.
if [[ "$contract_schema" == 2 ]]; then
  install_exact_stack stack-repin "$exact_stack_versions"
else
  while IFS=$'\t' read -r worker version; do
    add_with_retry "repin-$worker" "$worker@$version" --force
  done < <(jq -r 'to_entries | sort_by(.key)[] | [.key,.value] | @tsv' <<<"$stack_versions")
fi

target_harness_version=$(jq -r '.target.version' "$contract_path")
if [[ "$contract_schema" == 2 ]]; then
  python3 "$contract_tool" verify-lock \
    --contract "$contract_path" \
    --lock "$project_dir/iii.lock" \
    --output "$artifact_dir/stack/stack-manifest.json"
else
  python3 "$repo_root/.github/scripts/verify_registry_lock.py" \
    --lock "$project_dir/iii.lock" \
    --worker harness \
    --version "$target_harness_version" \
    --required harness \
    --required "$runner_worker" \
    --required database \
    --required storage \
    --expected-versions-json "$stack_versions" \
    --output "$artifact_dir/stack/lock-verification.json" >/dev/null
fi

wait_for_functions \
  e2e::scenarios-list e2e::run e2e::status e2e::results-get \
  harness::send harness::status router::models::get
"$iii_bin" trigger engine::workers::list --port "$engine_port" --json '{}' >"$project_dir/workers.json"

failure_phase=materialization
"$iii_bin" trigger e2e::scenarios-list --port "$engine_port" \
  --json "$(jq -cn --argjson seed "$seed" '{seed:$seed}')" >"$artifact_dir/catalog.json"
python3 "$contract_tool" materialize \
  --contract "$contract_path" \
  --catalog "$artifact_dir/catalog.json" \
  --output "$artifact_dir/run-request.json"

failure_phase=execution
timeout --signal=TERM --kill-after=30s 60 \
  "$iii_bin" trigger e2e::run --port "$engine_port" \
  --json "$(jq -c . "$artifact_dir/run-request.json")" >"$artifact_dir/accepted.json"
remote_execution_id=$(jq -er '.execution_id | select(type == "string" and length > 0)' "$artifact_dir/accepted.json")
printf '%s\n' "$remote_execution_id" >"$artifact_dir/remote-execution-id.txt"

started_at=$SECONDS
poll_index=0
while true; do
  timeout --signal=TERM --kill-after=30s 60 \
    "$iii_bin" trigger e2e::status --port "$engine_port" \
    --json "$(jq -cn --arg execution_id "$remote_execution_id" '{execution_id:$execution_id}')" \
    >"$artifact_dir/status.json"
  jq -e --arg id "$remote_execution_id" '.execution_id == $id' "$artifact_dir/status.json" >/dev/null
  [[ "$(jq -r '.terminal // false' "$artifact_dir/status.json")" == true ]] && break
  if ((SECONDS - started_at >= run_timeout_seconds)); then
    fail "E2E execution exceeded ${run_timeout_seconds}s"
  fi
  case "$poll_index" in 0) delay=2 ;; 1) delay=5 ;; 2) delay=10 ;; *) delay=30 ;; esac
  poll_index=$((poll_index + 1))
  sleep "$delay"
done

failure_phase=results
timeout --signal=TERM --kill-after=30s 120 \
  "$iii_bin" trigger e2e::results-get --port "$engine_port" \
  --json "$(jq -cn --arg execution_id "$remote_execution_id" '{execution_id:$execution_id}')" \
  >"$artifact_dir/results.json"
jq -e --arg id "$remote_execution_id" '.execution_id == $id' "$artifact_dir/results.json" >/dev/null

# This proves the archive contract while the runner is alive. GitHub Artifact,
# not this ephemeral local storage, is the D0 retention boundary.
if "$iii_bin" trigger e2e::archive --port "$engine_port" \
  --json "$(jq -cn --arg execution_id "$remote_execution_id" '{execution_id:$execution_id,retention_class:"longitudinal"}')" \
  >"$artifact_dir/archive.json" 2>"$artifact_dir/logs/archive.log"; then
  "$iii_bin" trigger e2e::archive-head --port "$engine_port" \
    --json "$(jq -cn --arg execution_id "$remote_execution_id" '{execution_id:$execution_id}')" \
    >"$artifact_dir/archive-head.json" 2>>"$artifact_dir/logs/archive.log" || true
fi

failure_phase=complete
