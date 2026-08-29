"""Tests for the Registry Compose smoke runner."""
from __future__ import annotations

from pathlib import Path
from typing import Any

import registry_worker_smoke


def add_catalog_worker(root: Path, worker: str) -> None:
    import yaml
    directory = root / worker
    directory.mkdir()
    (directory / "Cargo.toml").write_text(f'[package]\nname="{worker}"\nversion="1.0.0"\n')
    path = root / "worker-compose.yaml"
    catalog = yaml.safe_load(path.read_text()) if path.exists() else {"workers": {}, "stacks": {}}
    catalog["workers"][worker] = {
        "source": {"path": worker, "package_manifest": "Cargo.toml"},
        "artifact": {"kind": "rust-binary", "binary": worker, "targets": ["x86_64-unknown-linux-gnu"]},
        "runtime": {"exec": [worker]},
        "registry": {"description": worker, "license": "Apache-2.0", "tags": ["test"], "dependencies": {}, "publish": True},
        "validation": {"interface": "required"},
    }
    path.write_text(yaml.safe_dump(catalog, sort_keys=False))


def test_stable_workers_excludes_non_registry_workers_and_keeps_harness_next_last(
    tmp_path: Path,
    monkeypatch,
) -> None:
    for worker in ("browser", "harness", "acp", "lsp", "a2ui"):
        add_catalog_worker(tmp_path, worker)

    monkeypatch.setattr(registry_worker_smoke, "REPO_ROOT", tmp_path)

    assert registry_worker_smoke.stable_workers() == ["browser", "harness@next"]


def test_worker_key_accepts_registry_selectors() -> None:
    assert registry_worker_smoke.worker_key("harness@next") == "harness"
    assert registry_worker_smoke.worker_key("scope/path/shell@0.11.10") == "shell"


def test_ordered_workers_replaces_harness_selector_and_moves_it_last() -> None:
    assert registry_worker_smoke.ordered_workers(
        ["harness", "browser", "harness@1.8.1", "state"]
    ) == ["browser", "state", "harness@next"]


def test_worker_stops_isolated_project_when_initial_up_errors(monkeypatch) -> None:
    calls: list[str] = []

    def fake_trigger(
        _namespace: str,
        function_id: str,
        _payload: dict[str, Any],
    ) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
        calls.append(function_id)
        if function_id == "compose::up":
            return None, {"code": "CLI_TIMEOUT", "message": "timed out"}
        return {"status": "ok"}, None

    monkeypatch.setattr(registry_worker_smoke, "trigger", fake_trigger)
    monkeypatch.setattr(
        registry_worker_smoke,
        "compose_text",
        lambda namespace: f"namespace: {namespace}\ncontainers: {{}}\n",
    )

    assert registry_worker_smoke.test_worker("a", "browser") == {
        "worker": "browser",
        "status": "fail",
        "errors": [{"code": "CLI_TIMEOUT", "message": "timed out"}],
    }
    assert calls == ["compose::up", "compose::down"]


def test_worker_uses_add_result_and_always_stops_isolated_project(
    monkeypatch,
) -> None:
    calls: list[tuple[str, dict[str, Any]]] = []

    def fake_trigger(
        _namespace: str,
        function_id: str,
        payload: dict[str, Any],
    ) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
        calls.append((function_id, payload))
        if function_id == "compose::up":
            return {"status": "ok", "containers": []}, None
        if function_id == "compose::add":
            return {
                "status": "failed",
                "up": {
                    "containers": [
                        {
                            "container": "harness",
                            "state": "failed",
                            "error": {
                                "code": "PACKAGE_NOT_RESOLVED",
                                "message": "not found",
                            },
                        }
                    ]
                },
            }, None
        return {"status": "ok"}, None

    monkeypatch.setattr(registry_worker_smoke, "trigger", fake_trigger)
    monkeypatch.setattr(
        registry_worker_smoke,
        "compose_text",
        lambda namespace: f"namespace: {namespace}\ncontainers: {{}}\n",
    )

    result = registry_worker_smoke.test_worker("a", "harness@next")

    assert result == {
        "worker": "harness@next",
        "status": "fail",
        "errors": [
            {
                "container": "harness",
                "code": "PACKAGE_NOT_RESOLVED",
                "message": "not found",
            }
        ],
    }
    assert [function_id for function_id, _payload in calls] == [
        "compose::up",
        "compose::add",
        "compose::down",
    ]
    assert calls[1][1]["worker"] == "harness@next"
    assert calls[0][1]["file"] == calls[1][1]["file"] == calls[2][1]["file"]


def test_worker_reports_ready_add_result_as_pass(monkeypatch) -> None:
    def fake_trigger(
        _namespace: str,
        function_id: str,
        _payload: dict[str, Any],
    ) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
        if function_id == "compose::add":
            return {
                "status": "ok",
                "up": {
                    "containers": [
                        {"container": "browser", "state": "ready"},
                    ]
                },
            }, None
        return {"status": "ok", "containers": []}, None

    monkeypatch.setattr(registry_worker_smoke, "trigger", fake_trigger)
    monkeypatch.setattr(
        registry_worker_smoke,
        "compose_text",
        lambda namespace: f"namespace: {namespace}\ncontainers: {{}}\n",
    )

    assert registry_worker_smoke.test_worker("a", "browser")["status"] == "pass"
