from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest

from src import handlers as handlers_mod
from src.agent_feed import AGENT_EVENT_TYPE, RAW_EVENT_TYPE
from src.handlers import create_handlers
from tests._helpers.fake_iii import FakeIii, FakeLogger, base_cfg


def _make(monkeypatch, *, send_result="ok", sessions_raw="(none)"):
    fake = FakeIii()
    logger = FakeLogger()

    async def fake_send(hermes, platform, message):
        fake._last_send = (hermes, platform, message)
        return send_result

    async def fake_sessions(hermes):
        return sessions_raw

    monkeypatch.setattr(handlers_mod.hermes_cli, "send", fake_send)
    monkeypatch.setattr(handlers_mod.hermes_cli, "sessions_list", fake_sessions)
    h = create_handlers(fake, lambda: base_cfg(), logger)
    return fake, logger, h


def test_send_delivers_to_platform(monkeypatch):
    fake, _, h = _make(monkeypatch, send_result="sent#1")
    res = asyncio.run(h["send"]({"platform": "telegram", "message": "hi"}))
    assert res == {"platform": "telegram", "sent": True, "detail": "sent#1"}
    assert fake._last_send[1:] == ("telegram", "hi")


def test_send_requires_platform_and_message(monkeypatch):
    _, _, h = _make(monkeypatch)
    with pytest.raises(ValueError):
        asyncio.run(h["send"]({"platform": "telegram"}))
    with pytest.raises(ValueError):
        asyncio.run(h["send"]({"message": "hi"}))


def test_send_uses_configured_executable(monkeypatch):
    fake = FakeIii()
    captured = {}

    async def fake_send(hermes, platform, message):
        captured["hermes"] = hermes
        return "ok"

    monkeypatch.setattr(handlers_mod.hermes_cli, "send", fake_send)
    h = create_handlers(fake, lambda: base_cfg(hermes_executable="/opt/hermes"), FakeLogger())
    asyncio.run(h["send"]({"platform": "discord", "message": "x"}))
    assert captured["hermes"] == "/opt/hermes"


def test_sessions_list_merges_state_and_raw(monkeypatch):
    fake, _, h = _make(monkeypatch, sessions_raw="raw-text")
    fake.state["hermes_sessions/s1"] = {"session_id": "s1"}
    fake.state["hermes_sessions/s2"] = {"session_id": "s2"}
    res = asyncio.run(h["sessions_list"]({}))
    assert sorted(s["session_id"] for s in res["sessions"]) == ["s1", "s2"]
    assert res["hermes_sessions_raw"] == "raw-text"


def test_sessions_list_empty(monkeypatch):
    _, _, h = _make(monkeypatch)
    res = asyncio.run(h["sessions_list"]({}))
    assert res["sessions"] == []


def test_status_reports_stored_record_and_not_live(monkeypatch):
    fake, _, h = _make(monkeypatch)
    fake.state["hermes_sessions/s1"] = {"session_id": "s1", "status": "done"}
    res = asyncio.run(h["status"]({"session_id": "s1"}))
    assert res == {"session_id": "s1", "live": False, "record": {"session_id": "s1", "status": "done"}}


def test_status_unknown_session(monkeypatch):
    _, _, h = _make(monkeypatch)
    res = asyncio.run(h["status"]({"session_id": "none"}))
    assert res == {"session_id": "none", "live": False, "record": None}


def test_stop_reports_not_interruptible(monkeypatch):
    _, _, h = _make(monkeypatch)
    res = asyncio.run(h["stop"]({"session_id": "s1"}))
    assert res["session_id"] == "s1"
    assert res["stopped"] is False
    assert "interruptible" in res["reason"]


def test_inbound_republishes_delivery(monkeypatch):
    fake, _, h = _make(monkeypatch)
    fake.bind(RAW_EVENT_TYPE, "chat-7")
    req = SimpleNamespace(body={"session_id": "chat-7", "text": "hello"})
    resp = asyncio.run(h["inbound"](req, FakeLogger()))
    assert resp.model_dump(by_alias=True)["statusCode"] == 200
    frames = fake.feed_frames(RAW_EVENT_TYPE)
    assert frames[0]["session_id"] == "chat-7"
    assert frames[0]["seq"] == 0
    assert frames[0]["event"] == {"type": "inbound", "body": {"session_id": "chat-7", "text": "hello"}}
    # inbound deliveries never touch the normalized feed
    assert fake.feed_frames(AGENT_EVENT_TYPE) == []


def test_inbound_derives_session_from_chat_id(monkeypatch):
    fake, _, h = _make(monkeypatch)
    fake.bind(RAW_EVENT_TYPE, "tg-42")
    req = SimpleNamespace(body={"chat_id": "tg-42", "text": "hi"})
    asyncio.run(h["inbound"](req, FakeLogger()))
    assert fake.feed_frames(RAW_EVENT_TYPE)[0]["session_id"] == "tg-42"


def test_inbound_synthesizes_session_when_missing(monkeypatch):
    fake, _, h = _make(monkeypatch)
    monkeypatch.setattr(handlers_mod.uuid, "uuid4", lambda: "synth-1")
    fake.bind(RAW_EVENT_TYPE, "synth-1")
    req = SimpleNamespace(body={"text": "hi"})
    asyncio.run(h["inbound"](req, FakeLogger()))
    assert fake.feed_frames(RAW_EVENT_TYPE)[0]["session_id"] == "synth-1"


def test_inbound_handles_empty_body(monkeypatch):
    fake, _, h = _make(monkeypatch)
    monkeypatch.setattr(handlers_mod.uuid, "uuid4", lambda: "synth-2")
    fake.bind(RAW_EVENT_TYPE, "synth-2")
    req = SimpleNamespace(body=None)
    resp = asyncio.run(h["inbound"](req, FakeLogger()))
    assert resp.model_dump(by_alias=True)["statusCode"] == 200
    assert fake.feed_events(RAW_EVENT_TYPE)[0]["body"] == {}


def test_inbound_without_binding_makes_no_call(monkeypatch):
    fake, _, h = _make(monkeypatch)
    asyncio.run(h["inbound"](SimpleNamespace(body={"session_id": "chat-7"}), FakeLogger()))
    assert fake.calls == []


def test_create_handlers_registers_both_feeds(monkeypatch):
    fake, _, _ = _make(monkeypatch)
    assert set(fake.trigger_types) == {AGENT_EVENT_TYPE, RAW_EVENT_TYPE}


def test_create_handlers_exposes_full_surface(monkeypatch):
    _, _, h = _make(monkeypatch)
    assert set(h) == {"run", "start", "send", "sessions_list", "status", "stop", "inbound"}
