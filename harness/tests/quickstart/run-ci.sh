#!/usr/bin/env bash
set -Eeuo pipefail

# Validate the documented install path against the published registry. This
# intentionally does not start a provider or make a model request: the goal is
# to prove that a fresh project can install and boot the harness stack.
#
# Output contract (mirrors iii-hq/quickstart-validator):
#   - every step logs a timestamped narrative line plus [ok] assertions, so
#     the CI job log shows live what happened and in which order;
#   - each stage appends a row to $artifact_dir/timings.tsv and the run ends
#     with an aligned [timing breakdown];
#   - a self-contained $artifact_dir/EVIDENCE.md digest (status, versions,
#     timing, log tails) is always written, even when the run fails.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../../.." && pwd)
artifact_dir=${HARNESS_QUICKSTART_ARTIFACTS_DIR:-"$repo_root/target/harness-quickstart"}
install_url=${III_INSTALL_URL:-https://install.iii.dev/iii/main/install.sh}
channel=${III_CHANNEL:-main}
engine_port=${HARNESS_QUICKSTART_ENGINE_PORT:-49134}
wait_seconds=${HARNESS_QUICKSTART_WAIT_SECONDS:-180}
add_timeout_seconds=${HARNESS_QUICKSTART_ADD_TIMEOUT_SECONDS:-600}
tail_lines=${HARNESS_QUICKSTART_EVIDENCE_TAIL_LINES:-80}
# Live message check through the Console: runs only when ZAI_API_KEY is set.
send_model=${HARNESS_QUICKSTART_MODEL:-glm-5.2}
send_provider=${HARNESS_QUICKSTART_PROVIDER:-zai}
send_prompt=${HARNESS_QUICKSTART_PROMPT:-"Reply with a one-sentence confirmation that you received this message."}
turn_timeout_seconds=${HARNESS_QUICKSTART_TURN_TIMEOUT_SECONDS:-240}

case "$channel" in
  main | next) ;;
  *)
    echo "III_CHANNEL must be 'main' or 'next' (got: $channel)" >&2
    exit 2
    ;;
esac

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
timings_file="$artifact_dir/timings.tsv"
evidence_file="$artifact_dir/EVIDENCE.md"
mkdir -p "$home_dir" "$project_dir" "$log_dir"
# Reset per-run outputs so local reruns don't accumulate stale rows.
: >"$timings_file"
rm -f "$evidence_file"

export HOME="$home_dir"
export XDG_CONFIG_HOME="$home_dir/.config"
export PATH="$HOME/.local/bin:$HOME/.iii/bin:$PATH"

engine_pid=""
failure_reason=""
cli_version="unknown"
message_check="skipped"

log() {
  printf '\n[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

ok() {
  printf '  [ok] %s\n' "$*" >&2
}

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

# --- per-stage timing ------------------------------------------------------
stage_name=""
stage_start_epoch=""

stage_start() {
  stage_name=$1
  stage_start_epoch=$(date +%s)
}

stage_end() {
  [[ -n "$stage_name" ]] || return 0
  local duration=$(($(date +%s) - stage_start_epoch))
  if [[ ! -s "$timings_file" ]]; then
    printf 'stage\tduration_seconds\n' >"$timings_file"
  fi
  printf '%s\t%s\n' "$stage_name" "$duration" >>"$timings_file"
  log "[timing] $stage_name: ${duration}s"
  stage_name=""
  stage_start_epoch=""
}

print_timing_breakdown() {
  [[ -s "$timings_file" ]] || return 0
  local data_lines
  data_lines=$(tail -n +2 "$timings_file" | wc -l | tr -d '[:space:]')
  [[ "${data_lines:-0}" -gt 0 ]] || return 0
  awk -F'\t' '
    NR == 1 { next }
    {
      n++
      stages[n] = $1
      durations[n] = $2 + 0
      total += $2
      if (length($1) > maxname) maxname = length($1)
    }
    END {
      if (maxname < 6) maxname = 6
      width = maxname + 10
      print "[timing breakdown]"
      for (i = 1; i <= n; i++) {
        dots = ""
        for (j = 0; j < width - length(stages[i]) - 1; j++) dots = dots "."
        printf "  %s %s %3ds\n", stages[i], dots, durations[i]
      }
      sep = ""
      for (j = 0; j < width + 6; j++) sep = sep "-"
      printf "  %s\n", sep
      dots = ""
      for (j = 0; j < width - 6; j++) dots = dots "."
      printf "  %s %s %3ds\n", "Total", dots, total
    }
  ' "$timings_file" >&2
}

write_result() {
  local status=$1
  local finished_at_ms elapsed_ms timings_json
  finished_at_ms=$(now_ms)
  elapsed_ms=$((finished_at_ms - started_at_ms))

  if command -v jq >/dev/null 2>&1; then
    timings_json='[]'
    if [[ -s "$timings_file" ]]; then
      timings_json=$(tail -n +2 "$timings_file" | jq -Rn \
        '[inputs | split("\t") | {stage: .[0], duration_seconds: (.[1] | tonumber)}]' \
        2>/dev/null) || timings_json='[]'
    fi
    jq -n \
      --arg status "$status" \
      --arg reason "${failure_reason:-}" \
      --arg cli_version "$cli_version" \
      --arg install_url "$install_url" \
      --arg channel "$channel" \
      --arg message_check "$message_check" \
      --argjson elapsed_ms "$elapsed_ms" \
      --argjson engine_port "$engine_port" \
      --argjson timings "$timings_json" \
      '{status: $status, failure_reason: $reason, cli_version: $cli_version,
        install_url: $install_url, channel: $channel,
        message_check: $message_check, elapsed_ms: $elapsed_ms,
        engine_port: $engine_port, timings: $timings}' \
      >"$artifact_dir/result.json"
  else
    printf '{"status":"%s","failure_reason":"%s"}\n' \
      "$status" "${failure_reason:-}" >"$artifact_dir/result.json"
  fi
}

