from __future__ import annotations

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
    assert cfg["events_stream"] == "agent::events"
    assert cfg["raw_events_stream"] == "hermes::events"
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
