#!/usr/bin/env bash
set -Eeuo pipefail

# Validate the documented install path against the published registry. This
# intentionally does not start a provider or make a model request: the goal is
# to prove that a fresh project can install and boot the harness stack.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../../.." && pwd)
artifact_dir=${HARNESS_QUICKSTART_ARTIFACTS_DIR:-"$repo_root/target/harness-quickstart"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
engine_port=${HARNESS_QUICKSTART_ENGINE_PORT:-49134}
wait_seconds=${HARNESS_QUICKSTART_WAIT_SECONDS:-180}
add_timeout_seconds=${HARNESS_QUICKSTART_ADD_TIMEOUT_SECONDS:-600}

[[ "$engine_port" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_QUICKSTART_ENGINE_PORT must be numeric" >&2
  exit 2
}
[[ "$wait_seconds" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_QUICKSTART_WAIT_SECONDS must be numeric" >&2
  exit 2
}

mkdir -p "$artifact_dir"
artifact_dir=$(cd "$artifact_dir" && pwd)

run_root=$(mktemp -d "${TMPDIR:-/tmp}/harness-quickstart.XXXXXX")
home_dir="$run_root/home"
project_dir="$run_root/project"
log_dir="$artifact_dir/logs"
mkdir -p "$home_dir" "$project_dir" "$log_dir"

export HOME="$home_dir"
export XDG_CONFIG_HOME="$home_dir/.config"
export PATH="$HOME/.local/bin:$HOME/.iii/bin:$PATH"

engine_pid=""
failure_reason=""
cli_version="unknown"

now_ms() {
  local value
  value=$(date +%s%3N 2>/dev/null || true)
  if [[ "$value" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$value"
  else
    python3 -c 'import time; print(time.time_ns() // 1_000_000)'
  fi
}

started_at_ms=$(now_ms)

write_result() {
  local status=$1
  local finished_at_ms elapsed_ms
  finished_at_ms=$(now_ms)
  elapsed_ms=$((finished_at_ms - started_at_ms))

  if command -v jq >/dev/null 2>&1; then
    jq -n \
      --arg status "$status" \
      --arg reason "${failure_reason:-}" \
      --arg cli_version "$cli_version" \
      --arg install_url "$install_url" \
      --argjson elapsed_ms "$elapsed_ms" \
      --argjson engine_port "$engine_port" \
      '{status: $status, failure_reason: $reason, cli_version: $cli_version,
        install_url: $install_url, elapsed_ms: $elapsed_ms,
        engine_port: $engine_port}' \
      >"$artifact_dir/result.json"
  else
    printf '{"status":"%s","failure_reason":"%s"}\n' \
      "$status" "${failure_reason:-}" >"$artifact_dir/result.json"
  fi
}

stop_engine() {
  if [[ -n "$engine_pid" ]] && kill -0 "$engine_pid" 2>/dev/null; then
    # The engine is started in its own process group so workers started by the
    # CLI cannot survive the validator after the job exits.
    kill -- "-$engine_pid" 2>/dev/null || kill "$engine_pid" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$engine_pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL -- "-$engine_pid" 2>/dev/null || kill -KILL "$engine_pid" 2>/dev/null || true
    wait "$engine_pid" 2>/dev/null || true
  fi
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM ERR
  set +e

  stop_engine

  if [[ -f "$project_dir/config.yaml" ]]; then
    cp "$project_dir/config.yaml" "$artifact_dir/config.yaml"
  fi
  if [[ -f "$project_dir/iii.lock" ]]; then
    cp "$project_dir/iii.lock" "$artifact_dir/iii.lock"
  fi

  if ((status == 0)); then
    write_result passed
  else
    write_result failed
    echo "quickstart validator failed: ${failure_reason:-exit $status}" >&2
  fi

  rm -rf "$run_root"
  exit "$status"
}

on_error() {
  local status=$? line=$1
  if [[ -z "$failure_reason" ]]; then
    failure_reason="command failed at line $line (exit $status)"
  fi
  return "$status"
}

trap 'on_error "$LINENO"' ERR
trap cleanup EXIT INT TERM

die() {
  failure_reason=$1
  return 1
}

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v jq >/dev/null 2>&1 || die "jq is required"

wait_for_engine() {
  local response
  for _ in $(seq 1 "$wait_seconds"); do
    if ! kill -0 "$engine_pid" 2>/dev/null; then
      die "engine exited before becoming ready"
    fi
    response=$("$iii_bin" trigger engine::workers::list --port "$engine_port" \
      --json '{}' 2>>"$log_dir/discovery.log" || true)
    if jq -e '.workers != null' <<<"$response" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  die "engine did not become ready within ${wait_seconds}s"
}

wait_for_functions() {
  local response missing function_id
  local required_functions=(
    harness::send
    harness::status
    queue::define
    session::messages
    context::assemble
    router::models::get
    console::status
  )

  for _ in $(seq 1 "$wait_seconds"); do
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    missing=""
    for function_id in "${required_functions[@]}"; do
      if ! jq -e --arg id "$function_id" \
        '(.functions // []) | any(.[]; .function_id == $id)' \
        <<<"$response" >/dev/null 2>&1; then
        missing+=" $function_id"
      fi
    done
    if [[ -z "$missing" ]]; then
      return 0
    fi
    sleep 1
  done
  die "required functions did not register:${missing:- unknown}"
}

run_add() {
  if command -v timeout >/dev/null 2>&1; then
    timeout --signal=TERM --kill-after=15s "$add_timeout_seconds" \
      "$iii_bin" worker add harness console
  else
    "$iii_bin" worker add harness console
  fi
}

start_engine() {
  if command -v setsid >/dev/null 2>&1; then
    setsid "$iii_bin" -c "$project_dir/config.yaml" --no-update-check \
      >"$log_dir/engine.log" 2>&1 &
  else
    "$iii_bin" -c "$project_dir/config.yaml" --no-update-check \
      >"$log_dir/engine.log" 2>&1 &
  fi
  engine_pid=$!
}

mkdir -p "$project_dir"
cd "$project_dir"

curl -fsSL --retry 3 --retry-connrefused --retry-delay 5 \
  "$install_url" -o "$run_root/install.sh"
sh "$run_root/install.sh" >"$log_dir/install.log" 2>&1

iii_bin=$(command -v iii || true)
[[ -n "$iii_bin" && -x "$iii_bin" ]] || die "iii CLI was not installed"
cli_version=$("$iii_bin" --version 2>&1)
printf '%s\n' "$cli_version" >"$artifact_dir/cli-version.txt"

printf 'workers: []\n' >config.yaml
start_engine
wait_for_engine

run_add >"$log_dir/worker-add.log" 2>&1
wait_for_functions

console_status=$("$iii_bin" trigger console::status --port "$engine_port" \
  --json '{}' 2>"$log_dir/console-status.log")
printf '%s\n' "$console_status" >"$artifact_dir/console-status.json"
console_port=$(jq -er '.http_port | select(type == "number")' <<<"$console_status") \
  || die "console::status did not return a numeric http_port"

curl -fsS --retry 10 --retry-delay 1 \
  "http://127.0.0.1:$console_port/" -o "$artifact_dir/console.html"

[[ -s config.yaml ]] || die "worker add did not write config.yaml"
[[ -s iii.lock ]] || die "worker add did not write iii.lock"
grep -Eiq 'harness' config.yaml || die "config.yaml does not contain harness"
grep -Eiq 'console' config.yaml || die "config.yaml does not contain console"
grep -Eiq 'harness' iii.lock || die "iii.lock does not contain harness"
grep -Eiq 'console' iii.lock || die "iii.lock does not contain console"

echo "Harness quickstart passed with $cli_version"
