from __future__ import annotations

import argparse

from release_result import build_result


def args(**overrides):
    values = {
        "repository": "iii-hq/workers",
        "release_run_id": "123",
        "run_attempt": 1,
        "operation_id": "op",
        "step_id": "step",
        "worker": "pdf",
        "version": "1.2.3",
        "maturity": "stable",
        "release_tag": "pdf/v1.2.3",
        "tag_sha": "a" * 40,
        "source_sha": "b" * 40,
        "deploy": "binary",
        "registry_tag": "next",
        "image_digest": "",
        "dry_run": "false",
        "staged": "true",
        "interface_smoke": "true",
        "setup_result": "success",
        "github_release_result": "success",
        "binary_build_result": "success",
        "container_build_result": "skipped",
        "bundle_build_result": "skipped",
        "publish_result": "success",
        "container_alias_result": "skipped",
        "candidate_result": "success",
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def test_successful_release_result_contains_only_release_truth():
    result = build_result(args())
    assert result["status"] == "succeeded"
    assert "notification" not in result


def test_failure_after_registry_publish_is_partial():
    result = build_result(args(candidate_result="failure"))
    assert result["status"] == "partial"
    assert result["phase"] == "channel_aligned"


def test_image_requires_alias_after_publish():
    result = build_result(
        args(
            deploy="image",
            binary_build_result="skipped",
            container_build_result="success",
            container_alias_result="failure",
        )
    )
    assert result["status"] == "partial"
    assert "container_alias" in result["failed_requirements"]
