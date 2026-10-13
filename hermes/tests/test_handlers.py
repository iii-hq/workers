from __future__ import annotations

from pathlib import Path

import pytest

from src.handlers import _extract_prompt
from src.main import load_config


def test_extract_prompt_prefers_prompt():
    assert _extract_prompt({"prompt": "hi"}) == "hi"


def test_extract_prompt_joins_last_user_message():
    payload = {
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "first"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "reply"}]},
            {"role": "user", "content": [{"type": "text", "text": "line one"}, {"type": "text", "text": "line two"}]},
        ]
    }
    assert _extract_prompt(payload) == "line one\nline two"


def test_extract_prompt_plain_string_content():
    assert _extract_prompt({"messages": [{"role": "user", "content": "plain"}]}) == "plain"


def test_extract_prompt_requires_a_user_message():
    with pytest.raises(ValueError):
        _extract_prompt({"messages": [{"role": "assistant", "content": "x"}]})


def test_load_config_defaults(monkeypatch, tmp_path):
    monkeypatch.setenv("HERMES_WORKER_CONFIG", str(tmp_path / "missing.yaml"))
    cfg = load_config()
    assert cfg["engine_url"] == "ws://127.0.0.1:49134"
    assert "events_stream" not in cfg and "raw_events_stream" not in cfg
    assert cfg["iii_context"] is True
    assert cfg["inbound_api_path"] == "/hermes/inbound"
    assert cfg["defaults"]["model"] == ""


def test_load_config_merges_partial(monkeypatch, tmp_path):
    path = tmp_path / "config.yaml"
    path.write_text("engine_url: ws://10.0.0.1:49134\niii_context: false\ndefaults:\n  model: anthropic/claude\n")
    monkeypatch.setenv("HERMES_WORKER_CONFIG", str(path))
    cfg = load_config()
    assert cfg["engine_url"] == "ws://10.0.0.1:49134"
    assert cfg["iii_context"] is False
    assert cfg["defaults"]["model"] == "anthropic/claude"
    assert cfg["defaults"]["cwd"] == ""


def test_load_config_tolerates_legacy_stream_keys(monkeypatch, tmp_path):
    # Configs written for the old iii-stream feeds still load; the stream names
    # are dropped (the feeds are now the fixed hermes::agent-event / raw-event).
    path = tmp_path / "config.yaml"
    path.write_text("events_stream: agent::events\nraw_events_stream: hermes::events\ninbound_api_path: /in\n")
    monkeypatch.setenv("HERMES_WORKER_CONFIG", str(path))
    cfg = load_config()
    assert cfg["inbound_api_path"] == "/in"
    assert "events_stream" not in cfg and "raw_events_stream" not in cfg


def test_shipped_config_yaml_loads_without_stream_keys(monkeypatch):
    path = Path(__file__).resolve().parent.parent / "config.yaml"
    monkeypatch.setenv("HERMES_WORKER_CONFIG", str(path))
    assert "stream" not in path.read_text()
    cfg = load_config()
    assert cfg["inbound_api_path"] == "/hermes/inbound"
