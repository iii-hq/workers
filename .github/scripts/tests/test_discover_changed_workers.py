from __future__ import annotations

import sys
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import discover_changed_workers as discover  # noqa: E402


def harness_changed(files: list[str]) -> bool:
    return discover.suite_changed(
        files,
        set(),
        discover.INTEGRATION_WORKERS,
        discover.INTEGRATION_INFRA_PATHS,
        discover.INTEGRATION_EXCLUDED_PREFIXES,
    )


def test_harness_ignores_worker_release_metadata() -> None:
    assert harness_changed(["console/Cargo.toml", "console/Cargo.lock"]) is False


def test_harness_runs_for_worker_source_changes() -> None:
    assert harness_changed(["console/src/main.rs"]) is True


def test_harness_runs_for_integration_infrastructure_changes() -> None:
    assert harness_changed([".github/workflows/_harness-integration.yml"]) is True
