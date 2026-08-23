"""Regression tests for worker dependency compatibility."""
from __future__ import annotations

import tomllib
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]
EXPERIMENTAL_WORKERS = {
    "a2ui",
    "canvas",
    "document",
    "eval",
    "pdf",
    "provider-opencode-go",
    "provider-openrouter",
}
DEPENDENCY_RANGES = {
    "configuration": "0.x",
    "console": "^1.9.12",
    "context-manager": "^1.1.3",
    "cron": "^0.21.9",
    "harness": "^1.8.5",
    "iii-directory": "^1.2.3",
    "iii-observability": "0.x",
    "iii-stream": "0.x",
    "llm-router": "^1.4.12",
    "memory": "^0.2.2",
    "provider-anthropic": "^1.2.8",
    "provider-openai": "^1.2.7",
    "provider-openai-codex": "^0.4.4",
    "queue": "^0.21.5",
    "session-manager": "^1.0.13",
    "shell": "^0.11.9",
    "skills": "0.x",
    "state": "^0.22.2",
}

# Harness is installed from its candidate channel while its stable release is
# pending. Stable workers must resolve only stable dependency releases.
WORKER_DEPENDENCY_RANGE_OVERRIDES = {
    "harness": {"shell": "^0.11.10"},
}


def dependencies(worker: str) -> dict[str, str]:
    manifest = REPO_ROOT / worker / "iii.worker.yaml"
    data = yaml.safe_load(manifest.read_text(encoding="utf-8"))
    return data.get("dependencies", {})


def test_worker_dependency_graph_is_acyclic() -> None:
    workers = {
        manifest.parent.name
        for manifest in REPO_ROOT.glob("*/iii.worker.yaml")
    }
    graph = {
        worker: set(dependencies(worker)).intersection(workers)
        for worker in workers
    }
    visiting: list[str] = []
    visited: set[str] = set()

    def visit(worker: str) -> None:
        if worker in visited:
            return
        if worker in visiting:
            start = visiting.index(worker)
            cycle = [*visiting[start:], worker]
            raise AssertionError(f"worker dependency cycle: {' -> '.join(cycle)}")

        visiting.append(worker)
        for dependency in sorted(graph[worker]):
            visit(dependency)
        visiting.pop()
        visited.add(worker)

    for worker in sorted(workers):
        visit(worker)


def test_non_experimental_workers_use_validated_dependency_ranges() -> None:
    consumers: dict[str, list[str]] = {
        dependency: [] for dependency in DEPENDENCY_RANGES
    }

    for manifest in sorted(REPO_ROOT.glob("*/iii.worker.yaml")):
        worker = manifest.parent.name
        if worker in EXPERIMENTAL_WORKERS:
            continue
        worker_dependencies = dependencies(worker)
        worker_overrides = WORKER_DEPENDENCY_RANGE_OVERRIDES.get(worker, {})
        for dependency, expected_range in DEPENDENCY_RANGES.items():
            if dependency in worker_dependencies:
                expected_range = worker_overrides.get(dependency, expected_range)
                consumers[dependency].append(worker)
                dependency_range = worker_dependencies[dependency]
                assert dependency_range == expected_range, (
                    f"{manifest.relative_to(REPO_ROOT)} pins shared dependency "
                    f"{dependency} to {dependency_range!r}; use {expected_range!r} so "
                    "non-experimental consumers resolve the validated worker release"
                )

    for dependency, workers in consumers.items():
        assert workers, f"expected at least one consumer of {dependency}"


def test_worker_dependency_range_overrides_are_active() -> None:
    for worker, overrides in WORKER_DEPENDENCY_RANGE_OVERRIDES.items():
        worker_dependencies = dependencies(worker)
        for dependency, expected_range in overrides.items():
            assert worker_dependencies.get(dependency) == expected_range


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
        if manifest.parent.name not in EXPERIMENTAL_WORKERS
    )

    for worker in ("llm-router", *providers):
        assert dependencies(worker)["state"] == expected


def test_provider_related_lockfiles_track_llm_router_version() -> None:
    router_manifest = tomllib.loads(
        (REPO_ROOT / "llm-router" / "Cargo.toml").read_text(encoding="utf-8"),
    )
    expected = router_manifest["package"]["version"]

    lockfiles = sorted(
        lockfile
        for lockfile in REPO_ROOT.glob("provider-*/Cargo.lock")
        if lockfile.parent.name not in EXPERIMENTAL_WORKERS
    )
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
