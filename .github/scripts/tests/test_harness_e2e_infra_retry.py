from __future__ import annotations

import pytest

from harness_e2e_infra_retry import retry_eligible


@pytest.mark.parametrize("phase", ["bootstrap", "registry"])
def test_retries_only_pre_execution_infrastructure_failures(phase: str) -> None:
    assert retry_eligible(
        deployment={"status": "infra_failed", "failure_phase": phase},
        exit_code=1,
        results_exist=False,
    )


@pytest.mark.parametrize("phase", ["preflight", "e2e", "quality", "hard_gate"])
def test_does_not_retry_scenario_or_gate_failures(phase: str) -> None:
    assert not retry_eligible(
        deployment={"status": "failed", "failure_phase": phase},
        exit_code=1,
        results_exist=False,
    )


@pytest.mark.parametrize("exit_code", [0, 124, 130, 137, 143])
def test_does_not_retry_success_timeout_or_interruption(exit_code: int) -> None:
    assert not retry_eligible(
        deployment={"status": "infra_failed", "failure_phase": "registry"},
        exit_code=exit_code,
        results_exist=False,
    )


def test_does_not_retry_after_results_exist() -> None:
    assert not retry_eligible(
        deployment={"status": "infra_failed", "failure_phase": "registry"},
        exit_code=1,
        results_exist=True,
    )


def test_requires_structured_deployment_evidence() -> None:
    assert not retry_eligible(deployment=None, exit_code=1, results_exist=False)
