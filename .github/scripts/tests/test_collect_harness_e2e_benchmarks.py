"""Tests for Harness E2E benchmark collection."""

from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path

import pytest

from collect_harness_e2e_benchmarks import (
    CollectionConfig,
    CollectionError,
    collect,
    semantic_result_status,
)


def test_semantic_result_status_uses_blocking_precedence() -> None:
    assert (
        semantic_result_status(
            passed=False,
            hard_gate_failures=1,
            technical_failures=1,
        )
        == "technical_failed"
    )
    assert (
        semantic_result_status(
            passed=False,
            hard_gate_failures=1,
            technical_failures=0,
        )
        == "hard_gate_failed"
    )
    assert (
        semantic_result_status(
            passed=False,
            hard_gate_failures=0,
            technical_failures=0,
        )
        == "infra_failed"
    )
    assert (
        semantic_result_status(
            passed=True,
            hard_gate_failures=0,
            technical_failures=0,
            complete=False,
        )
        == "incomplete"
    )
    assert (
        semantic_result_status(
            passed=True,
            hard_gate_failures=0,
            technical_failures=0,
        )
        == "passed"
    )
    assert (
        semantic_result_status(
            passed=True,
            hard_gate_failures=0,
            technical_failures=0,
            infra_failures=1,
        )
        == "infra_failed"
    )


@pytest.mark.parametrize(
    ("passed", "hard_gates", "technical", "expected"),
    (
        (False, 0, 0, "infra_failed"),
        (False, 1, 0, "hard_gate_failed"),
        (False, 0, 1, "technical_failed"),
    ),
)
def test_collects_semantic_scenario_status(
    tmp_path: Path,
    passed: bool,
    hard_gates: int,
    technical: int,
    expected: str,
) -> None:
    write_report(
        tmp_path,
        subject_id="glm",
        scenario_id="direct_answer",
        passed=passed,
        hard_gate_failures=hard_gates,
        technical_failures=technical,
    )

    quality, _, snapshot, _ = collect(config(tmp_path, ["direct_answer"]))

    assert snapshot["subjects"][0]["scenarios"][0]["status"] == expected
    scenario_metric = next(
        metric
        for metric in quality
        if metric["name"] == "quality::glm::direct_answer::median_score"
    )
    assert json.loads(scenario_metric["extra"])["status"] == expected


def write_report(
    root: Path,
    *,
    subject_id: str,
    scenario_id: str,
    passed: bool = True,
    median_score: float | None = 90,
    pass_rate: float = 1,
    hard_gate_failures: int = 0,
    technical_failures: int = 0,
    total_cost: float | None = 0.3,
    wall_time_ms: int = 20_000,
    retries: int = 0,
) -> None:
    directory = root / f"harness-e2e-{subject_id}-{scenario_id}-results"
    directory.mkdir(parents=True)
    (directory / "benchmark-context.json").write_text(
        json.dumps({"subject_id": subject_id, "scenario_id": scenario_id})
    )
    retry_attempts = [
        {
            "run_id": f"retry-{index}",
            "session_id": f"session-{index}",
            "wall_time_ms": 10,
            "status": "subject_error",
            "cost": {
                "subject_usd": 0.01,
                "judge_usd": 0,
                "total_usd": 0.01,
            },
            "failures": [],
        }
        for index in range(retries)
    ]
    (directory / "results.json").write_text(
        json.dumps(
            {
                "subject": {"model": "glm-5.2", "provider": "zai"},
                "judge": {"model": "glm-5.2", "provider": "zai"},
                "engine_revision": "a" * 40,
                "passed": passed,
                "scenarios": [
                    {
                        "scenario_id": scenario_id,
                        "aggregate": {
                            "runs": 1,
                            "scored_runs": int(median_score is not None),
                            "passed_runs": int(passed),
                            "required_passes": 1,
                            "pass_rate": pass_rate,
                            "median_score": median_score,
                            "hard_gate_failures": hard_gate_failures,
                            "technical_failures": technical_failures,
                            "cost": {
                                "subject_usd": (
                                    total_cost * 0.8
                                    if total_cost is not None
                                    else None
                                ),
                                "judge_usd": (
                                    total_cost * 0.2
                                    if total_cost is not None
                                    else None
                                ),
                                "total_usd": total_cost,
                            },
                        },
                        "passed": passed,
                        "runs": [
                            {
                                "run_id": "run-1",
                                "session_id": "session-1",
                                "prompt": "Complete the E2E task.",
                                "wall_time_ms": wall_time_ms,
                                "score": median_score,
                                "status": "passed" if passed else "hard_gate_failed",
                                "hard_gates": [
                                    {
                                        "id": "state_present",
                                        "passed": passed,
                                        "reason": "fixture evidence",
                                    }
                                ],
                                "criteria": [
                                    {
                                        "id": "completion",
                                        "possible": 100,
                                        "awarded": median_score,
                                        "reason": "fixture score",
                                    }
                                ],
                                "transcript": {
                                    "messages": [
                                        {
                                            "entry_id": "message-1",
                                            "message": {
                                                "role": "assistant",
                                                "content": "Done.",
                                            },
                                        }
                                    ]
                                },
                                "metrics": {
                                    "complete": True,
                                    "root_session_id": "session-1",
                                    "totals": {"input_tokens": 100},
                                    "by_session": [],
                                    "traces": {"trace_count": 1},
                                },
                                "cost": {
                                    "subject_usd": total_cost,
                                    "judge_usd": 0,
                                    "total_usd": total_cost,
                                },
                                "retry_attempts": retry_attempts,
                                "failures": [],
                            }
                        ],
                    }
                ],
            }
        )
    )


