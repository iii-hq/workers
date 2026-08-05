#!/usr/bin/env bash
set -Eeuo pipefail

# Run the checkout's E2E runner against workers resolved from the public registry.

: "${HARNESS_E2E_RELEASE_WORKER:?HARNESS_E2E_RELEASE_WORKER is required}"
: "${HARNESS_E2E_RELEASE_VERSION:?HARNESS_E2E_RELEASE_VERSION is required}"
: "${HARNESS_E2E_MODEL:?HARNESS_E2E_MODEL is required}"
: "${HARNESS_E2E_PROVIDER:?HARNESS_E2E_PROVIDER is required}"
: "${HARNESS_E2E_JUDGE_MODEL:?HARNESS_E2E_JUDGE_MODEL is required}"
: "${HARNESS_E2E_JUDGE_PROVIDER:?HARNESS_E2E_JUDGE_PROVIDER is required}"
: "${HARNESS_E2E_SCENARIO:?HARNESS_E2E_SCENARIO is required}"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
harness_root=$(cd -- "$script_dir/../.." && pwd)
repo_root=$(cd -- "$harness_root/.." && pwd)
artifact_dir=${HARNESS_E2E_ARTIFACTS_DIR:-"$repo_root/target/harness-e2e"}
e2e_bin=${HARNESS_E2E_BIN:-"$harness_root/target/release/harness-e2e"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
cli_channel=${III_CLI_CHANNEL:-latest}
worker_tag=${III_WORKER_TAG:-latest}
stack_versions=${HARNESS_E2E_STACK_VERSIONS:-'{}'}
runs=${HARNESS_E2E_RUNS:-1}
engine_port=49134
wait_seconds=180
add_timeout_seconds=600

if [[ -n "${III_CHANNEL:-}" ]]; then
  echo "III_CHANNEL was split into III_CLI_CHANNEL and III_WORKER_TAG" >&2
  exit 2
fi

case "$cli_channel" in
  latest | next) ;;
  *)
    echo "III_CLI_CHANNEL must be latest or next" >&2
    exit 2
    ;;
esac
[[ "$worker_tag" =~ ^[A-Za-z0-9._-]+$ ]] || {
  echo "III_WORKER_TAG must be a valid Registry tag" >&2
  exit 2
}
stack_versions=$(jq -c \
  --arg worker "$HARNESS_E2E_RELEASE_WORKER" \
  --arg version "$HARNESS_E2E_RELEASE_VERSION" '
    if length == 0 then {($worker): $version} else . end
  ' <<<"$stack_versions")
jq -e --arg worker "$HARNESS_E2E_RELEASE_WORKER" --arg version "$HARNESS_E2E_RELEASE_VERSION" '
  type == "object" and length > 0 and
  all(to_entries[];
    (.key | test("^[a-z0-9][a-z0-9_-]*$")) and
    (.value | type == "string" and test("^[0-9]+\\.[0-9]+\\.[0-9]+(-(experimental|alpha|beta))?$"))
  ) and .[$worker] == $version
' <<<"$stack_versions" >/dev/null || {
  echo "HARNESS_E2E_STACK_VERSIONS must contain the release worker and strict exact versions" >&2
  exit 2
}
[[ -x "$e2e_bin" ]] || {
  echo "Harness E2E binary is not executable: $e2e_bin" >&2
  exit 2
}

rm -rf "$artifact_dir/logs" "$artifact_dir/stack" "$artifact_dir/results"
rm -f "$artifact_dir/deployment.json" "$artifact_dir/cli-version.txt"
mkdir -p "$artifact_dir/logs" "$artifact_dir/stack" "$artifact_dir/results"
artifact_dir=$(cd "$artifact_dir" && pwd)
log_dir="$artifact_dir/logs"
stack_dir="$artifact_dir/stack"
run_root=$(mktemp -d "${TMPDIR:-/tmp}/harness-e2e-deployed.XXXXXX")
project_dir="$run_root/project"
e2e_home="$run_root/home"
mkdir -p "$project_dir" "$e2e_home"

export HOME="$e2e_home"
export XDG_CONFIG_HOME="$e2e_home/.config"
export PATH="$e2e_home/.local/bin:$e2e_home/.iii/bin:$PATH"
export III_CLI_CHANNEL="$cli_channel"
export III_WORKER_TAG="$worker_tag"

iii_bin=""
engine_pid=""
failure_reason=""
failure_phase=bootstrap
cli_version=unknown
actual_release_version=""
started_at_seconds=$SECONDS

