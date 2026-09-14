"""Regression tests for worker dependency compatibility."""
from __future__ import annotations

import tomllib
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]
RELEASE_CATALOG = yaml.safe_load(
    (REPO_ROOT / ".deploy" / "workers.yaml").read_text(encoding="utf-8")
)["workers"]
EXPERIMENTAL_WORKERS = {
    "a2ui",
    "canvas",
    "document",
    "eval",
    "pdf",
    "provider-opencode-go",
    "provider-openrouter",
}
def dependencies(worker: str) -> dict[str, str]:
    worker_path = RELEASE_CATALOG[worker]["source"]["path"]
    manifest = yaml.safe_load(
        (REPO_ROOT / worker_path / "iii.worker.yaml").read_text(encoding="utf-8")
    )
    return manifest.get("dependencies", {})


def test_worker_dependency_graph_is_acyclic() -> None:
    workers = {
        name for name, entry in RELEASE_CATALOG.items()
        if "/" not in entry["source"]["path"]
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


def test_workers_use_latest_runtime_dependencies() -> None:
    for worker in RELEASE_CATALOG:
        for dependency, selector in dependencies(worker).items():
            assert selector == "latest", (
                f"{worker}/iii.worker.yaml dependency {dependency} uses {selector!r}"
            )


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
