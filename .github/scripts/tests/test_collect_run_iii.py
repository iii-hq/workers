"""The engine CLI command collect_worker_interface.py runs for each read."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest

import collect_worker_interface as collector


def capture_command(monkeypatch):
    calls = []

    def fake_run(command, **kwargs):
        calls.append((command, kwargs))
        return SimpleNamespace(stdout=json.dumps({"workers": []}))

    monkeypatch.setattr(collector.subprocess, "run", fake_run)
    return calls


def test_reads_the_engine_named_by_its_port_with_engine(monkeypatch):
    monkeypatch.setenv("III_ENGINE_PORT", "55461")
    calls = capture_command(monkeypatch)

    assert collector.run_iii("engine::workers::list", {}) == {"workers": []}

    [(command, kwargs)] = calls
    assert command == [
        "iii",
        "call",
        "engine::workers::list",
        "--json",
        "{}",
        "--engine",
        "ws://127.0.0.1:55461",
    ]
    assert "--port" not in command
    assert kwargs["check"] is True and kwargs["timeout"] == 30


def test_without_a_port_the_cli_picks_its_engine(monkeypatch):
    monkeypatch.delenv("III_ENGINE_PORT", raising=False)
    calls = capture_command(monkeypatch)

    collector.run_iii("engine::functions::list", {"include_internal": True})

    [(command, _)] = calls
    assert command == [
        "iii",
        "call",
        "engine::functions::list",
        "--json",
        '{"include_internal": true}',
    ]


@pytest.mark.parametrize("port", ["0", "65536", "ws://x", "49134 --engine"])
def test_a_port_out_of_range_is_refused(monkeypatch, port):
    monkeypatch.setenv("III_ENGINE_PORT", port)
    capture_command(monkeypatch)

    with pytest.raises(ValueError, match="III_ENGINE_PORT"):
        collector.run_iii("engine::workers::list", {})
