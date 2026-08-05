from __future__ import annotations

import argparse
import json

import pytest

from harness_e2e_evidence import build_evidence, validate_evidence


def build_args(**overrides):
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
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def test_build_marks_matching_green_suite_ready():
    assert build_evidence(build_args())["e2e_ready"] is True
    assert build_evidence(build_args())["stack_versions"]["state"] == "0.22.0"


def test_build_rejects_latest_as_promotion_evidence():
    assert build_evidence(build_args(registry_tag="latest"))["e2e_ready"] is False


def test_validate_checks_release_identity(tmp_path):
    path = tmp_path / "evidence.json"
    path.write_text(json.dumps(build_evidence(build_args())))
    args = argparse.Namespace(
        evidence=path,
        repository="iii-hq/workers",
        release_run_id="100",
        e2e_run_id="200",
        worker="harness",
        version="1.2.3",
        operation_id="",
        stack_versions='{"harness":"1.2.3","state":"0.22.0"}',
        output=None,
    )
    assert validate_evidence(args)["e2e_ready"] is True
    args.version = "1.2.4"
    with pytest.raises(SystemExit, match="release_version"):
        validate_evidence(args)
