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
        "evidence_run_id": "1234",
        "run_attempt": 1,
        "tag_sha": "a" * 40,
        "source_sha": "b" * 40,
        "release_tag": "harness/v1.2.3",
        "worker": "harness",
        "version": "1.2.3",
        "maturity": "stable",
        "deploy": "binary",
        "registry_tag": "next",
        "operation_id": "op-1",
        "step_id": "step-1",
        "image_digest": "",
        "promotable": True,
        "publish_result": "success",
        "candidate_smoke_result": "success",
        "container_alias_result": "skipped",
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def validate_args(evidence: Path, **overrides):
    values = {
        "evidence": evidence,
        "repository": "iii-hq/workers",
        "release_run_id": "1234",
        "evidence_run_id": "1234",
        "operation_id": "op-1",
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


def test_non_harness_candidate_ignores_skipped_harness_gates():
    evidence = build_evidence(
        build_args(
            worker="pdf",
            release_tag="pdf/v1.2.3",
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
        build_args(
            version="1.2.3-alpha",
            maturity="alpha",
            release_tag="harness/v1.2.3-alpha",
            promotable=False,
        )
    )
    path = write_evidence(tmp_path, evidence)
    with pytest.raises(SystemExit, match="stable semver"):
        validate_evidence(validate_args(path, version="1.2.3-alpha"))


def test_image_candidate_requires_alias_and_digest():
    evidence = build_evidence(
        build_args(
            worker="image-resize",
            release_tag="image-resize/v1.2.3",
            deploy="image",
            image_digest="sha256:" + "c" * 64,
            container_alias_result="failure",
        )
    )
    assert evidence["candidate_ready"] is False


def test_validate_accepts_legacy_schema_one_evidence(tmp_path):
    legacy = {
        "schema_version": 1,
        "repository": "iii-hq/workers",
        "release_run_id": "1234",
        "run_attempt": 1,
        "tag_sha": "a" * 40,
        "release_tag": "harness/v1.2.3",
        "worker": "harness",
        "version": "1.2.3",
        "deploy": "binary",
        "registry_tag": "next",
        "harness_gate_required": False,
        "promotable": True,
        "candidate_ready": True,
        "results": {"publish": "success", "candidate_smoke": "success"},
    }
    path = write_evidence(tmp_path, legacy)
    result = validate_evidence(validate_args(path, operation_id=""))
    assert result["schema_version"] == 1