log() {
  printf '\n[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

die() {
  failure_reason=$1
  printf '\n[FAIL] %s\n' "$failure_reason" >&2
  return 1
}

write_deployment_result() {
  local status=$1
  jq -n \
    --arg status "$status" \
    --arg reason "$failure_reason" \
    --arg phase "$failure_phase" \
    --arg cli_version "$cli_version" \
    --arg cli_channel "$cli_channel" \
    --arg worker_tag "$worker_tag" \
    --arg release_worker "$HARNESS_E2E_RELEASE_WORKER" \
    --arg release_version "$HARNESS_E2E_RELEASE_VERSION" \
    --arg actual_release_version "$actual_release_version" \
    --arg release_tag "${HARNESS_E2E_RELEASE_TAG:-}" \
    --arg release_run_id "${HARNESS_E2E_RELEASE_RUN_ID:-}" \
    --arg smoke_run_id "${HARNESS_E2E_SMOKE_RUN_ID:-}" \
    --argjson stack_versions "$stack_versions" \
    --argjson elapsed_ms "$(((SECONDS - started_at_seconds) * 1000))" \
    '{
      status: $status,
      failure_reason: $reason,
      failure_phase: $phase,
      cli_version: $cli_version,
      cli_channel: $cli_channel,
      worker_tag: $worker_tag,
      release_worker: $release_worker,
      release_version: $release_version,
      actual_release_version: $actual_release_version,
      release_tag: $release_tag,
      release_run_id: $release_run_id,
      smoke_run_id: $smoke_run_id,
      stack_versions: $stack_versions,
      elapsed_ms: $elapsed_ms
    }' >"$artifact_dir/deployment.json"
}

snapshot_stack() {
  for output in config.yaml iii.lock workers.json; do
    [[ -f "$project_dir/$output" ]] && cp "$project_dir/$output" "$stack_dir/$output"
  done
}

