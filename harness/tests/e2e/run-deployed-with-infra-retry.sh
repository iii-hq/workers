#!/usr/bin/env bash
set -Eeuo pipefail

# Retry only cleanly classified provisioning failures. The scenario runner and
# quality gates are never repeated here.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
harness_root=$(cd -- "$script_dir/../.." && pwd)
repo_root=$(cd -- "$harness_root/.." && pwd)
base_artifact_dir=${HARNESS_E2E_ARTIFACTS_DIR:-"$repo_root/target/harness-e2e"}
max_attempts=${HARNESS_E2E_PROVISIONING_ATTEMPTS:-1}
launcher=${HARNESS_E2E_DEPLOYED_LAUNCHER:-"$script_dir/run-deployed-ci.sh"}

[[ "$max_attempts" =~ ^[12]$ ]] || {
  echo "HARNESS_E2E_PROVISIONING_ATTEMPTS must be 1 or 2" >&2
  exit 2
}

mkdir -p "$base_artifact_dir/attempts"
base_artifact_dir=$(cd "$base_artifact_dir" && pwd)
history_file="$base_artifact_dir/attempts/history.jsonl"
: >"$history_file"

final_attempt=1
final_status=1
for attempt in $(seq 1 "$max_attempts"); do
  attempt_dir="$base_artifact_dir/attempts/attempt-$attempt"
  mkdir -p "$attempt_dir"

  set +e
  HARNESS_E2E_ARTIFACTS_DIR="$attempt_dir" \
    "$launcher"
  status=$?
  set -e

  deployment="$attempt_dir/deployment.json"
  if [[ -f "$deployment" ]]; then
    jq -c --argjson attempt "$attempt" '{
      attempt: $attempt,
      status,
      failure_phase,
      failure_reason,
      elapsed_ms
    }' "$deployment" >>"$history_file"
  else
    jq -cn \
      --argjson attempt "$attempt" \
      --argjson exit_code "$status" \
      '{
        attempt: $attempt,
        status: "missing_deployment",
        failure_phase: "unknown",
        failure_reason: ("launcher exited with " + ($exit_code | tostring)),
        elapsed_ms: null
      }' >>"$history_file"
  fi

  final_attempt=$attempt
  final_status=$status
  if ((status == 0 || attempt == max_attempts)); then
    break
  fi
  if ! python3 "$repo_root/.github/scripts/harness_e2e_infra_retry.py" \
    --deployment "$deployment" \
    --results "$attempt_dir/results/results.json" \
    --exit-code "$status"; then
    break
  fi
  printf '\nRetrying clean provisioning after attempt %s\n' "$attempt" >&2
done

final_dir="$base_artifact_dir/attempts/attempt-$final_attempt"
for name in logs stack results; do
  [[ -d "$final_dir/$name" ]] && cp -a "$final_dir/$name" "$base_artifact_dir/$name"
done
[[ -f "$final_dir/cli-version.txt" ]] && \
  cp "$final_dir/cli-version.txt" "$base_artifact_dir/cli-version.txt"

if [[ -f "$final_dir/deployment.json" ]]; then
  history=$(jq -s . "$history_file")
  jq \
    --argjson attempts "$final_attempt" \
    --argjson history "$history" '
      . + {
        provisioning_attempts: $attempts,
        provisioning_attempt_history: $history
      }
    ' "$final_dir/deployment.json" >"$base_artifact_dir/deployment.json"
  if [[ -d "$base_artifact_dir/results" ]]; then
    cp "$base_artifact_dir/deployment.json" \
      "$base_artifact_dir/results/deployment.json"
  fi
fi

exit "$final_status"
