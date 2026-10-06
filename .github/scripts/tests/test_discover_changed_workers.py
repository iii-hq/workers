from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest


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
    assert harness_changed(["ade/Cargo.toml", "ade/Cargo.lock"]) is False


def test_harness_runs_for_worker_source_changes() -> None:
    assert harness_changed(["ade/src/main.rs"]) is True


def test_harness_runs_for_integration_infrastructure_changes() -> None:
    assert harness_changed([".github/workflows/_harness-integration.yml"]) is True


def test_crate_dependents_follow_crates_that_depend_on_the_crate(tmp_path: Path) -> None:
    manifests = {
        "crates/native/Cargo.toml": "",
        "crates/runtime/Cargo.toml": 'native = { path = "../native" }',
        "direct/Cargo.toml": 'native = { path = "../crates/native" }',
        "through/Cargo.toml": 'runtime = { path = "../crates/runtime" }',
        "other/Cargo.toml": "",
    }
    for path, text in manifests.items():
        (tmp_path / path).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / path).write_text(text)
    workers = {"direct", "through", "other"}
    assert discover.crate_dependents(tmp_path, "native", workers) == ["direct", "through"]
    assert discover.crate_dependents(tmp_path, "runtime", workers) == ["through"]


def test_a_fixture_another_crate_loads_runs_that_crate(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(SCRIPTS.parents[1])
    monkeypatch.delenv("GITHUB_OUTPUT", raising=False)
    monkeypatch.setattr(
        discover, "changed_files", lambda base, head: ["judge-laya/tests/fixtures/tiny.reference.gguf"]
    )
    assert discover.main(["--base", "main"]) == 0
    assert "llama-native" in json.loads(capsys.readouterr().out)["crates"]
