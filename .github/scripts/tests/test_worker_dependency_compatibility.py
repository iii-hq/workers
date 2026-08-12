"""Regression tests for shared worker dependency ranges."""
from __future__ import annotations

from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]


def dependencies(worker: str) -> dict[str, str]:
    manifest = REPO_ROOT / worker / "iii.worker.yaml"
    data = yaml.safe_load(manifest.read_text(encoding="utf-8"))
    return data.get("dependencies", {})


def test_approval_gate_uses_shared_runtime_dependency_ranges() -> None:
    approval_gate = dependencies("approval-gate")

    expected_ranges = {
        "configuration": dependencies("iii-directory")["configuration"],
        "state": dependencies("harness")["state"],
    }

    for dependency, expected in expected_ranges.items():
        assert approval_gate[dependency] == expected


def test_harness_llm_stack_uses_shared_state_range() -> None:
    expected = dependencies("harness")["state"]
    providers = sorted(
        manifest.parent.name
        for manifest in REPO_ROOT.glob("provider-*/iii.worker.yaml")
    )

    for worker in ("llm-router", *providers):
        assert dependencies(worker)["state"] == expected
