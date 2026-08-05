from __future__ import annotations

import argparse
import json

import pytest

from harness_e2e_evidence import build_evidence, validate_evidence
from harness_e2e_profiles import CATALOG_PATH, load_profile_catalog


def snapshot(tmp_path, scenarios: list[str], *, missing: str | None = None):
    path = tmp_path / "snapshot.json"
    path.write_text(
        json.dumps(
            {
                "requested_runs": 1,
                "subjects": [
                    {
                        "id": "anthropic-sonnet",
                        "model": "claude-sonnet-4-6",
                        "provider": "anthropic",
                        "scenarios": [
                            {
                                "id": scenario,
                                "status": "missing_report" if scenario == missing else "passed",
                                "runs": 0 if scenario == missing else 1,
                            }
                            for scenario in scenarios
                        ],
                    }
                ],
            }
        )
    )
    return path


def build_args(tmp_path, **overrides):
    catalog = load_profile_catalog()
    selected = list(catalog.release_scenarios)
    values = {
        "repository": "iii-hq/workers",
        "release_run_id": "100",
        "e2e_run_id": "200",
        "run_attempt": 1,
        "operation_id": "op",
        "step_id": "e2e",
        "release_tag": "harness/v1.2.3",
        "release_worker": "harness",
        "release_version": "1.2.3",
        "registry_tag": "next",
        "cli_channel": "next",
        "suite_result": "success",
        "stack_versions": '{"harness":"1.2.3","state":"0.22.0"}',
        "validation_profile": "release",
        "scenarios_json": json.dumps(selected),
        "required_scenarios_json": json.dumps(selected),
        "profile_digest": catalog.profile_digest,
        "catalog_sha": "a" * 40,
        "suite_sha": "a" * 40,
        "runs": 1,
        "benchmark_snapshot": snapshot(tmp_path, selected),
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def validate_args(path, **overrides):
    values = {
        "evidence": path,
        "repository": "iii-hq/workers",
        "release_run_id": "100",
        "e2e_run_id": "200",
        "worker": "harness",
        "version": "1.2.3",
        "operation_id": "",
        "catalog": CATALOG_PATH,
        "output": None,
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def test_build_marks_matching_release_gate_ready(tmp_path):
    evidence = build_evidence(build_args(tmp_path))
    assert evidence["e2e_ready"] is True
    assert evidence["promotion_ready"] is True
    assert evidence["completed_scenarios"] == evidence["selected_scenarios"]
    assert evidence["stack_versions"]["state"] == "0.22.0"


def test_build_keeps_green_custom_subset_non_promotable(tmp_path):
    selected = ["persistent_state"]
    args = build_args(tmp_path, validation_profile="custom", scenarios_json=json.dumps(selected))
    args.benchmark_snapshot = snapshot(tmp_path, selected)
    evidence = build_evidence(args)
    assert evidence["e2e_ready"] is True
    assert evidence["promotion_ready"] is False


def test_build_rejects_missing_report_and_latest(tmp_path):
    args = build_args(tmp_path)
    selected = json.loads(args.scenarios_json)
    args.benchmark_snapshot = snapshot(tmp_path, selected, missing=selected[0])
    assert build_evidence(args)["e2e_ready"] is False
    assert build_evidence(build_args(tmp_path, registry_tag="latest"))["e2e_ready"] is False


def test_build_records_an_early_suite_failure_with_catalog_fallbacks(tmp_path):
    evidence = build_evidence(
        build_args(
            tmp_path,
            suite_result="failure",
            scenarios_json="[]",
            required_scenarios_json="[]",
            profile_digest="",
            benchmark_snapshot=None,
        )
    )
    catalog = load_profile_catalog()
    assert evidence["selected_scenarios"] == list(catalog.release_scenarios)
    assert evidence["required_scenarios"] == list(catalog.release_scenarios)
    assert evidence["profile_digest"] == catalog.profile_digest
    assert evidence["e2e_ready"] is False
    assert evidence["promotion_ready"] is False


def test_validate_checks_v3_release_identity_and_gate(tmp_path):
    path = tmp_path / "evidence.json"
    evidence = build_evidence(build_args(tmp_path))
    path.write_text(json.dumps(evidence))
    assert validate_evidence(validate_args(path))["promotion_ready"] is True
    evidence["promotion_ready"] = False
    path.write_text(json.dumps(evidence))
    with pytest.raises(SystemExit, match="promotion_ready"):
        validate_evidence(validate_args(path))


def test_validate_accepts_legacy_full_evidence(tmp_path):
    path = tmp_path / "evidence.json"
    path.write_text(
        json.dumps(
            {
                "schema_version": 2,
                "repository": "iii-hq/workers",
                "release_run_id": "100",
                "e2e_run_id": "200",
                "run_attempt": 1,
                "release_tag": "harness/v1.2.3",
                "release_worker": "harness",
                "release_version": "1.2.3",
                "registry_tag": "next",
                "suite_result": "success",
                "stack_versions": {"harness": "1.2.3"},
                "e2e_ready": True,
            }
        )
    )
    assert validate_evidence(validate_args(path))["schema_version"] == 2
