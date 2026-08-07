"""Tests for publishing retained Harness E2E execution history."""

from __future__ import annotations

import json
from pathlib import Path

from publish_harness_e2e_dashboard import (
    build_summary,
    build_execution_efficiency_totals,
    compact_public_detail,
    contract_fingerprint,
    execution_status,
    load_manifest,
    publish,
    report_diagnostic,
    scenario_contract,
)


def write_json(path: Path, value: dict) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))
    return path


def metadata(run_id: str, attempt: int = 1, *, conclusion: str = "success") -> dict:
    day = int(run_id[-2:])
    return {
        "id": f"{run_id}-{attempt}",
        "run_id": run_id,
        "attempt": attempt,
        "workflow_name": "Harness E2E Daily",
        "workflow_url": f"https://github.com/iii-hq/workers/actions/runs/{run_id}",
        "event": "schedule",
        "actor": "github-actions",
        "started_at": f"2026-07-{day:02d}T06:00:00Z",
        "completed_at": f"2026-07-{day:02d}T06:10:00Z",
        "conclusion": conclusion,
        "head_sha": str(day) * 40,
        "head_branch": "main",
        "repository": "iii-hq/workers",
    }


def snapshot(run_id: str, attempt: int = 1, *, passed: bool = True) -> dict:
    return {
        "schema_version": 2,
        "execution": {
            "id": f"{run_id}-{attempt}",
            "run_id": run_id,
            "attempt": attempt,
        },
        "generated_at": metadata(run_id, attempt)["completed_at"],
        "lane": "daily",
        "source": {"sha": "a" * 40, "ref": "main", "repository": "iii-hq/workers"},
        "workflow_url": metadata(run_id, attempt)["workflow_url"],
        "release": {"tag": f"daily/2026-07-{int(run_id[-2:]):02d}"},
        "requested_runs": 3,
        "subjects": [
            {
                "id": "glm",
                "model": "glm-5.2",
                "provider": "zai",
                "passed": passed,
                "expected_reports": 1,
                "received_reports": 1,
                "hard_gate_failures": 0 if passed else 1,
                "technical_failures": 0,
                "retry_attempts": 0,
                "total_cost_usd": 1.5,
                "wall_time_seconds": 90,
                "scenarios": [
                    {
                        "id": "direct_answer",
                        "status": "passed" if passed else "failed",
                        "passed": passed,
                        "threshold": 80,
                        "runs": 3,
                        "median_score": 92 if passed else 60,
                        "pass_rate": 1 if passed else 0.33,
                        "hard_gate_failures": 0 if passed else 1,
                        "technical_failures": 0,
                        "retries": 0,
                        "total_cost_usd": 1.5,
                        "wall_time_seconds": 90,
                    }
                ],
            }
        ],
    }


def detail(run_id: str, attempt: int = 1) -> dict:
    value = snapshot(run_id, attempt)
    value["reports"] = [
        {
            "subject_id": "glm",
            "scenario_id": "direct_answer",
            "available": True,
            "report": {
                "subject": {"model": "glm-5.2", "provider": "zai"},
                "scenarios": [
                    {
                        "scenario_id": "direct_answer",
                        "scenario_version": 1,
                        "threshold": 80,
                        "execution_policy": {
                            "max_turns": 2,
                            "max_output_tokens": 2048,
                            "max_total_tokens": 32768,
                            "stuck_timeout_seconds": 120,
                        },
                        "runs": [
                            {
                                "prompt": "public prompt",
                                "transcript": {"messages": []},
                                "wall_time_ms": 10_000,
                                "cost": {"total_usd": 0.2},
                                "metrics": {
                                    "totals": {
                                        "input_tokens": 100,
                                        "output_tokens": 20,
                                        "function_calls": 2,
                                        "function_call_errors": 0,
                                        "sessions": 1,
                                        "turns": 4,
                                    }
                                },
                            },
                            {
                                "prompt": "public prompt",
                                "transcript": {"messages": []},
                                "wall_time_ms": 20_000,
                                "cost": {"total_usd": 0.6},
                                "metrics": {
                                    "totals": {
                                        "input_tokens": 200,
                                        "output_tokens": 40,
                                        "function_calls": 4,
                                        "function_call_errors": 2,
                                        "sessions": 3,
                                        "turns": 8,
                                    }
                                },
                            },
                        ],
                    }
                ],
            },
        }
    ]
    return value


