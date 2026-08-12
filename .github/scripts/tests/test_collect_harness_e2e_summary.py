from __future__ import annotations

import argparse
import json
from pathlib import Path

from collect_harness_e2e_summary import build_summary


def args(root: Path, **overrides: object) -> argparse.Namespace:
    values: dict[str, object] = {
        "reports_root": root,
        "subjects": '[{"id":"subject","model":"model","provider":"provider"}]',
        "scenarios_json": '["scenario"]',
        "required_scenarios_json": '["scenario"]',
        "lane": "daily",
        "profile": "full",
        "profile_digest": "d" * 64,
        "catalog_sha": "a" * 40,
        "requested_runs": 1,
        "source_sha": "a" * 40,
        "source_ref": "main",
        "repository": "iii-hq/workers",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/123",
        "workflow_sha": "b" * 40,
        "workflow_ref": "refs/heads/main",
        "release_worker": "none",
        "release_version": "none",
        "release_tag": "none",
        "registry_tag": "next",
        "stack_versions": '{"harness":"1.2.3"}',
        "judge_model": "judge",
        "judge_provider": "provider",
        "run_id": 123,
        "run_attempt": 1,
        "event": "workflow_dispatch",
        "actor": "iii-release-control[bot]",
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def write_report(root: Path, *, retries: bool = False) -> None:
    directory = root / "result"
    directory.mkdir()
    (directory / "benchmark-context.json").write_text(
        json.dumps({"subject_id": "subject", "scenario_id": "scenario"})
    )
    retry_attempts = (
        [
            {
                "status": "subject_error",
                "wall_time_ms": 10,
                "cost": {"subject_usd": 0.01, "judge_usd": 0, "total_usd": 0.01},
                "failures": [{"phase": "execute", "message": "retry"}],
            }
        ]
        if retries
        else []
    )
    (directory / "results.json").write_text(
        json.dumps(
            {
                "subject": {"model": "model", "provider": "provider"},
                "judge": {"model": "judge", "provider": "provider"},
                "judge_protocol": "plain-json",
                "engine_revision": "c" * 40,
                "passed": True,
                "scenarios": [
                    {
                        "scenario_id": "scenario",
                        "scenario_version": 2,
                        "execution_policy": {"timeout_seconds": 60},
                        "passed": True,
                        "runs": [
                            {
                                "run_id": "secret-run-id",
                                "session_id": "secret-session-id",
                                "prompt": "secret prompt",
                                "status": "passed",
                                "score": 91,
                                "wall_time_ms": 100,
                                "hard_gates": [{"id": "gate", "passed": True, "reason": "private"}],
                                "metrics": {
                                    "totals": {
                                        "input_tokens": 10,
                                        "output_tokens": 2,
                                        "function_calls": 3,
                                        "function_call_errors": 1,
                                    }
                                },
                                "judge_usage": {"input_tokens": 4, "output_tokens": 1},
                                "judge_attempts": 1,
                                "cost": {"subject_usd": 0.1, "judge_usd": 0.02, "total_usd": 0.12},
                                "retry_attempts": retry_attempts,
                                "failures": [],
                            }
                        ],
                    }
                ],
            }
        )
    )


def test_builds_complete_privacy_safe_canonical_summary(tmp_path: Path) -> None:
    write_report(tmp_path, retries=True)
    summary = build_summary(args(tmp_path))
    assert summary["schema_version"] == 1
    assert summary["status"] == "passed"
    assert summary["data_availability"] == "complete"
    assert summary["release"]["registry_tag"] == "next"
    assert summary["coverage"] == {
        "expected_reports": 1,
        "received_reports": 1,
        "expected_samples": 1,
        "received_samples": 1,
        "complete_samples": 1,
        "scored_samples": 1,
    }
    assert [(sample["attempt"], sample["terminal"]) for sample in summary["samples"]] == [(1, False), (2, True)]
    assert summary["samples"][1]["contract_fingerprint"].startswith("sha256:")
    rendered = json.dumps(summary)
    assert "secret prompt" not in rendered
    assert "secret-session-id" not in rendered
    assert "private" not in rendered


def test_reports_missing_matrix_cells_as_partial(tmp_path: Path) -> None:
    summary = build_summary(args(tmp_path))
    assert summary["status"] == "incomplete"
    assert summary["data_availability"] == "unavailable"
    assert summary["coverage"]["received_reports"] == 0
    assert summary["samples"] == []
