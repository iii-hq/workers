from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest

from src import handlers as handlers_mod
from src.handlers import _emit, create_handlers
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
    req = SimpleNamespace(body={"session_id": "chat-7", "text": "hello"})
    resp = asyncio.run(h["inbound"](req, FakeLogger()))
    assert resp.model_dump(by_alias=True)["statusCode"] == 200
    frames = fake.stream_frames("hermes::events")
    assert frames[0]["group_id"] == "chat-7"
    assert frames[0]["data"] == {"type": "inbound", "body": {"session_id": "chat-7", "text": "hello"}}


def test_inbound_derives_group_from_chat_id(monkeypatch):
    fake, _, h = _make(monkeypatch)
    req = SimpleNamespace(body={"chat_id": "tg-42", "text": "hi"})
    asyncio.run(h["inbound"](req, FakeLogger()))
    assert fake.stream_frames("hermes::events")[0]["group_id"] == "tg-42"


def test_inbound_synthesizes_group_when_missing(monkeypatch):
    fake, _, h = _make(monkeypatch)
    req = SimpleNamespace(body={"text": "hi"})
    asyncio.run(h["inbound"](req, FakeLogger()))
    gid = fake.stream_frames("hermes::events")[0]["group_id"]
    assert isinstance(gid, str) and len(gid) > 0


def test_inbound_handles_empty_body(monkeypatch):
    fake, _, h = _make(monkeypatch)
    req = SimpleNamespace(body=None)
    resp = asyncio.run(h["inbound"](req, FakeLogger()))
    assert resp.model_dump(by_alias=True)["statusCode"] == 200
    assert fake.stream_frames("hermes::events")[0]["data"]["body"] == {}


def test_emit_writes_stream_set_frame():
    fake = FakeIii()
    asyncio.run(_emit(fake, "agent::events", "s1", {"type": "agent_end"}))
    frames = fake.stream_frames("agent::events")
    assert frames[0]["group_id"] == "s1"
    assert frames[0]["data"] == {"type": "agent_end"}


def test_emit_item_ids_monotonic_per_session():
    fake = FakeIii()
    for _ in range(3):
        asyncio.run(_emit(fake, "agent::events", "s1", {"type": "x"}))
    ids = [f["item_id"] for f in fake.stream_frames("agent::events")]
    assert len(set(ids)) == 3 and ids == sorted(ids)


def test_create_handlers_exposes_full_surface(monkeypatch):
    _, _, h = _make(monkeypatch)
    assert set(h) == {"run", "start", "send", "sessions_list", "status", "stop", "inbound"}
