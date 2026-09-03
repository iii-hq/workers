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
DEPENDENCY_RANGES = {
    "configuration": "0.x",
    "console": "^1.9.11",
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
    # `skills` left this table with the worker itself: its functionality was
    # folded into iii-directory and the last consumer (mcp) dropped the
    # dependency in #1018, so a range here would fail the consumer check.
    "state": "^0.22.2",
}

WORKER_DEPENDENCY_RANGE_OVERRIDES: dict[str, dict[str, str]] = {
    # Both terminals need shell 0.12.0's named-program PTY session
    # (`shell::pty::open` with `program`/`args`/`env`) to run the agent CLI
    # with no login shell around it; other consumers stay on 0.11.9.
    "claude-code": {"shell": "^0.12.0"},
    "pi": {"shell": "^0.12.0"},
    # Speech providers need the router that carries router::transcribe and
    # router::speak and filters speech models out of chat listings; an
    # older router would offer their models to chat pickers.
    "provider-elevenlabs": {"llm-router": "^1.4.16"},
    "provider-sarvam": {"llm-router": "^1.4.16"},
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


def test_non_experimental_workers_use_validated_dependency_ranges() -> None:
    consumers: dict[str, list[str]] = {
        dependency: [] for dependency in DEPENDENCY_RANGES
    }

    for worker in sorted(
        name for name, entry in RELEASE_CATALOG.items()
        if "/" not in entry["source"]["path"]
    ):
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
                    f"{worker}/iii.worker.yaml pins shared dependency "
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
        name for name in RELEASE_CATALOG
        if name.startswith("provider-") and name not in EXPERIMENTAL_WORKERS
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