def config(root: Path, scenarios: list[str]) -> CollectionConfig:
    return CollectionConfig(
        reports_root=root,
        output_dir=root / "output",
        subjects=[{"id": "glm", "model": "glm-5.2", "provider": "zai"}],
        scenarios=scenarios,
        lane="release",
        requested_runs=3,
        source_sha="b" * 40,
        source_ref="provider-zai/v0.5.1",
        repository="iii-hq/workers",
        workflow_url="https://github.com/iii-hq/workers/actions/runs/1",
        release_tag="provider-zai/v0.5.1",
        release_worker="provider-zai",
        release_version="0.5.1",
        release_url="https://github.com/iii-hq/workers/releases/tag/provider-zai/v0.5.1",
        registry_tag="latest",
        judge_model="glm-5.2",
        judge_provider="zai",
        execution_run_id="12345",
        execution_attempt=2,
        execution_event="workflow_dispatch",
        execution_actor="octocat",
        generated_at="2026-07-29T12:00:00+00:00",
    )


def by_name(metrics: list[dict]) -> dict[str, dict]:
    return {entry["name"]: entry for entry in metrics}


def test_collects_quality_reliability_and_efficiency(tmp_path: Path) -> None:
    write_report(
        tmp_path,
        subject_id="glm",
        scenario_id="direct_answer",
        median_score=92,
        total_cost=0.25,
        wall_time_ms=12_000,
        retries=1,
    )
    write_report(
        tmp_path,
        subject_id="glm",
        scenario_id="security_review",
        median_score=88,
        total_cost=0.75,
        wall_time_ms=28_000,
    )

    quality, efficiency, snapshot, execution = collect(
        config(tmp_path, ["direct_answer", "security_review"])
    )
    quality_by_name = by_name(quality)
    efficiency_by_name = by_name(efficiency)

    assert (
        quality_by_name["quality::glm::direct_answer::median_score"]["value"]
        == 92
    )
    assert quality_by_name["quality::glm::suite::scenario_pass_rate"]["value"] == 100
    assert quality_by_name["quality::glm::suite::report_coverage"]["value"] == 100
    assert (
        efficiency_by_name["efficiency::glm::suite::total_cost_usd"]["value"]
        == 1
    )
    assert (
        efficiency_by_name["efficiency::glm::suite::wall_time_seconds"]["value"]
        == 40
    )
    assert (
        efficiency_by_name["reliability::glm::suite::retry_attempts"]["value"]
        == 1
    )
    subject = snapshot["subjects"][0]
    assert subject["passed"] is True
    assert subject["total_cost_usd"] == 1
    assert subject["engine_revision"] == "a" * 40
    assert snapshot["execution"]["id"] == "12345-2"
    assert execution["reports"][0]["report"]["scenarios"][0]["runs"][0]["prompt"] == (
        "Complete the E2E task."
    )
    assert execution["reports"][0]["report"]["scenarios"][0]["runs"][0][
        "transcript"
    ]["messages"][0]["message"]["content"] == "Done."


