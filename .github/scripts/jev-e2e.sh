#!/usr/bin/env bash
# Standalone JEV over an isolated real engine and loopback provider mocks.
# Build jev/ui first (or let its build script do so). No inference key is needed.
set -euo pipefail

unset TYPESAFE_API_KEY
: "${III_ENGINE_BIN:?Set III_ENGINE_BIN to an absolute path to the iii engine}"
if [[ "$III_ENGINE_BIN" != /* || ! -x "$III_ENGINE_BIN" ]]; then
  echo 'III_ENGINE_BIN must be an absolute path to an executable engine' >&2
  exit 1
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
report_dir="${JEV_E2E_REPORT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/jev-e2e-reports.XXXXXX")}"
mkdir -p "$report_dir"
export JEV_E2E_REPORT_DIR="$(cd -- "$report_dir" && pwd)"
printf 'JEV E2E logs: %s\n' "$JEV_E2E_REPORT_DIR"

run_suite() {
  local suite="$1"
  shift
  if (( $# == 0 )); then
    echo "Refusing empty $suite test selection" >&2
    return 1
  fi
  cargo test --manifest-path "$repo_root/jev/Cargo.toml" --locked --release \
    --test "$suite" -- --ignored --exact --test-threads=1 --nocapture "$@" \
    2>&1 | tee "$JEV_E2E_REPORT_DIR/$suite.log"
  # libtest succeeds when filters match zero tests. Require every selected case
  # to pass so renames/removals fail this gate. The fixture preserves engine logs
  # separately and kills/reaps each child before deleting its scratch directory.
  if ! grep -Fq "test result: ok. $# passed; 0 failed; 0 ignored;" "$JEV_E2E_REPORT_DIR/$suite.log"; then
    echo "Expected all $# selected $suite cases to pass; check $JEV_E2E_REPORT_DIR/$suite.log" >&2
    return 1
  fi
}

# Never run all ignored tests: future live-provider tests must stay unselected.
run_suite engine \
  independent_consumer_evaluates_mixed_primitives_and_lists_models \
  cancellation_is_scoped_to_the_persistent_engine_caller
