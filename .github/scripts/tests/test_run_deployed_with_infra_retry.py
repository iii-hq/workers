from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


REPOSITORY = Path(__file__).parents[3]
WRAPPER = REPOSITORY / "harness/tests/e2e/run-deployed-with-infra-retry.sh"


def write_launcher(path: Path, first_phase: str) -> None:
    path.write_text(
        f"""#!/usr/bin/env bash
set -euo pipefail
mkdir -p "$HARNESS_E2E_ARTIFACTS_DIR/results"
if [[ "$HARNESS_E2E_ARTIFACTS_DIR" == *attempt-1 ]]; then
  jq -n '{{
    status: "infra_failed",
    failure_phase: "{first_phase}",
    failure_reason: "first failed",
    elapsed_ms: 10
  }}' >"$HARNESS_E2E_ARTIFACTS_DIR/deployment.json"
  exit 1
fi
jq -n '{{
  status: "passed",
  failure_phase: "e2e",
  failure_reason: "",
  elapsed_ms: 20
}}' >"$HARNESS_E2E_ARTIFACTS_DIR/deployment.json"
printf '{{"scenarios":[]}}\n' >"$HARNESS_E2E_ARTIFACTS_DIR/results/results.json"
"""
    )
    path.chmod(0o755)


def run_wrapper(tmp_path: Path, phase: str) -> subprocess.CompletedProcess[str]:
    launcher = tmp_path / "launcher.sh"
    write_launcher(launcher, phase)
    env = {
        **os.environ,
        "HARNESS_E2E_ARTIFACTS_DIR": str(tmp_path / "artifacts"),
        "HARNESS_E2E_PROVISIONING_ATTEMPTS": "2",
        "HARNESS_E2E_DEPLOYED_LAUNCHER": str(launcher),
    }
    return subprocess.run(
        ["bash", str(WRAPPER)],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )


def test_retries_registry_failure_and_preserves_both_attempts(tmp_path: Path) -> None:
    result = run_wrapper(tmp_path, "registry")

    assert result.returncode == 0, result.stderr
    artifacts = tmp_path / "artifacts"
    deployment = json.loads((artifacts / "deployment.json").read_text())
    assert deployment["provisioning_attempts"] == 2
    assert [
        item["status"] for item in deployment["provisioning_attempt_history"]
    ] == ["infra_failed", "passed"]
    assert (artifacts / "attempts/attempt-1/deployment.json").is_file()
    assert (artifacts / "attempts/attempt-2/deployment.json").is_file()
    assert (artifacts / "results/results.json").is_file()


def test_does_not_retry_e2e_failure(tmp_path: Path) -> None:
    result = run_wrapper(tmp_path, "e2e")

    assert result.returncode == 1
    deployment = json.loads(
        (tmp_path / "artifacts/deployment.json").read_text()
    )
    assert deployment["provisioning_attempts"] == 1
    assert not (tmp_path / "artifacts/attempts/attempt-2").exists()
