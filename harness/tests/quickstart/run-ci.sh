#!/usr/bin/env bash
set -Eeuo pipefail

# Validate the published quickstart in an isolated home and project:
# install iii, boot an empty engine, bring up harness + console through
# `iii compose`, and complete the first Console conversation, switch from
# Anthropic Sonnet 5 to OpenAI Luna, start a second Luna conversation, and
# exercise one real Harness capability.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../../.." && pwd)
artifact_dir=${HARNESS_QUICKSTART_ARTIFACTS_DIR:-"$repo_root/target/harness-quickstart"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
cli_channel=${III_CLI_CHANNEL:-rc}
worker_tag=${III_WORKER_TAG:-latest}
release_worker=${HARNESS_QUICKSTART_RELEASE_WORKER:-}
release_version=${HARNESS_QUICKSTART_RELEASE_VERSION:-}
engine_port=49134
wait_seconds=${HARNESS_QUICKSTART_WAIT_SECONDS:-180}
add_timeout_seconds=${HARNESS_QUICKSTART_ADD_TIMEOUT_SECONDS:-600}
trace_enabled=${HARNESS_QUICKSTART_TRACE:-0}
result_provider_id=openai
result_model_id=gpt-5.6-luna
result_marker=QUICKSTART_OPENAI_NEW_CHAT_OK
capability_function=shell::exec
capability_output_marker=HARNESS_FIRST_CAPABILITY_OUTPUT
playwright_bin=${HARNESS_QUICKSTART_PLAYWRIGHT_BIN:-"$repo_root/console/web/node_modules/.bin/playwright"}

if [[ -n "${III_CHANNEL:-}" ]]; then
  echo "III_CHANNEL was split into III_CLI_CHANNEL and III_WORKER_TAG" >&2
  exit 2
fi

case "$trace_enabled" in
  0 | 1) ;;
  *)
    echo "HARNESS_QUICKSTART_TRACE must be '0' or '1' (got: $trace_enabled)" >&2
    exit 2
    ;;
esac

# The CLI's `next` prerelease channel died with iii/v0.21.8-next.4; the
# prerelease channel is `rc` now, and `iii compose` only exists from
# 0.23.0-rc onward.
case "$cli_channel" in
  latest | rc) ;;
  *)
    echo "III_CLI_CHANNEL must be 'latest' or 'rc' (got: $cli_channel)" >&2
    exit 2
    ;;
esac

case "$worker_tag" in
  latest | next) ;;
  *)
    echo "III_WORKER_TAG must be 'latest' or 'next' (got: $worker_tag)" >&2
    exit 2
    ;;
esac

if [[ -n "$release_worker" || -n "$release_version" ]]; then
  [[ -n "$release_worker" && -n "$release_version" ]] || {
    echo "HARNESS_QUICKSTART_RELEASE_WORKER and HARNESS_QUICKSTART_RELEASE_VERSION must be set together" >&2
    exit 2
  }
fi

[[ -n "${ANTHROPIC_API_KEY:-}" ]] || {
  echo "ANTHROPIC_API_KEY is required" >&2
  exit 2
}
[[ -n "${OPENAI_API_KEY:-}" ]] || {
  echo "OPENAI_API_KEY is required" >&2
  exit 2
}

for command_name in curl jq ffmpeg; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "$command_name is required" >&2
    exit 2
  }
done

