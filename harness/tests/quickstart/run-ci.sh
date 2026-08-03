#!/usr/bin/env bash
set -Eeuo pipefail

# Validate the published quickstart in an isolated home and project:
# install iii, boot an empty engine, add harness + console, and probe the
# resulting function and HTTP surfaces.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../../.." && pwd)
artifact_dir=${HARNESS_QUICKSTART_ARTIFACTS_DIR:-"$repo_root/target/harness-quickstart"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
channel=${III_CHANNEL:-latest}
engine_port=49134
wait_seconds=${HARNESS_QUICKSTART_WAIT_SECONDS:-180}
add_timeout_seconds=${HARNESS_QUICKSTART_ADD_TIMEOUT_SECONDS:-600}
trace_enabled=${HARNESS_QUICKSTART_TRACE:-0}

case "$trace_enabled" in
  0 | 1) ;;
  *)
    echo "HARNESS_QUICKSTART_TRACE must be '0' or '1' (got: $trace_enabled)" >&2
    exit 2
    ;;
esac

case "$channel" in
  latest | next) ;;
  *)
    echo "III_CHANNEL must be 'latest' or 'next' (got: $channel)" >&2
    exit 2
    ;;
esac

for command_name in curl jq; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "$command_name is required" >&2
    exit 2
  }
done

