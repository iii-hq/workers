#!/usr/bin/env bash
set -Eeuo pipefail

# Run Harness E2E scenarios against workers resolved from the public registry.
# The test runner is built from this checkout, but every runtime worker comes
# from the installer and `iii worker add` path used by consumers.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
harness_root=$(cd -- "$script_dir/../.." && pwd)
repo_root=$(cd -- "$harness_root/.." && pwd)
artifact_dir=${HARNESS_E2E_ARTIFACTS_DIR:-"$repo_root/target/harness-e2e"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
channel=${III_CHANNEL:-latest}
release_worker=${HARNESS_E2E_RELEASE_WORKER:-}
release_version=${HARNESS_E2E_RELEASE_VERSION:-}
release_tag=${HARNESS_E2E_RELEASE_TAG:-}
release_run_id=${HARNESS_E2E_RELEASE_RUN_ID:-}
smoke_run_id=${HARNESS_E2E_SMOKE_RUN_ID:-}
subject_model=${HARNESS_E2E_MODEL:?HARNESS_E2E_MODEL is required}
subject_provider=${HARNESS_E2E_PROVIDER:?HARNESS_E2E_PROVIDER is required}
judge_model=${HARNESS_E2E_JUDGE_MODEL:?HARNESS_E2E_JUDGE_MODEL is required}
judge_provider=${HARNESS_E2E_JUDGE_PROVIDER:?HARNESS_E2E_JUDGE_PROVIDER is required}
e2e_bin=${HARNESS_E2E_BIN:-"$harness_root/target/release/harness-e2e"}
scenario=${HARNESS_E2E_SCENARIO:?HARNESS_E2E_SCENARIO is required}
runs=${HARNESS_E2E_RUNS:-1}
engine_port=${HARNESS_E2E_PORT:-49134}
wait_seconds=${HARNESS_E2E_WAIT_SECONDS:-180}
add_timeout_seconds=${HARNESS_E2E_ADD_TIMEOUT_SECONDS:-600}

case "$channel" in
  latest | next) ;;
  *)
    echo "III_CHANNEL must be 'latest' or 'next' (got: $channel)" >&2
    exit 2
    ;;
esac
export III_CHANNEL="$channel"