[[ -x "$playwright_bin" ]] || {
  echo "Playwright is required at $playwright_bin" >&2
  exit 2
}

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
  "$artifact_dir/model.json" \
  "$artifact_dir/model-anthropic.json" \
  "$artifact_dir/model-openai.json" \
  "$artifact_dir/model-catalog.json" \
  "$artifact_dir/browser-evidence.json" \
  "$artifact_dir/terminal-status.json" \
  "$artifact_dir/first-capability-browser-evidence.json" \
  "$artifact_dir/first-capability-evidence.json" \
  "$artifact_dir/config.yaml" \
  "$artifact_dir/iii.lock" \
  "$artifact_dir/worker-compose.yaml" \
  "$artifact_dir/commands.log" \
  "$log_dir"/*.log
rm -rf "$artifact_dir/playwright-output" "$artifact_dir/slack-evidence"

trace_log="$artifact_dir/commands.log"
if [[ "$trace_enabled" == 1 ]]; then
  : >"$trace_log"
fi

export HOME="$quickstart_home"
export XDG_CONFIG_HOME="$quickstart_home/.config"
export PATH="$quickstart_home/.local/bin:$quickstart_home/.iii/bin:$PATH"
unset ZAI_API_KEY DEEPSEEK_API_KEY OPENROUTER_API_KEY XAI_API_KEY KIMI_API_KEY

engine_pid=""
compose_pid=""
iii_bin=""
failure_reason=""
cli_version="unknown"
compose_file="$project_dir/worker-compose.yaml"
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
  local browser=null terminal=null first_capability_browser=null first_capability_durable=null
  local video=null
  local video_path="$artifact_dir/slack-evidence/quickstart-provider-switch.mp4"
  [[ -f "$artifact_dir/browser-evidence.json" ]] && browser=$(jq -c . "$artifact_dir/browser-evidence.json")
  [[ -f "$artifact_dir/terminal-status.json" ]] && terminal=$(jq -c . "$artifact_dir/terminal-status.json")
  [[ -f "$artifact_dir/first-capability-browser-evidence.json" ]] && first_capability_browser=$(jq -c . "$artifact_dir/first-capability-browser-evidence.json")
  [[ -f "$artifact_dir/first-capability-evidence.json" ]] && first_capability_durable=$(jq -c . "$artifact_dir/first-capability-evidence.json")
  if [[ -f "$video_path" ]]; then
    video=$(jq -n \
      --arg path "slack-evidence/quickstart-provider-switch.mp4" \
      --arg media_type "video/mp4" \
      --arg sha256 "$(sha256sum "$video_path" | awk '{print $1}')" \
      --argjson size_bytes "$(stat -c %s "$video_path")" \
      '{path:$path,media_type:$media_type,sha256:$sha256,size_bytes:$size_bytes}')
  fi
  jq -n \
    --arg status "$status" \
    --arg reason "$failure_reason" \
    --arg cli_version "$cli_version" \
    --arg install_url "$install_url" \
    --arg cli_channel "$cli_channel" \
    --arg worker_tag "$worker_tag" \
    --arg provider "$result_provider_id" \
    --arg model "$result_model_id" \
    --arg marker "$result_marker" \
    --argjson browser "$browser" \
    --argjson terminal "$terminal" \
    --argjson first_capability_browser "$first_capability_browser" \
    --argjson first_capability_durable "$first_capability_durable" \
    --argjson slack_evidence "$video" \
    --argjson elapsed_ms "$(((SECONDS - started_at_seconds) * 1000))" \
    --argjson engine_port "$engine_port" \
    '{
      status: $status,
      failure_reason: $reason,
      cli_version: $cli_version,
      install_url: $install_url,
      cli_channel: $cli_channel,
      worker_tag: $worker_tag,
      provider: $provider,
      model: $model,
      marker: $marker,
      providers: ["anthropic", "openai"],
      models: ["claude-sonnet-5", "gpt-5.6-luna"],
      browser: $browser,
      terminal: $terminal,
      first_capability: {
        browser: $first_capability_browser,
        durable: $first_capability_durable
      },
      slack_evidence: $slack_evidence,
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
  for output in config.yaml worker-compose.yaml; do
    [[ -f "$project_dir/$output" ]] && cp "$project_dir/$output" "$artifact_dir/$output"
  done
}

stop_compose() {
  [[ -n "$compose_pid" ]] || return 0
  # Ask the daemon to stop the project's children first; killing the daemon
  # alone would leave them supervised by nothing until the orphan sweep.
  if [[ -n "$iii_bin" && -n "$engine_pid" ]] \
    && kill -0 "$engine_pid" 2>/dev/null && kill -0 "$compose_pid" 2>/dev/null; then
    "$iii_bin" trigger compose::down --port "$engine_port" \
      --json "$(jq -cn --arg file "$compose_file" '{file: $file}')" \
      >"$log_dir/compose-down.log" 2>&1 || true
  fi
  kill -0 "$compose_pid" 2>/dev/null || return 0
  kill -- "-$compose_pid" 2>/dev/null || kill "$compose_pid" 2>/dev/null || true
  for _ in {1..20}; do
    kill -0 "$compose_pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL -- "-$compose_pid" 2>/dev/null || kill -KILL "$compose_pid" 2>/dev/null || true
  wait "$compose_pid" 2>/dev/null || true
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
  stop_compose
  stop_engine
  stop_owned_orphans

  if artifacts_contain_secret; then
    status=1
    failure_reason="an artifact contains a provider credential"
    # Do not leave a secret-bearing file available to the workflow uploader.
    find "$artifact_dir" -type f -delete
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
    "shell::exec",
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

wait_for_model() {
  local provider_id=$1 model_id=$2 output_path=$3
  local response attempt
  log "Waiting for $provider_id/$model_id in the router catalog (up to ${wait_seconds}s)"
  log_command "iii trigger router::models::get --port $engine_port --json '{\"provider\":\"$provider_id\",\"id\":\"$model_id\"}'"
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger router::models::get --port "$engine_port" \
      --json "{\"provider\":\"$provider_id\",\"id\":\"$model_id\"}" \
      2>>"$log_dir/model-discovery.log" || true)
    if jq -e --arg provider "$provider_id" --arg model "$model_id" \
      '.model.provider == $provider and .model.id == $model' <<<"$response" >/dev/null 2>&1; then
      printf '%s\n' "$response" >"$output_path"
      ok "$provider_id/$model_id available after ${attempt}s"
      return 0
    fi
    if ((attempt > 0 && attempt % 15 == 0)); then
      log "Still waiting for $provider_id/$model_id"
    fi
    sleep 1
  done
  "$iii_bin" trigger router::models::list --port "$engine_port" --json '{}' \
    >"$artifact_dir/model-catalog.json" 2>>"$log_dir/model-discovery.log" || true
  die "$provider_id/$model_id did not appear in the router catalog"
}

record_browser_video() {
  local playwright_status video_source
  local output_dir="$artifact_dir/slack-evidence"
  export HARNESS_QUICKSTART_CONSOLE_URL="http://127.0.0.1:$console_port/"
  export NODE_PATH="$repo_root/console/web/node_modules${NODE_PATH:+:$NODE_PATH}"
  set +e
  "$playwright_bin" test \
    --config "$script_dir/playwright.config.ts" \
    >"$log_dir/playwright.log" 2>&1
  playwright_status=$?
  set -e

  video_source=$(find "$artifact_dir/playwright-output" \
    -type f -path '*/console-first-message-*/video.webm' -print -quit 2>/dev/null || true)
  if [[ -n "$video_source" ]]; then
    mkdir -p "$output_dir"
    ffmpeg -hide_banner -loglevel error -y -i "$video_source" \
      -map_metadata -1 -c:v libx264 -pix_fmt yuv420p -movflags +faststart \
      "$output_dir/quickstart-provider-switch.mp4" \
      >"$log_dir/ffmpeg.log" 2>&1 || die "Playwright video conversion failed"
  elif ((playwright_status == 0)); then
    die "provider-switch Playwright test passed without producing a video"
  fi

  ((playwright_status == 0)) || die "Console quickstart Playwright tests failed"
  ok "Console provider switch and first capability completed with recorded evidence"
}