stop_workers() {
  [[ -n "$iii_bin" && -n "$engine_pid" && -f "$project_dir/config.yaml" ]] || return 0
  kill -0 "$engine_pid" 2>/dev/null || return 0

  mapfile -t workers < <(python3 - "$project_dir/config.yaml" <<'PY'
import sys
from pathlib import Path
import yaml

config = yaml.safe_load(Path(sys.argv[1]).read_text()) or {}
for worker in config.get("workers") or []:
    if isinstance(worker, dict) and worker.get("name"):
        print(worker["name"])
PY
  )
  ((${#workers[@]} > 0)) || return 0
  (cd "$project_dir" && "$iii_bin" worker remove -y "${workers[@]}") \
    >"$log_dir/worker-remove.log" 2>&1 || true
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

stop_owned_orphans() {
  mapfile -t pids < <(ps -eo pid=,args= | awk -v root="$run_root/" '
    {
      pid = $1
      $1 = ""
      sub(/^[[:space:]]+/, "")
      if (index($0, root) == 1) print pid
    }
  ')
  ((${#pids[@]} > 0)) || return 0
  kill -TERM "${pids[@]}" 2>/dev/null || true
  sleep 1
  kill -KILL "${pids[@]}" 2>/dev/null || true
}

# Worker-level logs are the only view into wake/binding delivery; the engine
# log alone cannot explain a lost notification. Collected before teardown so
# failed scenario jobs ship them in the diagnostics artifact.
collect_worker_logs() {
  [[ -n "$iii_bin" && -n "$engine_pid" ]] || return 0
  kill -0 "$engine_pid" 2>/dev/null || return 0
  local name
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    (cd "$project_dir" && timeout 20 "$iii_bin" worker logs "$name") \
      >"$log_dir/worker-$name.log" 2>&1 || true
  done < <(python3 - "$project_dir/config.yaml" <<'PY'
import sys
from pathlib import Path
import yaml

config = yaml.safe_load(Path(sys.argv[1]).read_text()) or {}
for worker in config.get("workers") or []:
    if isinstance(worker, dict) and worker.get("name"):
        print(worker["name"])
PY
  )
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM ERR
  set +e
  snapshot_stack
  collect_worker_logs
  stop_workers
  stop_engine
  stop_owned_orphans
  if ((status == 0)); then
    write_deployment_result passed
  else
    [[ -n "$failure_reason" ]] || failure_reason="deployed E2E failed during $failure_phase (exit $status)"
    write_deployment_result failed
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
    kill -0 "$engine_pid" 2>/dev/null || die "engine exited before becoming ready"
    response=$("$iii_bin" trigger engine::workers::list --port "$engine_port" \
      --json '{}' 2>>"$log_dir/discovery.log" || true)
    jq -e '.workers != null' <<<"$response" >/dev/null 2>&1 && return 0
    sleep 1
  done
  die "engine did not become ready within ${wait_seconds}s"
}

wait_for_functions() {
  local required response missing
  required=$(printf '%s\n' "$@" | jq -R . | jq -s .)
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    missing=$(jq -r --argjson required "$required" '
      (.functions // [] | map(.function_id)) as $available
      | ($required - $available) | join(" ")
    ' <<<"$response" 2>/dev/null || printf 'function discovery failed')
    [[ -z "$missing" ]] && return 0
    sleep 1
  done
  die "required functions did not register: $missing"
}

wait_for_triggers() {
  local required response missing
  required=$(printf '%s\n' "$@" | jq -R . | jq -s .)
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger engine::triggers::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    missing=$(jq -r --argjson required "$required" '
      (.triggers // [] | map(.id)) as $available
      | ($required - $available) | join(" ")
    ' <<<"$response" 2>/dev/null || printf 'trigger discovery failed')
    [[ -z "$missing" ]] && return 0
    sleep 1
  done
  die "required trigger types did not register: $missing"
}

wait_for_model() {
  local provider=$1 model=$2 request response
  request=$(jq -cn --arg provider "$provider" --arg id "$model" '{provider: $provider, id: $id}')
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger router::models::get --port "$engine_port" \
      --json "$request" 2>>"$log_dir/discovery.log" || true)
    jq -e --arg id "$model" '.model.id == $id' <<<"$response" >/dev/null 2>&1 && return 0
    sleep 1
  done
  die "model $provider/$model did not resolve within ${wait_seconds}s"
}

log "Installing iii from $cli_channel"
curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 \
  "$install_url" -o "$run_root/install.sh"
if [[ "$cli_channel" == next ]]; then
  sh "$run_root/install.sh" --next 2>&1 | tee "$log_dir/install.log"
else
  sh "$run_root/install.sh" 2>&1 | tee "$log_dir/install.log"
fi
iii_bin=$(command -v iii)
cli_version=$("$iii_bin" --version 2>&1)
printf '%s\n' "$cli_version" >"$artifact_dir/cli-version.txt"

printf 'workers: []\n' >"$project_dir/config.yaml"
(cd "$project_dir" && exec setsid "$iii_bin" -c config.yaml --no-update-check) \
  >"$log_dir/engine.log" 2>&1 &
engine_pid=$!
wait_for_engine

workers=("harness@$worker_tag" "database@$worker_tag" "fp@$worker_tag" "web@$worker_tag")
declare -A providers=()
for provider in "$HARNESS_E2E_PROVIDER" "$HARNESS_E2E_JUDGE_PROVIDER"; do
  if [[ -z "${providers[$provider]:-}" ]]; then
    workers+=("provider-$provider@$worker_tag")
    providers[$provider]=1
  fi
done

# The registry's /resolve sits at ~7s per call and a stack install issues
# dozens of them; one transient network blip fails the whole add. Retry the
# full command — worker add is idempotent (re-adds are no-ops).
add_with_retry() {
  local label=$1; shift
  local attempt
  for attempt in 1 2 3; do
    if (cd "$project_dir" && timeout --signal=TERM --kill-after=15s "$add_timeout_seconds" \
      "$iii_bin" worker add "$@") 2>&1 | tee -a "$log_dir/$label.log"; then
      return 0
    fi
    log "worker add ($label) failed on attempt $attempt; retrying in 15s"
    sleep 15
  done
  return 1
}

log "Installing registry stack: ${workers[*]}"
add_with_retry worker-add "${workers[@]}"

while IFS=$'\t' read -r candidate_worker candidate_version; do
  log "Installing exact stack candidate: ${candidate_worker}@${candidate_version}"
  add_with_retry "candidate-${candidate_worker}" \
    "${candidate_worker}@${candidate_version}" --force
done < <(jq -r 'to_entries | sort_by(.key)[] | [.key, .value] | @tsv' <<<"$stack_versions")

wait_for_functions \
  harness::send harness::status worker::add database::query state::get \
  queue::define session::messages context::assemble router::models::get \
  directory::skills::list
wait_for_triggers cron
wait_for_model "$HARNESS_E2E_PROVIDER" "$HARNESS_E2E_MODEL"
wait_for_model "$HARNESS_E2E_JUDGE_PROVIDER" "$HARNESS_E2E_JUDGE_MODEL"

"$iii_bin" trigger engine::workers::list --port "$engine_port" \
  --json '{}' >"$project_dir/workers.json"

verify_args=(
  --lock "$project_dir/iii.lock"
  --manifest "$harness_root/iii.worker.yaml"
  --required harness
  --required database
  --worker "$HARNESS_E2E_RELEASE_WORKER"
  --version "$HARNESS_E2E_RELEASE_VERSION"
  --expected-versions-json "$stack_versions"
  --output "$stack_dir/lock-verification.json"
)
for worker in "${workers[@]}"; do
  verify_args+=(--required "${worker%@*}")
done
verification=$(python3 "$repo_root/.github/scripts/verify_registry_lock.py" "${verify_args[@]}")
actual_release_version=$(jq -r '.actual_version' <<<"$verification")

failure_phase=e2e
export HARNESS_E2E_RUN_DIR="$project_dir"
export HARNESS_E2E_ENGINE_REVISION="$cli_version"
"$e2e_bin" run \
  --url "ws://127.0.0.1:$engine_port" \
  --model "$HARNESS_E2E_MODEL" \
  --provider "$HARNESS_E2E_PROVIDER" \
  --judge-model "$HARNESS_E2E_JUDGE_MODEL" \
  --judge-provider "$HARNESS_E2E_JUDGE_PROVIDER" \
  --output "$artifact_dir/results" \
  --scenario "$HARNESS_E2E_SCENARIO" \
  --runs "$runs"