[[ -n "$release_worker" ]] || {
  echo "HARNESS_E2E_RELEASE_WORKER is required" >&2
  exit 2
}
[[ -n "$release_version" ]] || {
  echo "HARNESS_E2E_RELEASE_VERSION is required" >&2
  exit 2
}
[[ "$release_worker" =~ ^[A-Za-z0-9_-]+$ ]] || {
  echo "invalid release worker: $release_worker" >&2
  exit 2
}
[[ "$subject_provider" =~ ^[A-Za-z0-9_-]+$ ]] || {
  echo "invalid subject provider: $subject_provider" >&2
  exit 2
}
[[ "$judge_provider" =~ ^[A-Za-z0-9_-]+$ ]] || {
  echo "invalid judge provider: $judge_provider" >&2
  exit 2
}
[[ "$runs" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_E2E_RUNS must be a positive integer" >&2
  exit 2
}
runs=$((10#$runs))
((runs > 0)) || {
  echo "HARNESS_E2E_RUNS must be a positive integer" >&2
  exit 2
}
[[ "$engine_port" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_E2E_PORT must be an integer" >&2
  exit 2
}
engine_port=$((10#$engine_port))
((engine_port >= 1 && engine_port <= 65535)) || {
  echo "HARNESS_E2E_PORT must be between 1 and 65535" >&2
  exit 2
}
[[ "$wait_seconds" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_E2E_WAIT_SECONDS must be a positive integer" >&2
  exit 2
}
wait_seconds=$((10#$wait_seconds))
((wait_seconds > 0)) || {
  echo "HARNESS_E2E_WAIT_SECONDS must be a positive integer" >&2
  exit 2
}
[[ "$add_timeout_seconds" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_E2E_ADD_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 2
}
add_timeout_seconds=$((10#$add_timeout_seconds))
((add_timeout_seconds > 0)) || {
  echo "HARNESS_E2E_ADD_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 2
}

for command_name in curl jq python3; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "$command_name is required" >&2
    exit 2
  }
done
python3 -c 'import yaml' >/dev/null 2>&1 || {
  echo "python3 with PyYAML is required" >&2
  exit 2
}
[[ -x "$e2e_bin" ]] || {
  echo "Harness E2E binary is not executable: $e2e_bin" >&2
  exit 2
}

mkdir -p "$artifact_dir/logs" "$artifact_dir/stack" "$artifact_dir/results"
artifact_dir=$(cd "$artifact_dir" && pwd)
log_dir="$artifact_dir/logs"
stack_artifact_dir="$artifact_dir/stack"

run_root=$(mktemp -d "${TMPDIR:-/tmp}/harness-e2e-deployed.XXXXXX")
project_dir="$run_root/project"
e2e_home="$run_root/home"
mkdir -p "$project_dir" "$e2e_home"

export HOME="$e2e_home"
export XDG_CONFIG_HOME="$e2e_home/.config"
export PATH="$e2e_home/.local/bin:$e2e_home/.iii/bin:$PATH"

iii_url="ws://127.0.0.1:$engine_port"
engine_pid=""
iii_bin=""
failure_reason=""
failure_phase="bootstrap"
cli_version="unknown"
actual_release_version=""
started_at_seconds=$SECONDS

log() {
  printf '\n[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

log_command() {
  local rendered="$*"
  if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
    rendered=${rendered//"$ANTHROPIC_API_KEY"/[REDACTED]}
  fi
  if [[ -n "${OPENAI_API_KEY:-}" ]]; then
    rendered=${rendered//"$OPENAI_API_KEY"/[REDACTED]}
  fi
  if [[ -n "${ZAI_API_KEY:-}" ]]; then
    rendered=${rendered//"$ZAI_API_KEY"/[REDACTED]}
  fi
  printf '  $ %s\n' "$rendered" >&2
  printf '$ %s\n' "$rendered" >>"$artifact_dir/commands.log"
}

ok() {
  printf '  [ok] %s\n' "$*" >&2
}

die() {
  failure_reason=$1
  return 1
}

write_deployment_result() {
  local status=$1
  jq -n \
    --arg status "$status" \
    --arg reason "$failure_reason" \
    --arg phase "$failure_phase" \
    --arg cli_version "$cli_version" \
    --arg channel "$channel" \
    --arg release_worker "$release_worker" \
    --arg release_version "$release_version" \
    --arg actual_release_version "$actual_release_version" \
    --arg release_tag "$release_tag" \
    --arg release_run_id "$release_run_id" \
    --arg smoke_run_id "$smoke_run_id" \
    --arg subject_model "$subject_model" \
    --arg subject_provider "$subject_provider" \
    --arg judge_model "$judge_model" \
    --arg judge_provider "$judge_provider" \
    --argjson elapsed_ms "$(((SECONDS - started_at_seconds) * 1000))" \
    --argjson engine_port "$engine_port" \
    '{
      status: $status,
      failure_reason: $reason,
      failure_phase: $phase,
      cli_version: $cli_version,
      channel: $channel,
      release_worker: $release_worker,
      release_version: $release_version,
      actual_release_version: $actual_release_version,
      release_tag: $release_tag,
      release_run_id: $release_run_id,
      smoke_run_id: $smoke_run_id,
      subject: {model: $subject_model, provider: $subject_provider},
      judge: {model: $judge_model, provider: $judge_provider},
      elapsed_ms: $elapsed_ms,
      engine_port: $engine_port
    }' >"$artifact_dir/deployment.json"
}

stop_engine() {
  [[ -n "$engine_pid" ]] && kill -0 "$engine_pid" 2>/dev/null || return 0

  kill -- "-$engine_pid" 2>/dev/null || kill "$engine_pid" 2>/dev/null || true
  for _ in {1..20}; do
    kill -0 "$engine_pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL -- "-$engine_pid" 2>/dev/null || kill -KILL "$engine_pid" 2>/dev/null || true
  wait "$engine_pid" 2>/dev/null || true
}

stop_managed_workers() {
  [[ -n "$iii_bin" && -f "$project_dir/config.yaml" ]] || return 0
  kill -0 "$engine_pid" 2>/dev/null || return 0

  local worker_names
  worker_names=$(python3 - "$project_dir/config.yaml" <<'PY'
import sys
from pathlib import Path

import yaml

config = yaml.safe_load(Path(sys.argv[1]).read_text()) or {}
for worker in config.get("workers") or []:
    if isinstance(worker, dict) and worker.get("name"):
        print(worker["name"])
PY
  )
  [[ -n "$worker_names" ]] || return 0
  (
    cd "$project_dir"
    "$iii_bin" worker remove -y $worker_names
  ) >"$log_dir/worker-remove.log" 2>&1 || true
}

stop_orphaned_processes() {
  local pids
  pids=$(ps -eo pid=,args= | awk -v root="$run_root/" '
    {
      pid = $1
      $1 = ""
      sub(/^[[:space:]]+/, "")
      if (index($0, root) == 1) print pid
    }
  ')
  [[ -n "$pids" ]] || return 0
  kill -TERM $pids 2>/dev/null || true
  sleep 1
  kill -KILL $pids 2>/dev/null || true
}

snapshot_stack_files() {
  for output in config.yaml iii.lock; do
    [[ -f "$project_dir/$output" ]] && cp "$project_dir/$output" "$stack_artifact_dir/$output"
  done
  [[ -f "$harness_root/iii.worker.yaml" ]] && cp "$harness_root/iii.worker.yaml" "$stack_artifact_dir/harness.iii.worker.yaml"
  if [[ -f "$project_dir/workers.json" ]]; then
    cp "$project_dir/workers.json" "$stack_artifact_dir/workers.json"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM ERR
  set +e

  snapshot_stack_files
  stop_managed_workers
  stop_engine
  stop_orphaned_processes

  if ((status == 0)); then
    write_deployment_result passed
  else
    [[ -n "$failure_reason" ]] || failure_reason="deployed E2E failed during $failure_phase (exit $status)"
    write_deployment_result failed
    echo "deployed E2E failed: $failure_reason" >&2
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
  local response attempt
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    kill -0 "$engine_pid" 2>/dev/null || die "engine exited before becoming ready"
    response=$("$iii_bin" trigger engine::workers::list --port "$engine_port" \
      --json '{}' 2>>"$log_dir/discovery.log" || true)
    if jq -e '.workers != null' <<<"$response" >/dev/null 2>&1; then
      ok "engine ready after ${attempt}s"
      return 0
    fi
    sleep 1
  done
  die "engine did not become ready within ${wait_seconds}s"
}

wait_for_function() {
  local function_id=$1 response attempt
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    if jq -e --arg id "$function_id" \
      'any(.functions[]?; .function_id == $id)' \
      <<<"$response" >/dev/null 2>&1; then
      ok "$function_id registered after ${attempt}s"
      return 0
    fi
    if ((attempt > 0 && attempt % 15 == 0)); then
      log "Still waiting for $function_id"
    fi
    sleep 1
  done
  die "$function_id did not register within ${wait_seconds}s"
}

wait_for_trigger_type() {
  local trigger_type=$1 response attempt
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger engine::triggers::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    if jq -e --arg id "$trigger_type" \
      'any(.triggers[]?; .id == $id)' \
      <<<"$response" >/dev/null 2>&1; then
      ok "$trigger_type registered after ${attempt}s"
      return 0
    fi
    sleep 1
  done
  die "$trigger_type did not register within ${wait_seconds}s"
}

wait_for_model() {
  local provider=$1 model=$2 request response attempt
  request=$(jq -cn --arg provider "$provider" --arg id "$model" \
    '{provider: $provider, id: $id}')
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger router::models::get --port "$engine_port" \
      --json "$request" 2>>"$log_dir/discovery.log" || true)
    if jq -e --arg id "$model" '.model.id == $id' <<<"$response" >/dev/null 2>&1; then
      ok "model $provider/$model resolved after ${attempt}s"
      return 0
    fi
    sleep 1
  done
  die "model $provider/$model did not resolve within ${wait_seconds}s"
}

run_worker_add() {
  local workers=("$@")
  log_command "iii worker add ${workers[*]}"
  if command -v timeout >/dev/null 2>&1; then
    (
      cd "$project_dir"
      timeout --signal=TERM --kill-after=15s "$add_timeout_seconds" \
        "$iii_bin" worker add "${workers[@]}"
    ) 2>&1 | tee "$log_dir/worker-add.log"
  else
    (
      cd "$project_dir"
      "$iii_bin" worker add "${workers[@]}"
    ) 2>&1 | tee "$log_dir/worker-add.log"
  fi
}

verify_lock() {
  local verification
  if ! verification=$(python3 - \
      "$project_dir/iii.lock" \
      "$harness_root/iii.worker.yaml" \
      "$release_worker" \
      "$release_version" <<'PY'
import json
import sys
from pathlib import Path

import yaml

lock_path = Path(sys.argv[1])
manifest_path = Path(sys.argv[2])
release_worker = sys.argv[3]
expected_version = sys.argv[4]
lock = yaml.safe_load(lock_path.read_text()) or {}
manifest = yaml.safe_load(manifest_path.read_text()) or {}
workers = lock.get("workers") or {}

record = workers.get(release_worker)
if not isinstance(record, dict):
    raise SystemExit(f"artifact_mismatch: {release_worker} is absent from iii.lock")

actual_version = str(record.get("version", ""))
if actual_version != expected_version:
    raise SystemExit(
        f"artifact_mismatch: expected {release_worker} {expected_version}, resolved {actual_version or 'unknown'}"
    )

required = {"harness", *(manifest.get("dependencies") or {}).keys()}
missing = sorted(required - workers.keys())
if missing:
    raise SystemExit(f"stack_incomplete: mandatory Harness workers absent from iii.lock: {', '.join(missing)}")

print(json.dumps({"actual_release_version": actual_version, "required_workers": sorted(required)}))
PY
  ); then
    failure_reason="registry lock verification failed"
    return 1
  fi
  actual_release_version=$(jq -er '.actual_release_version' <<<"$verification")
  printf '%s\n' "$verification" >"$stack_artifact_dir/lock-verification.json"
  ok "iii.lock resolves $release_worker $actual_release_version and all mandatory Harness workers"
}

log "Installing iii from $install_url (channel=$channel)"
log_command "curl -fsSL $install_url -o install.sh"
curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 \
  "$install_url" -o "$run_root/install.sh"
if [[ "$channel" == "next" ]]; then
  log_command "sh install.sh --next"
  sh "$run_root/install.sh" --next 2>&1 | tee "$log_dir/install.log"
else
  log_command "sh install.sh"
  sh "$run_root/install.sh" 2>&1 | tee "$log_dir/install.log"
fi
iii_bin=$(command -v iii || true)
[[ -n "$iii_bin" && -x "$iii_bin" ]] || die "iii CLI was not installed"
log_command "iii --version"
cli_version=$("$iii_bin" --version 2>&1)
printf '%s\n' "$cli_version" >"$artifact_dir/cli-version.txt"
ok "installed $cli_version"

printf 'workers: []\n' >"$project_dir/config.yaml"
log_command "iii -c config.yaml --no-update-check"
(
  cd "$project_dir"
  if command -v setsid >/dev/null 2>&1; then
    setsid "$iii_bin" -c config.yaml --no-update-check
  else
    exec "$iii_bin" -c config.yaml --no-update-check
  fi
) >"$log_dir/engine.log" 2>&1 &
engine_pid=$!
wait_for_engine

workers=(harness database)
declare -A requested_providers=()
for provider in "$subject_provider" "$judge_provider"; do
  if [[ -z "${requested_providers[$provider]:-}" ]]; then
    workers+=("provider-$provider")
    requested_providers[$provider]=1
  fi
done

run_worker_add "${workers[@]}"
wait_for_function harness::send
wait_for_function harness::status
wait_for_function worker::add
wait_for_function database::query
wait_for_function state::get
wait_for_function queue::define
wait_for_function session::messages
wait_for_function context::assemble
wait_for_function router::models::get
wait_for_function directory::skills::list
wait_for_trigger_type database::row-change
wait_for_trigger_type cron
wait_for_model "$subject_provider" "$subject_model"
wait_for_model "$judge_provider" "$judge_model"

"$iii_bin" trigger engine::workers::list --port "$engine_port" \
  --json '{}' >"$project_dir/workers.json"
verify_lock

failure_phase=e2e
export HARNESS_E2E_RUN_DIR="$project_dir"
export HARNESS_E2E_ENGINE_REVISION="$cli_version"
log_command "$e2e_bin run --url $iii_url --model $subject_model --provider $subject_provider --scenario $scenario --runs $runs"
"$e2e_bin" run \
  --url "$iii_url" \
  --model "$subject_model" \
  --provider "$subject_provider" \
  --judge-model "$judge_model" \
  --judge-provider "$judge_provider" \
  --output "$artifact_dir/results" \
  --scenario "$scenario" \
  --runs "$runs"
