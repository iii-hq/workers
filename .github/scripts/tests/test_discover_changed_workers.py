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


def test_shared_test_support_runs_the_workers_that_compile_it(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(SCRIPTS.parents[1])
    monkeypatch.delenv("GITHUB_OUTPUT", raising=False)
    monkeypatch.setattr(
        discover, "changed_files", lambda base, head: ["judge-typesafe/tests/support/fake_engine.rs"]
    )
    assert discover.main(["--base", "main"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert {"judge", "judge-openai", "judge-typesafe"} <= set(payload["by_language"]["rust"])
    assert "judge-provider" in payload["crates"]
    assert payload["source_changed"] == ["judge-typesafe"]


def test_package_dependents_find_workers_whose_sub_projects_depend_on_the_package(tmp_path: Path) -> None:
    files = {
        "packages/native-ui/package.json": '{"name": "@iii-dev/native-ui"}',
        "app/native-ui/package.json": '{"dependencies": {"@iii-dev/native-ui": "workspace:*"}}',
        "top/package.json": '{"devDependencies": {"@iii-dev/native-ui": "workspace:*"}}',
        "console/ui/package.json": '{"dependencies": {"@iii-dev/console-ui": "workspace:*"}}',
        "plain/Cargo.toml": "",
    }
    for path, text in files.items():
        (tmp_path / path).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / path).write_text(text)
    workers = {"app", "top", "console", "plain"}
    assert discover.package_dependents(tmp_path, "native-ui", workers) == ["app", "top"]
    assert discover.package_dependents(tmp_path, "missing", workers) == []


def test_native_ui_job_runs_for_the_sdk_the_apps_the_crate_and_the_workspace() -> None:
    for path in (
        "packages/native-ui/src/list.ts",
        "browser/native-ui/src/app.tsx",
        "crates/native-ui/src/lib.rs",
        "pnpm-lock.yaml",
        "pnpm-workspace.yaml",
        ".github/workflows/ci.yml",
    ):
        assert discover.native_ui_changed([path]) is True, path
    for path in ("browser/src/main.rs", "browser/ui/page.tsx", "packages/console-ui/index.d.ts", "README.md"):
        assert discover.native_ui_changed([path]) is False, path


def test_an_sdk_change_rebuilds_the_workers_that_bundle_it(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(SCRIPTS.parents[1])
    monkeypatch.delenv("GITHUB_OUTPUT", raising=False)
    monkeypatch.setattr(discover, "changed_files", lambda base, head: ["packages/native-ui/src/list.ts"])
    assert discover.main(["--base", "main"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert "browser" in payload["by_language"]["rust"]
    assert payload["source_changed"] == []
    assert payload["native_ui"] is True


def test_sdk_docs_alone_rebuild_no_worker(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(SCRIPTS.parents[1])
    monkeypatch.delenv("GITHUB_OUTPUT", raising=False)
    monkeypatch.setattr(discover, "changed_files", lambda base, head: ["packages/native-ui/README.md"])
    assert discover.main(["--base", "main"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["changed_workers"] == []
