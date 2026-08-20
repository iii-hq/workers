"""Regression tests for shared worker dependency ranges."""
from __future__ import annotations

import tomllib
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]
SHARED_DEPENDENCY_RANGES = {
    "configuration": "0.x",
    "llm-router": "^1.0.0",
    "queue": "^0.21.4",
    "state": "^0.22.1",
}


def dependencies(worker: str) -> dict[str, str]:
    manifest = REPO_ROOT / worker / "iii.worker.yaml"
    data = yaml.safe_load(manifest.read_text(encoding="utf-8"))
    return data.get("dependencies", {})


def test_shared_dependencies_use_compatible_ranges() -> None:
    consumers: dict[str, list[str]] = {
        dependency: [] for dependency in SHARED_DEPENDENCY_RANGES
    }

    for manifest in sorted(REPO_ROOT.glob("*/iii.worker.yaml")):
        worker_dependencies = dependencies(manifest.parent.name)
        for dependency, expected_range in SHARED_DEPENDENCY_RANGES.items():
            if dependency in worker_dependencies:
                consumers[dependency].append(manifest.parent.name)
                dependency_range = worker_dependencies[dependency]
                assert dependency_range == expected_range, (
                    f"{manifest.relative_to(REPO_ROOT)} pins shared dependency "
                    f"{dependency} to {dependency_range!r}; use {expected_range!r} so "
                    "all consumers resolve the same compatible worker version"
                )

    for dependency, workers in consumers.items():
        assert workers, f"expected at least one consumer of {dependency}"


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


def test_provider_related_lockfiles_track_llm_router_version() -> None:
    router_manifest = tomllib.loads(
        (REPO_ROOT / "llm-router" / "Cargo.toml").read_text(encoding="utf-8"),
    )
    expected = router_manifest["package"]["version"]

    lockfiles = sorted(REPO_ROOT.glob("provider-*/Cargo.lock"))
    lockfiles.append(REPO_ROOT / "crates" / "provider-integration-testkit" / "Cargo.lock")

    for lockfile in lockfiles:
        lock = tomllib.loads(lockfile.read_text(encoding="utf-8"))
        locked_versions = [
            package["version"]
            for package in lock["package"]
            if package["name"] == "llm-router"
        ]
        if locked_versions:
            assert locked_versions == [expected], (
                f"{lockfile.relative_to(REPO_ROOT)} pins llm-router "
                f"{locked_versions}, expected {expected}"
            )
