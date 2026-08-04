from __future__ import annotations

import argparse
import json
from pathlib import Path

import pytest

from release_candidate import build_evidence, validate_evidence


def build_args(**overrides):
    values = {
        "repository": "iii-hq/workers",
        "release_run_id": "1234",
        "run_attempt": 1,
        "tag_sha": "a" * 40,
        "release_tag": "harness/v1.2.3",
        "worker": "harness",
        "version": "1.2.3",
        "deploy": "binary",
        "registry_tag": "next",
        "harness_gate_required": True,
        "promotable": True,
        "publish_result": "success",
        "candidate_smoke_result": "success",
        "harness_quickstart_result": "success",
        "harness_e2e_result": "success",
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def validate_args(evidence: Path, **overrides):
    values = {
        "evidence": evidence,
        "repository": "iii-hq/workers",
        "release_run_id": "1234",
        "worker": "harness",
        "version": "1.2.3",
        "output": None,
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def write_evidence(tmp_path: Path, evidence: dict) -> Path:
    path = tmp_path / "evidence.json"
    path.write_text(json.dumps(evidence))
    return path


def test_build_marks_complete_harness_candidate_ready():
    evidence = build_evidence(build_args())
    assert evidence["candidate_ready"] is True
    assert evidence["promotable"] is True


def test_build_requires_harness_e2e_when_applicable():
    evidence = build_evidence(build_args(harness_e2e_result="failure"))
    assert evidence["candidate_ready"] is False


def test_non_harness_candidate_ignores_skipped_harness_gates():
    evidence = build_evidence(
        build_args(
            worker="pdf",
            release_tag="pdf/v1.2.3",
            harness_gate_required=False,
            harness_quickstart_result="skipped",
            harness_e2e_result="skipped",
        )
    )
    assert evidence["candidate_ready"] is True


def test_validate_accepts_matching_promotable_evidence(tmp_path):
    path = write_evidence(tmp_path, build_evidence(build_args()))
    assert validate_evidence(validate_args(path))["worker"] == "harness"


@pytest.mark.parametrize(
    ("override", "message"),
    [
        ({"promotable": False}, "promotable"),
        ({"candidate_ready": False}, "candidate_ready"),
        ({"release_tag": "harness/v9.9.9"}, "release_tag"),
        ({"registry_tag": "latest"}, "registry_tag"),
    ],
)
def test_validate_rejects_mismatched_or_unready_evidence(tmp_path, override, message):
    evidence = build_evidence(build_args())
    evidence.update(override)
    path = write_evidence(tmp_path, evidence)
    with pytest.raises(SystemExit, match=message):
        validate_evidence(validate_args(path))


def test_validate_rejects_prerelease_version(tmp_path):
    evidence = build_evidence(
        build_args(version="1.2.3-alpha.1", release_tag="harness/v1.2.3-alpha.1", promotable=False)
    )
    path = write_evidence(tmp_path, evidence)
    with pytest.raises(SystemExit, match="stable semver"):
        validate_evidence(validate_args(path, version="1.2.3-alpha.1"))