[[ "$wait_seconds" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_QUICKSTART_WAIT_SECONDS must be a positive integer" >&2
  exit 2
}
wait_seconds=$((10#$wait_seconds))
((wait_seconds > 0)) || {
  echo "HARNESS_QUICKSTART_WAIT_SECONDS must be a positive integer" >&2
  exit 2
}
[[ "$add_timeout_seconds" =~ ^[0-9]+$ ]] || {
  echo "HARNESS_QUICKSTART_ADD_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 2
}
add_timeout_seconds=$((10#$add_timeout_seconds))
((add_timeout_seconds > 0)) || {
  echo "HARNESS_QUICKSTART_ADD_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 2
}
mkdir -p "$artifact_dir"
artifact_dir=$(cd "$artifact_dir" && pwd)

run_root=$(mktemp -d "${TMPDIR:-/tmp}/harness-quickstart.XXXXXX")
project_dir="$run_root/project"
quickstart_home="$run_root/home"
log_dir="$artifact_dir/logs"
mkdir -p "$project_dir" "$quickstart_home" "$log_dir"
rm -f \
  "$artifact_dir/result.json" \
  "$artifact_dir/cli-version.txt" \
  "$artifact_dir/console-status.json" \
  "$artifact_dir/console.html" \
  "$artifact_dir/config.yaml" \
  "$artifact_dir/iii.lock" \
  "$artifact_dir/commands.log" \
  "$log_dir"/*.log

trace_log="$artifact_dir/commands.log"
if [[ "$trace_enabled" == 1 ]]; then
  : >"$trace_log"
fi

export HOME="$quickstart_home"
export XDG_CONFIG_HOME="$quickstart_home/.config"
export PATH="$quickstart_home/.local/bin:$quickstart_home/.iii/bin:$PATH"
unset ANTHROPIC_API_KEY OPENAI_API_KEY ZAI_API_KEY

engine_pid=""
iii_bin=""
failure_reason=""
cli_version="unknown"
started_at_seconds=$SECONDS

log() {
  printf '\n[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

log_command() {
  [[ "$trace_enabled" == 1 ]] || return 0

  local rendered="$*"
  printf '\n  $ %s\n' "$rendered" >&2
  printf '$ %s\n' "$rendered" >>"$trace_log"
}

ok() {
  printf '  [ok] %s\n' "$*" >&2
}

die() {
  failure_reason=$1
  printf '\n[FAIL] %s\n' "$failure_reason" >&2
  return 1
}

write_result() {
  local status=$1
  jq -n \
    --arg status "$status" \
    --arg reason "$failure_reason" \
    --arg cli_version "$cli_version" \
    --arg install_url "$install_url" \
    --arg channel "$channel" \
    --argjson elapsed_ms "$(((SECONDS - started_at_seconds) * 1000))" \
    --argjson engine_port "$engine_port" \
    '{
      status: $status,
      failure_reason: $reason,
      cli_version: $cli_version,
      install_url: $install_url,
      channel: $channel,
      elapsed_ms: $elapsed_ms,
      engine_port: $engine_port
    }' >"$artifact_dir/result.json"
}

stop_engine() {
  [[ -n "$engine_pid" ]] && kill -0 "$engine_pid" 2>/dev/null || return 0

  # The engine gets its own process group so registry workers cannot outlive
  # the validator.
  kill -- "-$engine_pid" 2>/dev/null || kill "$engine_pid" 2>/dev/null || true
  for _ in {1..20}; do
    kill -0 "$engine_pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL -- "-$engine_pid" 2>/dev/null || kill -KILL "$engine_pid" 2>/dev/null || true
  wait "$engine_pid" 2>/dev/null || true
}

snapshot_project() {
  for output in config.yaml iii.lock; do
    [[ -f "$project_dir/$output" ]] && cp "$project_dir/$output" "$artifact_dir/$output"
  done
}

stop_managed_workers() {
  [[ -n "$iii_bin" && -n "$engine_pid" ]] || return 0
  kill -0 "$engine_pid" 2>/dev/null || return 0
  (cd "$project_dir" && "$iii_bin" worker remove -y harness console) \
    >"$log_dir/worker-remove.log" 2>&1 || true
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

cleanup() {
  local status=$?
  trap - EXIT INT TERM ERR
  set +e

  snapshot_project
  stop_managed_workers
  stop_engine
  stop_owned_orphans

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
  [[ -n "$failure_reason" ]] || failure_reason="command failed at line $line (exit $status)"
  return "$status"
}

trap 'on_error "$LINENO"' ERR
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

wait_for_engine() {
  local response attempt
  log "Waiting for engine on port $engine_port (up to ${wait_seconds}s)"
  log_command "iii trigger engine::workers::list --port $engine_port --json '{}'"
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

wait_for_functions() {
  local response missing attempt
  local required='[
    "harness::send",
    "harness::status",
    "queue::define",
    "session::messages",
    "context::assemble",
    "router::models::get",
    "console::status"
  ]'

  log "Waiting for the harness and Console function surface (up to ${wait_seconds}s)"
  log_command "iii trigger engine::functions::list --port $engine_port --json '{\"include_internal\":true}'"
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    missing=$(jq -r --argjson required "$required" '
      (.functions // [] | map(.function_id)) as $available
      | ($required - $available)
      | join(" ")
    ' <<<"$response" 2>/dev/null || printf 'function discovery failed')
    if [[ -z "$missing" ]]; then
      ok "all required functions registered after ${attempt}s"
      return 0
    fi
    if ((attempt > 0 && attempt % 15 == 0)); then
      log "Still waiting for: $missing"
    fi
    sleep 1
  done
  die "required functions did not register: $missing"
}

run_worker_add() {
  log_command "iii worker add $*"
  if command -v timeout >/dev/null 2>&1; then
    timeout --signal=TERM --kill-after=15s "$add_timeout_seconds" \
      "$iii_bin" worker add "$@"
  else
    "$iii_bin" worker add "$@"
  fi
}

start_engine() {
  local command=("$iii_bin" -c "$project_dir/config.yaml" --no-update-check)
  log_command "iii -c config.yaml --no-update-check"
  if command -v setsid >/dev/null 2>&1; then
    setsid "${command[@]}" >"$log_dir/engine.log" 2>&1 &
  else
    "${command[@]}" >"$log_dir/engine.log" 2>&1 &
  fi
  engine_pid=$!
}

cd "$project_dir"

log "Step 1/6: Install iii from $install_url (channel=$channel)"
log_command "curl -fsSL $install_url -o install.sh"
curl -fsSL --retry 3 --retry-connrefused --retry-delay 5 \
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

log "Step 2/6: Start an empty engine"
printf 'workers: []\n' >config.yaml
start_engine
wait_for_engine

log "Step 3/6: Add harness and Console"
run_worker_add harness console 2>&1 | tee "$log_dir/worker-add.log"
ok "iii worker add harness console exited successfully"

log "Step 4/6: Verify registered functions"
wait_for_functions

log "Step 5/6: Verify the Console HTTP surface"
log_command "iii trigger console::status --port $engine_port --json '{}'"
console_status=$("$iii_bin" trigger console::status --port "$engine_port" \
  --json '{}' 2>"$log_dir/console-status.log")
printf '%s\n' "$console_status" >"$artifact_dir/console-status.json"
console_port=$(jq -er '.http_port | select(type == "number")' <<<"$console_status") \
  || die "console::status did not return a numeric http_port"
log_command "curl -fsS http://127.0.0.1:$console_port/"
curl -fsS --retry 10 --retry-all-errors --retry-delay 1 \
  "http://127.0.0.1:$console_port/" -o "$artifact_dir/console.html"
ok "Console answered on port $console_port"

log "Step 6/6: Verify generated project files"
for output in config.yaml iii.lock; do
  [[ -s "$output" ]] || die "worker add did not write $output"
  grep -Eiq 'harness' "$output" || die "$output does not contain harness"
  grep -Eiq 'console' "$output" || die "$output does not contain console"
done
ok "config.yaml and iii.lock reference harness + console"

log "ALL QUICKSTART ASSERTIONS PASSED ($cli_version, channel=$channel)"
