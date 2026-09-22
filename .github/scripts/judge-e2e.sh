#!/usr/bin/env bash
# judge hub + judge-typesafe + judge-semif over an isolated real engine (TypeSafe mocks, tiny GGUF).
# Build the workers' UIs first (or let their build scripts do so). No inference key is needed.
set -euo pipefail

unset TYPESAFE_API_KEY
: "${III_ENGINE_BIN:?Set III_ENGINE_BIN to an absolute path to the iii engine}"
if [[ "$III_ENGINE_BIN" != /* || ! -x "$III_ENGINE_BIN" ]]; then
  echo 'III_ENGINE_BIN must be an absolute path to an executable engine' >&2
  exit 1
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
report_dir="${JUDGE_E2E_REPORT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/judge-e2e-reports.XXXXXX")}"
mkdir -p "$report_dir"
export JUDGE_E2E_REPORT_DIR="$(cd -- "$report_dir" && pwd)"
printf 'judge E2E logs: %s\n' "$JUDGE_E2E_REPORT_DIR"

run_suite() {
  local worker="$1" suite="$2"
  shift 2
  if (( $# == 0 )); then
    echo "Refusing empty $worker $suite test selection" >&2
    return 1
  fi
  cargo test --manifest-path "$repo_root/$worker/Cargo.toml" --locked --release \
    --test "$suite" -- --ignored --exact --test-threads=1 --nocapture "$@" \
    2>&1 | tee "$JUDGE_E2E_REPORT_DIR/$worker.$suite.log"
  # libtest succeeds when filters match zero tests. Require every selected case
  # to pass so renames/removals fail this gate. The fixture preserves engine logs
  # separately and kills/reaps each child before deleting its scratch directory.
  if ! grep -Fq "test result: ok. $# passed; 0 failed; 0 ignored;" "$JUDGE_E2E_REPORT_DIR/$worker.$suite.log"; then
    echo "Expected all $# selected $suite cases to pass; check $JUDGE_E2E_REPORT_DIR/$worker.$suite.log" >&2
    return 1
  fi
}

# Never run all ignored tests: future live-provider tests must stay unselected.
run_suite judge-typesafe engine \
  independent_consumer_evaluates_mixed_primitives_and_lists_models \
  cancellation_is_scoped_to_the_persistent_engine_caller
run_suite judge-semif engine \
  tiny_gguf_answers_through_a_real_engine
run_suite judge-laya engine \
  tiny_checkpoint_answers_through_a_real_engine
run_suite judge engine \
  hub_forwards_to_the_provider_and_scopes_cancellation_to_the_original_caller