def publish_run(
    root: Path,
    run_id: str,
    *,
    attempt: int = 1,
    include_snapshot: bool = True,
    include_detail: bool = True,
    conclusion: str = "success",
    jobs: dict | None = None,
) -> dict:
    inputs = root / "inputs"
    snapshot_path = (
        write_json(
            inputs / f"{run_id}-{attempt}-snapshot.json",
            snapshot(run_id, attempt),
        )
        if include_snapshot
        else None
    )
    detail_path = (
        write_json(inputs / f"{run_id}-{attempt}-detail.json", detail(run_id, attempt))
        if include_detail
        else None
    )
    jobs_path = (
        write_json(inputs / f"{run_id}-{attempt}-jobs.json", jobs)
        if jobs is not None
        else None
    )
    return publish(
        root / "site",
        snapshot_path=snapshot_path,
        detail_path=detail_path,
        jobs_path=jobs_path,
        metadata=metadata(run_id, attempt, conclusion=conclusion),
        repo_url="https://github.com/iii-hq/workers",
        max_summaries=3,
        max_details=2,
    )


def test_retains_summaries_and_prunes_complete_details(tmp_path: Path) -> None:
    publish_run(tmp_path, "10000000001")
    publish_run(tmp_path, "10000000002")
    manifest = publish_run(tmp_path, "10000000003")

    assert [entry["id"] for entry in manifest["executions"]] == [
        "10000000003-1",
        "10000000002-1",
        "10000000001-1",
    ]
    assert [entry["availability"] for entry in manifest["executions"]] == [
        "full",
        "full",
        "aggregate",
    ]
    assert not (tmp_path / "site/runs/10000000001-1.json").exists()
    assert (tmp_path / "site/runs/10000000002-1.json").is_file()

    manifest = publish_run(tmp_path, "10000000004")
    assert [entry["id"] for entry in manifest["executions"]] == [
        "10000000004-1",
        "10000000003-1",
        "10000000002-1",
    ]
    assert not (tmp_path / "site/runs/10000000002-1.json").exists()


def test_same_day_attempts_are_distinct_and_updates_are_idempotent(
    tmp_path: Path,
) -> None:
    publish_run(tmp_path, "10000000005", attempt=1)
    publish_run(tmp_path, "10000000005", attempt=2)
    publish_run(tmp_path, "10000000005", attempt=2)

    manifest = load_manifest(tmp_path / "site/executions.js")
    assert [entry["id"] for entry in manifest["executions"]] == [
        "10000000005-2",
        "10000000005-1",
    ]
    assert all(entry["availability"] == "full" for entry in manifest["executions"])


def test_records_cancelled_execution_without_artifacts(tmp_path: Path) -> None:
    manifest = publish_run(
        tmp_path,
        "10000000006",
        include_snapshot=False,
        include_detail=False,
        conclusion="cancelled",
    )

    execution = manifest["executions"][0]
    assert execution["status"] == "cancelled"
    assert execution["availability"] == "unavailable"
    assert execution["subjects"] == []


def test_publishes_schema_three_and_classifies_semantic_outcomes() -> None:
    complete_subjects = [{"passed": True}]
    advisory_subjects = [{"passed": False}]

    assert execution_status("", [], 0, 0, 0, 0) == "running"
    assert execution_status("cancelled", [], 0, 0, 0, 0) == "cancelled"
    assert execution_status("failure", [], 0, 0, 0, 0) == "infra_failed"
    assert execution_status("success", complete_subjects, 2, 1, 0, 0) == "incomplete"
    assert (
        execution_status("failure", complete_subjects, 1, 1, 0, 1)
        == "technical_failed"
    )
    assert (
        execution_status("failure", complete_subjects, 1, 1, 2, 0)
        == "hard_gate_failed"
    )
    assert (
        execution_status("success", advisory_subjects, 1, 1, 0, 0)
        == "quality_advisory"
    )
    assert execution_status("success", complete_subjects, 1, 1, 0, 0) == "passed"