# Aggregate everything we know into one markdown digest. Best-effort: runs in
# the cleanup trap, so it must never fail the script.
collect_evidence() {
  local status=$1
  local run_url=""
  if [[ -n "${GITHUB_SERVER_URL:-}" && -n "${GITHUB_REPOSITORY:-}" && -n "${GITHUB_RUN_ID:-}" ]]; then
    run_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"
  fi
  {
    printf '# Harness quickstart -- evidence\n\n'
    printf -- '- **Status:** %s\n' "$status"
    printf -- '- **Channel:** %s\n' "$channel"
    printf -- '- **CLI:** %s\n' "$cli_version"
    printf -- '- **Install URL:** %s\n' "$install_url"
    printf -- '- **Message check:** %s\n' "$message_check"
    printf -- '- **Timestamp (UTC):** %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf -- '- **Commit:** %s\n' "${GITHUB_SHA:-local}"
    [[ -n "$run_url" ]] && printf -- '- **CI run:** <%s>\n' "$run_url"
    printf '\n'

    if [[ -n "$failure_reason" ]]; then
      printf '## Failure\n\n```\n%s\n```\n\n' "$failure_reason"
    fi

    if [[ -f "$artifact_dir/console-send.json" ]]; then
      printf '## Console message check (%s via %s)\n\n' "$send_model" "$send_provider"
      printf '**Prompt:** %s\n\n' "$send_prompt"
      printf '**Reply:**\n\n```\n'
      jq -r '.reply' "$artifact_dir/console-send.json" 2>/dev/null
      printf '```\n\n'
    fi

    if [[ -s "$timings_file" ]]; then
      printf '## Timing breakdown\n\n```\n'
      print_timing_breakdown 2>&1
      printf '```\n\n'
    fi

    local f name
    for f in "$log_dir"/*.log; do
      [[ -f "$f" ]] || continue
      name=$(basename "$f")
      # Poll chatter and raw trigger output add noise without evidence value;
      # both files still ship in the logs artifact.
      case "$name" in
        discovery.log | console-status.log) continue ;;
      esac
      printf '## %s (last %s lines)\n\n```\n' "$name" "$tail_lines"
      tail -n "$tail_lines" "$f" 2>/dev/null
      printf '```\n\n'
    done

    if [[ -f "$artifact_dir/console-status.json" ]]; then
      printf '## console::status\n\n```json\n'
      cat "$artifact_dir/console-status.json"
      printf '```\n\n'
    fi

    if [[ -f "$artifact_dir/config.yaml" ]]; then
      printf '## config.yaml\n\n```yaml\n'
      cat "$artifact_dir/config.yaml"
      printf '```\n'
    fi
  } >"$evidence_file" 2>/dev/null
  echo "[collect-evidence] wrote $evidence_file" >&2
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

  # Close a stage interrupted by a failure so its partial duration still lands
  # in the breakdown.
  stage_end

  stop_engine

  if [[ -f "$project_dir/config.yaml" ]]; then
    cp "$project_dir/config.yaml" "$artifact_dir/config.yaml"
  fi
  if [[ -f "$project_dir/iii.lock" ]]; then
    cp "$project_dir/iii.lock" "$artifact_dir/iii.lock"
  fi

  print_timing_breakdown

  if ((status == 0)); then
    write_result passed
    collect_evidence PASS
  else
    write_result failed
    collect_evidence "FAIL (exit $status)"
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
  printf '\n[FAIL] %s\n' "$1" >&2
  return 1
}

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v jq >/dev/null 2>&1 || die "jq is required"

wait_for_engine() {
  local response i
  log "waiting for engine on port $engine_port (up to ${wait_seconds}s)"
  for ((i = 0; i < wait_seconds; i++)); do
    if ! kill -0 "$engine_pid" 2>/dev/null; then
      die "engine exited before becoming ready"
    fi
    response=$("$iii_bin" trigger engine::workers::list --port "$engine_port" \
      --json '{}' 2>>"$log_dir/discovery.log" || true)
    if jq -e '.workers != null' <<<"$response" >/dev/null 2>&1; then
      ok "engine answered engine::workers::list after ${i}s"
      return 0
    fi
    sleep 1
  done
  die "engine did not become ready within ${wait_seconds}s"
}

wait_for_functions() {
  local response missing function_id i
  local required_functions=(
    harness::send
    harness::status
    queue::define
    session::messages
    context::assemble
    router::models::get
    console::status
  )

  log "waiting for ${#required_functions[@]} required functions (up to ${wait_seconds}s)"
  for ((i = 0; i < wait_seconds; i++)); do
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
      ok "all required functions registered after ${i}s"
      return 0
    fi
    if ((i > 0 && i % 15 == 0)); then
      log "still waiting; missing:${missing}"
    fi
    sleep 1
  done
  die "required functions did not register:${missing:- unknown}"
}

# wait_for_function <function_id>: poll engine::functions::list until the
# function registers.
wait_for_function() {
  local function_id=$1 response i
  log "waiting for function $function_id (up to ${wait_seconds}s)"
  for ((i = 0; i < wait_seconds; i++)); do
    response=$("$iii_bin" trigger engine::functions::list --port "$engine_port" \
      --json '{"include_internal":true}' 2>>"$log_dir/discovery.log" || true)
    if jq -e --arg id "$function_id" \
      '(.functions // []) | any(.[]; .function_id == $id)' \
      <<<"$response" >/dev/null 2>&1; then
      ok "function $function_id registered after ${i}s"
      return 0
    fi
    sleep 1
  done
  die "function $function_id did not register within ${wait_seconds}s"
}

# run_add <worker...>: `iii worker add` with a hard timeout.
run_add() {
  if command -v timeout >/dev/null 2>&1; then
    timeout --signal=TERM --kill-after=15s "$add_timeout_seconds" \
      "$iii_bin" worker add "$@"
  else
    "$iii_bin" worker add "$@"
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

# ---------------------------------------------------------------------------
# Step 1: install the CLI from the published installer
# ---------------------------------------------------------------------------
stage_start install_cli
log "Step 1: install the iii CLI from $install_url (channel=$channel)"
curl -fsSL --retry 3 --retry-connrefused --retry-delay 5 \
  "$install_url" -o "$run_root/install.sh"
if [[ "$channel" == "next" ]]; then
  sh "$run_root/install.sh" --next 2>&1 | tee "$log_dir/install.log"
else
  sh "$run_root/install.sh" 2>&1 | tee "$log_dir/install.log"
fi

iii_bin=$(command -v iii || true)
[[ -n "$iii_bin" && -x "$iii_bin" ]] || die "iii CLI was not installed"
cli_version=$("$iii_bin" --version 2>&1)
printf '%s\n' "$cli_version" >"$artifact_dir/cli-version.txt"
ok "installed $cli_version at $iii_bin"
stage_end

# ---------------------------------------------------------------------------
# Step 2: start a clean engine
# ---------------------------------------------------------------------------
stage_start start_engine
log "Step 2: start the engine with an empty config"
printf 'workers: []\n' >config.yaml
start_engine
ok "engine started (pid $engine_pid)"
wait_for_engine
stage_end

# ---------------------------------------------------------------------------
# Step 3: the command this pipeline exists to guarantee
# ---------------------------------------------------------------------------
stage_start worker_add_harness_console
log "Step 3: iii worker add harness console"
run_add harness console 2>&1 | tee "$log_dir/worker-add.log"
ok "worker add exited 0"
stage_end

# ---------------------------------------------------------------------------
# Step 4: wait for the harness/Console function surface
# ---------------------------------------------------------------------------
stage_start wait_for_functions
log "Step 4: wait for the harness/Console function surface"
wait_for_functions
stage_end

# ---------------------------------------------------------------------------
# Step 5: Console answers over HTTP
# ---------------------------------------------------------------------------
stage_start console_check
log "Step 5: query console::status and fetch the Console root"
console_status=$("$iii_bin" trigger console::status --port "$engine_port" \
  --json '{}' 2>"$log_dir/console-status.log")
printf '%s\n' "$console_status" >"$artifact_dir/console-status.json"
console_port=$(jq -er '.http_port | select(type == "number")' <<<"$console_status") \
  || die "console::status did not return a numeric http_port"
ok "console reports http_port=$console_port"

curl -fsS --retry 10 --retry-delay 1 \
  "http://127.0.0.1:$console_port/" -o "$artifact_dir/console.html"
ok "fetched Console root ($(wc -c <"$artifact_dir/console.html" | tr -d ' ') bytes)"
stage_end

# ---------------------------------------------------------------------------
# Step 6: the install left the documented files behind
# ---------------------------------------------------------------------------
stage_start verify_files
log "Step 6: verify config.yaml and iii.lock"
[[ -s config.yaml ]] || die "worker add did not write config.yaml"
[[ -s iii.lock ]] || die "worker add did not write iii.lock"
grep -Eiq 'harness' config.yaml || die "config.yaml does not contain harness"
grep -Eiq 'console' config.yaml || die "config.yaml does not contain console"
grep -Eiq 'harness' iii.lock || die "iii.lock does not contain harness"
grep -Eiq 'console' iii.lock || die "iii.lock does not contain console"
ok "config.yaml and iii.lock reference harness + console"
stage_end

# ---------------------------------------------------------------------------
# Steps 7-8: add the Z.AI provider and prove a real model reply through the
# Console. Gated on ZAI_API_KEY: the engine (started in Step 2) inherited
# this shell's environment, so the provider worker it spawns sees the key.
# ---------------------------------------------------------------------------
if [[ -n "${ZAI_API_KEY:-}" ]]; then
  stage_start worker_add_provider_zai
  log "Step 7: iii worker add provider-zai"
  run_add provider-zai 2>&1 | tee "$log_dir/provider-add.log"
  wait_for_function provider::zai::stream
  stage_end

  stage_start console_message_check
  log "Step 8: send a message through the Console /ws proxy (model=$send_model provider=$send_provider)"
  message_check="failed"
  python3 -m venv "$run_root/venv" >>"$log_dir/console-send.log" 2>&1 \
    || die "python3 -m venv failed (see console-send.log)"
  "$run_root/venv/bin/pip" install --quiet websockets >>"$log_dir/console-send.log" 2>&1 \
    || die "pip install websockets failed (see console-send.log)"
  if ! "$run_root/venv/bin/python" "$script_dir/console_send.py" \
    --url "ws://127.0.0.1:$console_port/ws" \
    --model "$send_model" --provider "$send_provider" \
    --prompt "$send_prompt" --timeout "$turn_timeout_seconds" \
    >"$artifact_dir/console-send.json" \
    2> >(tee -a "$log_dir/console-send.log" >&2); then
    die "console message send failed (see console-send.log)"
  fi
  reply=$(jq -er '.reply | select(length > 0)' "$artifact_dir/console-send.json") \
    || die "console send did not produce a non-empty assistant reply"
  ok "assistant replied through the Console (${#reply} chars)"
  message_check="passed"
  stage_end
else
  log "Steps 7-8: ZAI_API_KEY is not set; skipping provider add + Console message check"
fi

log "ALL QUICKSTART ASSERTIONS PASSED ($cli_version, channel=$channel, message_check=$message_check)"