def test_missing_report_is_visible_and_totals_are_not_fabricated(
    tmp_path: Path,
) -> None:
    write_report(tmp_path, subject_id="glm", scenario_id="direct_answer")

    quality, efficiency, snapshot, execution = collect(
        config(tmp_path, ["direct_answer", "security_review"])
    )
    quality_by_name = by_name(quality)
    efficiency_by_name = by_name(efficiency)

    assert quality_by_name["quality::glm::suite::report_coverage"]["value"] == 50
    assert quality_by_name["quality::glm::suite::scenario_pass_rate"]["value"] == 50
    assert (
        efficiency_by_name["reliability::glm::suite::missing_reports"]["value"]
        == 1
    )
    assert "efficiency::glm::suite::total_cost_usd" not in efficiency_by_name
    assert "efficiency::glm::suite::wall_time_seconds" not in efficiency_by_name
    subject = snapshot["subjects"][0]
    assert subject["passed"] is False
    assert subject["total_cost_usd"] is None
    assert subject["scenarios"][1]["status"] == "missing_report"
    assert execution["reports"][1] == {
        "subject_id": "glm",
        "scenario_id": "security_review",
        "available": False,
        "report": None,
    }


def test_registry_identity_is_recorded_and_registry_failure_is_infra_failed(
    tmp_path: Path,
) -> None:
    write_report(tmp_path, subject_id="glm", scenario_id="direct_answer")
    directory = tmp_path / "harness-e2e-glm-security_review-results"
    directory.mkdir()
    (directory / "benchmark-context.json").write_text(
        json.dumps({"subject_id": "glm", "scenario_id": "security_review"})
    )
    (directory / "deployment.json").write_text(
        json.dumps(
            {
                "status": "infra_failed",
                "actual_release_version": "1.8.0",
                "stack_versions": {"harness": "1.8.0", "state": "0.22.0"},
                "stack_lock_digest": "a" * 64,
            }
        )
    )

    _, efficiency, snapshot, execution = collect(
        replace(
            config(tmp_path, ["direct_answer", "security_review"]),
            stack_mode="registry",
        )
    )
    efficiency_by_name = by_name(efficiency)
    security_extra = json.loads(
        efficiency_by_name[
            "reliability::glm::security_review::infra_failed"
        ]["extra"]
    )

    assert security_extra["status"] == "infra_failed"
    assert security_extra["release"]["version"] == "1.8.0"
    assert security_extra["stack"]["versions"] == {
        "harness": "1.8.0",
        "state": "0.22.0",
    }
    assert security_extra["stack"]["lock_digest"] == "a" * 64
    assert snapshot["stack"] == security_extra["stack"]
    assert snapshot["subjects"][0]["infra_failures"] == 1
    assert execution["reports"][1]["deployment"]["status"] == "infra_failed"


def test_unknown_cost_is_omitted_instead_of_recorded_as_zero(
    tmp_path: Path,
) -> None:
    write_report(
        tmp_path,
        subject_id="glm",
        scenario_id="direct_answer",
        total_cost=None,
    )

    _, efficiency, snapshot, _ = collect(config(tmp_path, ["direct_answer"]))
    efficiency_by_name = by_name(efficiency)

    assert "efficiency::glm::direct_answer::total_cost_usd" not in efficiency_by_name
    assert "efficiency::glm::suite::total_cost_usd" not in efficiency_by_name
    assert snapshot["subjects"][0]["total_cost_usd"] is None


def test_rejects_report_for_a_different_model(tmp_path: Path) -> None:
    write_report(tmp_path, subject_id="glm", scenario_id="direct_answer")
    report = next(tmp_path.rglob("results.json"))
    payload = json.loads(report.read_text())
    payload["subject"]["model"] = "different-model"
    report.write_text(json.dumps(payload))

    with pytest.raises(CollectionError, match="does not match"):
        collect(config(tmp_path, ["direct_answer"]))