def test_failed_build_without_artifacts_uses_first_failed_job(tmp_path: Path) -> None:
    jobs = {
        "jobs": [
            {
                "name": "harness e2e build",
                "conclusion": "failure",
                "started_at": "2026-07-06T06:00:00Z",
                "html_url": "https://github.com/iii-hq/workers/actions/runs/10000000006/job/1",
                "steps": [
                    {"number": 1, "name": "Checkout", "conclusion": "success"},
                    {
                        "number": 2,
                        "name": "Validate E2E manifests and lockfiles",
                        "conclusion": "failure",
                    },
                ],
            }
        ]
    }
    manifest = publish_run(
        tmp_path,
        "10000000016",
        include_snapshot=False,
        include_detail=False,
        conclusion="failure",
        jobs=jobs,
    )

    execution = manifest["executions"][0]
    assert manifest["schema_version"] == 3
    assert execution["status"] == "infra_failed"
    assert execution["first_failure"] == {
        "kind": "job",
        "job_name": "harness e2e build",
        "step_name": "Validate E2E manifests and lockfiles",
        "message": "harness e2e build: Validate E2E manifests and lockfiles",
        "url": "https://github.com/iii-hq/workers/actions/runs/10000000006/job/1",
    }
    assert "steps" not in json.dumps(execution)


def test_quality_only_failure_is_advisory_and_surfaces_score() -> None:
    value = snapshot("10000000017", passed=False)
    subject = value["subjects"][0]
    subject["hard_gate_failures"] = 0
    subject["scenarios"][0]["hard_gate_failures"] = 0

    summary = build_summary(value, metadata("10000000017"))

    assert summary["status"] == "quality_advisory"
    assert summary["first_failure"] == {
        "kind": "quality",
        "subject_id": "glm",
        "scenario_id": "direct_answer",
        "message": "Median score 60 is below threshold 80",
    }


def test_hard_gate_event_blocks_even_when_subject_aggregate_passed() -> None:
    value = snapshot("10000000018", passed=True)
    value["subjects"][0]["hard_gate_failures"] = 1
    value["subjects"][0]["scenarios"][0]["hard_gate_failures"] = 1

    summary = build_summary(value, metadata("10000000018", conclusion="failure"))

    assert summary["status"] == "hard_gate_failed"


def test_extracts_first_technical_and_hard_gate_diagnostics() -> None:
    value = detail("10000000020")
    run = value["reports"][0]["report"]["scenarios"][0]["runs"][0]
    run["failures"] = [{"phase": "execute", "message": "provider unavailable"}]
    run["hard_gates"] = [
        {"id": "required_state", "passed": False, "reason": "state missing"}
    ]

    assert report_diagnostic("technical_failed", None, value) == {
        "kind": "technical",
        "subject_id": "glm",
        "scenario_id": "direct_answer",
        "phase": "execute",
        "message": "provider unavailable",
    }
    assert report_diagnostic("hard_gate_failed", None, value) == {
        "kind": "hard_gate",
        "subject_id": "glm",
        "scenario_id": "direct_answer",
        "id": "required_state",
        "message": "state missing",
    }


def test_duplicate_event_does_not_downgrade_existing_full_report(
    tmp_path: Path,
) -> None:
    publish_run(tmp_path, "10000000007")
    manifest = publish_run(
        tmp_path,
        "10000000007",
        include_snapshot=False,
        include_detail=False,
    )

    assert manifest["executions"][0]["availability"] == "full"
    assert (tmp_path / "site/runs/10000000007-1.json").is_file()


def test_publishes_average_run_metrics_by_scenario(tmp_path: Path) -> None:
    manifest = publish_run(tmp_path, "10000000008")
    scenario = detail("10000000008")["reports"][0]["report"]["scenarios"][0]
    fingerprint = contract_fingerprint(
        scenario_contract(scenario, "direct_answer", scenario["runs"])
    )

    assert build_execution_efficiency_totals(detail("10000000008")) == {
        "total_tokens": 360,
        "function_calls": 6,
    }
    assert manifest["executions"][0]["totals"]["total_tokens"] == 360
    assert manifest["executions"][0]["totals"]["function_calls"] == 6

    assert manifest["executions"][0]["scenario_metrics"] == [
        {
            "subject_id": "glm",
            "scenario_id": "direct_answer",
            "scenario_version": 1,
            "contract_fingerprint": fingerprint,
            "run_count": 2,
            "averages": {
                "tokens": 180,
                "duration_seconds": 15,
                "cost_usd": 0.4,
                "function_calls": 3,
                "function_call_errors": 1,
                "sessions": 2,
                "turns": 6,
            },
            "samples": {
                "tokens": 2,
                "duration_seconds": 2,
                "cost_usd": 2,
                "function_calls": 2,
                "function_call_errors": 2,
                "sessions": 2,
                "turns": 2,
            },
        }
    ]