wait_for_terminal_turns() {
  local session_id response attempt completed statuses='[]'
  local -a session_ids
  mapfile -t session_ids < <(
    jq -er '.sessions | map(.session_id) | unique | .[]' \
      "$artifact_dir/browser-evidence.json"
  ) || die "browser evidence has no session ids"
  ((${#session_ids[@]} == 2)) \
    || die "browser evidence must contain exactly two unique session ids"

  log_command "iii trigger harness::status --port $engine_port --json '{\"session_id\":\"<console-session>\"}'"
  for session_id in "${session_ids[@]}"; do
    log "Verifying the durable terminal turn for session $session_id"
    response=''
    completed=0
    for ((attempt = 0; attempt < wait_seconds; attempt++)); do
      response=$("$iii_bin" trigger harness::status --port "$engine_port" \
        --json "{\"session_id\":\"$session_id\"}" 2>>"$log_dir/terminal-status.log" || true)
      if jq -e '.status == "completed" and (.expects_wake // false) == false' \
        <<<"$response" >/dev/null 2>&1; then
        statuses=$(jq -c \
          --arg session_id "$session_id" \
          --argjson status "$response" \
          '. + [{session_id:$session_id,status:$status}]' <<<"$statuses")
        ok "session $session_id completed durably after ${attempt}s"
        completed=1
        break
      fi
      if jq -e '.status == "failed" or .status == "cancelled"' \
        <<<"$response" >/dev/null 2>&1; then
        die "Console session $session_id reached terminal status $(jq -r '.status' <<<"$response")"
      fi
      sleep 1
    done
    ((completed == 1)) \
      || die "Console session $session_id did not reach durable completion"
  done
  jq -n --argjson sessions "$statuses" \
    '{schema_version:2,sessions:$sessions}' >"$artifact_dir/terminal-status.json"
}

verify_first_capability() {
  local session_id response attempt completed=0 transcript call_count result_count
  session_id=$(jq -er '.session_id' "$artifact_dir/first-capability-browser-evidence.json") \
    || die "first capability browser evidence has no session id"

  log "Verifying the first capability for session $session_id"
  log_command "iii trigger harness::status --port $engine_port --json '{\"session_id\":\"<capability-session>\"}'"
  response=''
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    response=$("$iii_bin" trigger harness::status --port "$engine_port" \
      --json "{\"session_id\":\"$session_id\"}" \
      2>>"$log_dir/first-capability-terminal.log" || true)
    if jq -e '.status == "completed" and (.expects_wake // false) == false' \
      <<<"$response" >/dev/null 2>&1; then
      completed=1
      break
    fi
    if jq -e '.status == "failed" or .status == "cancelled"' \
      <<<"$response" >/dev/null 2>&1; then
      die "first capability session reached terminal status $(jq -r '.status' <<<"$response")"
    fi
    sleep 1
  done
  ((completed == 1)) || die "first capability session did not reach durable completion"

  log_command "iii trigger session::messages --port $engine_port --json '{\"session_id\":\"<capability-session>\",\"limit\":500}'"
  transcript=$("$iii_bin" trigger session::messages --port "$engine_port" \
    --json "{\"session_id\":\"$session_id\",\"limit\":500}" \
    2>"$log_dir/first-capability-transcript.log") \
    || die "could not load the first capability transcript"

  call_count=$(jq -er --arg function "$capability_function" '
    def normalized_function_id:
      if .function_id == "agent_trigger"
      then .arguments.function
      else .function_id
      end;
    [
      .messages[]?.message?
      | select(.role == "assistant")
      | .content[]?
      | select(.type == "function_call")
      | select(normalized_function_id == $function)
    ] | length
  ' <<<"$transcript") || die "could not inspect the first capability function call"
  [[ "$call_count" == 1 ]] \
    || die "expected exactly one durable $capability_function call, found $call_count"

  result_count=$(jq -er \
    --arg function "$capability_function" \
    --arg marker "$capability_output_marker" '
    [
      .messages[]?.message?
      | select(.role == "function_result")
      | select(.function_id == $function)
      | select((.is_error // false) == false)
      | select(.details.exit_code == 0)
      | select(.details.stdout == $marker)
    ] | length
  ' <<<"$transcript") || die "could not inspect the first capability result"
  [[ "$result_count" == 1 ]] \
    || die "$capability_function did not persist the expected successful output"

  jq -n \
    --arg session_id "$session_id" \
    --arg function "$capability_function" \
    --arg output_marker "$capability_output_marker" \
    --arg status "$(jq -r '.status' <<<"$response")" \
    --argjson expects_wake "$(jq -c '.expects_wake // false' <<<"$response")" \
    --argjson call_count "$call_count" \
    --argjson successful_result_count "$result_count" \
    '{
      schema_version: 1,
      session_id: $session_id,
      terminal: {status: $status, expects_wake: $expects_wake},
      function: $function,
      output_marker: $output_marker,
      call_count: $call_count,
      successful_result_count: $successful_result_count
    }' >"$artifact_dir/first-capability-evidence.json"
  ok "$capability_function completed once with the expected durable output"
}

artifacts_contain_secret() {
  local credential
  for credential in "$ANTHROPIC_API_KEY" "$OPENAI_API_KEY"; do
    if LC_ALL=C grep -R -a -F -l -- "$credential" "$artifact_dir" >/dev/null 2>&1; then
      return 0
    fi
  done
  return 1
}

assert_secret_safe_artifacts() {
  if artifacts_contain_secret; then
    die "an artifact contains a provider credential"
  fi
  ok "artifacts do not contain provider credentials"
}

# Declares one worker (and everything the registry resolves for it) in the
# compose file, then lets the daemon restart the project. The call is
# synchronous: it returns after installs finish and every container reports
# ready, so it carries the old `worker add` timeout.
run_compose_add() {
  local spec=$1 payload response
  payload=$(jq -cn --arg file "$compose_file" --arg worker "$spec" \
    '{file: $file, worker: $worker}')
  # `iii trigger` waits only 30s for the invocation result by default, far
  # less than a cold registry install of the full graph on a CI runner —
  # the CLI must wait as long as the add itself is allowed to run; the
  # external `timeout` stays as the hard stop against a hung CLI.
  log_command "iii trigger compose::add --port $engine_port --timeout-ms $((add_timeout_seconds * 1000)) --json '$payload'"
  if command -v timeout >/dev/null 2>&1; then
    response=$(timeout --signal=TERM --kill-after=15s "$((add_timeout_seconds + 30))" \
      "$iii_bin" trigger compose::add --port "$engine_port" \
      --timeout-ms "$((add_timeout_seconds * 1000))" --json "$payload")
  else
    response=$("$iii_bin" trigger compose::add --port "$engine_port" \
      --timeout-ms "$((add_timeout_seconds * 1000))" --json "$payload")
  fi
  printf '%s\n' "$response"
  # A container that failed to start comes back as JSON with status "failed",
  # not as a non-zero exit; the exit code alone proves nothing.
  jq -e '.status == "ok"' <<<"$response" >/dev/null 2>&1 \
    || die "compose::add $spec did not report ok"
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

# The daemon serves `compose::*` in the `default` namespace so the triggers
# below reach it the same way every other function is reached. It runs in the
# foreground by design; the script owns backgrounding it, like the engine.
start_compose() {
  local command=("$iii_bin" compose --engine "ws://127.0.0.1:$engine_port" --namespace default)
  log_command "iii compose --engine ws://127.0.0.1:$engine_port --namespace default"
  if command -v setsid >/dev/null 2>&1; then
    setsid "${command[@]}" >"$log_dir/compose.log" 2>&1 &
  else
    "${command[@]}" >"$log_dir/compose.log" 2>&1 &
  fi
  compose_pid=$!
}

wait_for_compose() {
  local response attempt
  log "Waiting for the compose daemon (up to ${wait_seconds}s)"
  log_command "iii trigger engine::functions::list --port $engine_port --json '{\"include_internal\":true}'"
  for ((attempt = 0; attempt < wait_seconds; attempt++)); do
    kill -0 "$compose_pid" 2>/dev/null || die "compose daemon exited before becoming ready"
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    if jq -e '(.functions // [] | map(.function_id)) | index("compose::add") != null' \
      <<<"$response" >/dev/null 2>&1; then
      ok "compose daemon ready after ${attempt}s"
      return 0
    fi
    sleep 1
  done
  die "compose::add did not register within ${wait_seconds}s"
}

cd "$project_dir"

log "Step 1/9: Install iii from $install_url (channel=$cli_channel)"
log_command "curl -fsSL $install_url -o install.sh"
curl -fsSL --retry 3 --retry-connrefused --retry-delay 5 \
  "$install_url" -o "$run_root/install.sh"
if [[ "$cli_channel" == "rc" ]]; then
  log_command "sh install.sh --rc"
  sh "$run_root/install.sh" --rc 2>&1 | tee "$log_dir/install.log"
else
  log_command "sh install.sh"
  sh "$run_root/install.sh" 2>&1 | tee "$log_dir/install.log"
fi
iii_bin=$(command -v iii || true)
[[ -n "$iii_bin" && -x "$iii_bin" ]] || die "iii CLI was not installed"
log_command "iii --version"
cli_version=$("$iii_bin" --version 2>&1)
"$iii_bin" compose --help >/dev/null 2>&1 \
  || die "the installed iii CLI ($cli_version) has no compose command; the quickstart needs 0.23.0-rc or newer"
printf '%s\n' "$cli_version" >"$artifact_dir/cli-version.txt"
ok "installed $cli_version"

log "Step 2/9: Start an empty engine and the compose daemon"
printf 'workers: []\n' >config.yaml
start_engine
wait_for_engine
start_compose
wait_for_compose
# The seed only names the project: `compose::add` writes every container.
# `namespace: default` is required — harness and Console must register beside
# the engine builtins, exactly like the flow this validates. Written only
# AFTER the daemon is up: since 0.23.0-rc.8 the daemon validates the compose
# file at startup and exits on empty containers (EMPTY_CONTAINERS), while
# `compose::add` requires the file to exist — so the seed lands between the
# daemon boot and the first add.
cat >"$compose_file" <<'COMPOSE'
namespace: default
startup_timeout: 5m
stop_timeout: 10s

containers:
COMPOSE

log "Step 3/9: Add harness and Console from worker tag $worker_tag via compose"
run_compose_add "harness@$worker_tag" 2>&1 | tee "$log_dir/compose-add-harness.log"
run_compose_add "console@$worker_tag" 2>&1 | tee "$log_dir/compose-add-console.log"
ok "compose declared and started harness + console"

log "Step 4/9: Apply exact release candidate override"
if [[ -n "$release_worker" ]]; then
  run_compose_add "${release_worker}@${release_version}" \
    2>&1 | tee "$log_dir/candidate-override.log"
  ok "pinned exact candidate ${release_worker}@${release_version}"
else
  ok "no release candidate override requested"
fi

log "Step 5/9: Verify registered functions"
wait_for_functions

log "Step 6/9: Verify Sonnet 5 and Luna availability"
wait_for_model anthropic claude-sonnet-5 "$artifact_dir/model-anthropic.json"
wait_for_model openai gpt-5.6-luna "$artifact_dir/model-openai.json"

log "Step 7/9: Verify the Console HTTP surface"
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

log "Step 8/9: Validate provider switching and the first real capability"
record_browser_video
wait_for_terminal_turns
verify_first_capability

log "Step 9/9: Verify generated project files and secret-safe evidence"
[[ -s "$project_dir/config.yaml" ]] || die "the engine config.yaml is missing"
[[ -s "$compose_file" ]] || die "compose::add did not write worker-compose.yaml"
for required in harness console; do
  grep -Eiq "$required" "$compose_file" \
    || die "worker-compose.yaml does not contain $required"
done
grep -Eq '^[[:space:]]+version:' "$compose_file" \
  || die "worker-compose.yaml has no pinned versions"
ok "worker-compose.yaml declares harness + console with pinned versions"
snapshot_project
assert_secret_safe_artifacts

log "ALL QUICKSTART ASSERTIONS PASSED ($cli_version, cli=$cli_channel, workers=$worker_tag)"