def test_public_detail_excludes_prompts_transcripts_and_payloads(tmp_path: Path) -> None:
    raw = detail("10000000019")
    run = raw["reports"][0]["report"]["scenarios"][0]["runs"][0]
    run.update(
        {
            "prompt": "SECRET_PROMPT_MARKER",
            "transcript": {
                "messages": [
                    {
                        "role": "assistant",
                        "content": "SECRET_TRANSCRIPT_MARKER",
                        "arguments": {"token": "SECRET_TOOL_ARGUMENT_MARKER"},
                    }
                ]
            },
            "criteria": [
                {"id": "private", "reason": "SECRET_CRITERION_MARKER"}
            ],
            "hard_gates": [
                {"id": "required_state", "passed": False, "reason": "state missing"}
            ],
            "failures": [{"phase": "execute", "message": "provider unavailable"}],
        }
    )
    compact = compact_public_detail(raw, metadata("10000000019"))

    encoded = json.dumps(compact)
    for marker in (
        "SECRET_PROMPT_MARKER",
        "SECRET_TRANSCRIPT_MARKER",
        "SECRET_TOOL_ARGUMENT_MARKER",
        "SECRET_CRITERION_MARKER",
    ):
        assert marker not in encoded
    for forbidden_key in ('"prompt"', '"transcript"', '"criteria"', '"arguments"'):
        assert forbidden_key not in encoded
    public_run = compact["reports"][0]["report"]["scenarios"][0]["runs"][0]
    assert public_run["hard_gates"] == [
        {"id": "required_state", "passed": False, "reason": "state missing"}
    ]
    assert public_run["failures"] == [
        {"phase": "execute", "message": "provider unavailable"}
    ]
    assert public_run["metrics"]["totals"]["input_tokens"] == 100

    inputs = tmp_path / "inputs"
    manifest = publish(
        tmp_path / "site",
        snapshot_path=write_json(inputs / "snapshot.json", snapshot("10000000019")),
        detail_path=write_json(inputs / "detail.json", raw),
        metadata=metadata("10000000019"),
        repo_url="https://github.com/iii-hq/workers",
        max_summaries=3,
        max_details=2,
    )
    published = (tmp_path / "site" / manifest["executions"][0]["detail_path"]).read_text()
    assert "SECRET_" not in published
    assert '"prompt"' not in published
    assert '"transcript"' not in published


def test_next_publish_sanitizes_retained_schema_two_details(tmp_path: Path) -> None:
    first = publish_run(tmp_path, "10000000021")
    retained_path = tmp_path / "site" / first["executions"][0]["detail_path"]
    legacy = detail("10000000021")
    legacy["reports"][0]["report"]["scenarios"][0]["runs"][0][
        "prompt"
    ] = "SECRET_RETAINED_PROMPT"
    retained_path.write_text(json.dumps(legacy))

    publish_run(tmp_path, "10000000022")

    migrated = retained_path.read_text()
    assert "SECRET_RETAINED_PROMPT" not in migrated
    assert '"prompt"' not in migrated
    assert json.loads(migrated)["schema_version"] == 3


def test_ignores_artifacts_from_a_previous_attempt(tmp_path: Path) -> None:
    current_metadata = metadata("10000000023", attempt=2, conclusion="failure")
    stale_snapshot = snapshot("10000000023", attempt=1)
    stale_detail = detail("10000000023", attempt=1)
    stale_detail["reports"][0]["report"]["scenarios"][0]["runs"][0][
        "prompt"
    ] = "SECRET_STALE_ATTEMPT"
    inputs = tmp_path / "inputs"

    manifest = publish(
        tmp_path / "site",
        snapshot_path=write_json(inputs / "snapshot.json", stale_snapshot),
        detail_path=write_json(inputs / "detail.json", stale_detail),
        metadata=current_metadata,
        repo_url="https://github.com/iii-hq/workers",
        max_summaries=3,
        max_details=2,
    )

    execution = manifest["executions"][0]
    assert execution["id"] == "10000000023-2"
    assert execution["status"] == "infra_failed"
    assert execution["availability"] == "unavailable"
    assert execution["subjects"] == []
    assert execution["detail_path"] is None
    assert execution["first_failure"]["kind"] == "artifact_identity"
    assert "expected 10000000023 attempt 2" in execution["first_failure"]["message"]
    assert not list((tmp_path / "site/runs").glob("*.json"))
