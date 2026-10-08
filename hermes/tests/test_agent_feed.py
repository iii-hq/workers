"""Contract tests for the owned feeds `hermes::agent-event` / `hermes::raw-event`.

Cases (P3-1 producer brief, section 6): a delivery shape, b session filter,
c unbind, d config validation + binding cap, e sequencing, f failed delivery
swallowed. Case g (run frames via the feed) lives in test_run.py.
"""

from __future__ import annotations

import asyncio
import re

import pytest
from iii import TriggerActionVoid

from src import handlers as handlers_mod
from src.agent_feed import (
    AGENT_EVENT_TYPE,
    MAX_BINDINGS,
    RAW_EVENT_TYPE,
    create_feeds,
    validate_feed_config,
)
from src.handlers import create_handlers
from tests._helpers.fake_iii import FakeIii, FakeLogger, base_cfg, consumer_fn

FEEDS = [AGENT_EVENT_TYPE, RAW_EVENT_TYPE]


def _feed(fake: FakeIii, type_id: str):
    feeds = create_feeds(fake)
    return feeds.agent if type_id == AGENT_EVENT_TYPE else feeds.raw


def test_create_feeds_registers_both_trigger_types():
    fake = FakeIii()
    feeds = create_feeds(fake)
    assert set(fake.trigger_types) == {AGENT_EVENT_TYPE, RAW_EVENT_TYPE}
    assert fake.trigger_types[AGENT_EVENT_TYPE]["handler"] is feeds.agent
    assert fake.trigger_types[RAW_EVENT_TYPE]["handler"] is feeds.raw
    info = fake.trigger_types[AGENT_EVENT_TYPE]["info"]
    assert info["trigger_request_format"]["required"] == ["session_id"]
    assert info["description"]


# a. delivered to a bound consumer: payload shape, function_id, namespace, metadata, Void
@pytest.mark.parametrize("type_id", FEEDS)
def test_a_delivery_payload_and_routing(type_id):
    fake = FakeIii()
    feed = _feed(fake, type_id)
    fake.bind(
        type_id,
        config={"session_id": "s1", "metadata": {"conn": "c1"}},
        function_id="acp::__on_event::c1",
        namespace="tenant-a",
        metadata={"binding": True},
    )
    delivered = asyncio.run(feed.emit("s1", {"type": "agent_end", "messages": []}))
    assert delivered == 1
    [call] = fake.void_calls()
    assert call["function_id"] == "acp::__on_event::c1"
    assert call["namespace"] == "tenant-a"
    assert call["metadata"] == {"conn": "c1"}  # config metadata wins over binding metadata
    assert isinstance(call["action"], TriggerActionVoid)
    assert call["payload"] == {
        "session_id": "s1",
        "event_id": f"s1-{feed.epoch}-00000000",
        "seq": 0,
        "epoch": feed.epoch,
        "source": "hermes",
        "event": {"type": "agent_end", "messages": []},
    }


def test_a_binding_metadata_used_when_config_has_none_and_omitted_when_both_absent():
    fake = FakeIii()
    feed = _feed(fake, AGENT_EVENT_TYPE)
    fake.bind(AGENT_EVENT_TYPE, "s1", function_id="f::with", metadata={"m": 1})
    fake.bind(AGENT_EVENT_TYPE, "s1", function_id="f::without")
    asyncio.run(feed.emit("s1", {"type": "x"}))
    with_meta, without_meta = fake.void_calls()
    assert with_meta["function_id"] == "f::with" and with_meta["metadata"] == {"m": 1}
    assert without_meta["function_id"] == "f::without"
    assert "metadata" not in without_meta and "namespace" not in without_meta


# b. filter by session
@pytest.mark.parametrize("type_id", FEEDS)
def test_b_binding_only_receives_its_session(type_id):
    fake = FakeIii()
    feed = _feed(fake, type_id)
    fake.bind(type_id, "A", function_id="f::a")
    fake.bind(type_id, "B", function_id="f::b")
    asyncio.run(feed.emit("B", {"type": "x"}))
    asyncio.run(feed.emit("C", {"type": "x"}))
    assert fake.void_calls("f::a") == []
    assert [c["payload"]["session_id"] for c in fake.void_calls("f::b")] == ["B"]
    assert len(fake.calls) == 1  # session C has no binding: no call at all


# c. no delivery after unbind
@pytest.mark.parametrize("type_id", FEEDS)
def test_c_no_delivery_after_unbind(type_id):
    fake = FakeIii()
    feed = _feed(fake, type_id)
    bid = fake.bind(type_id, "s1")
    asyncio.run(feed.emit("s1", {"type": "x"}))
    fake.unbind(type_id, bid)
    assert feed.binding_count() == 0
    assert asyncio.run(feed.emit("s1", {"type": "y"})) == 0
    assert [p["event"]["type"] for p in fake.feed_frames(type_id)] == ["x"]


# d. invalid configs rejected + binding cap
@pytest.mark.parametrize(
    "config",
    [
        None,
        "s1",
        {},
        {"session_id": ""},
        {"session_id": 7},
        {"session_id": "x" * 513},
        {"session_id": "s1", "group_id": "s1"},
        {"session_id": "s1", "metadata": "nope"},
        {"session_id": "s1", "metadata": [1]},
    ],
)
@pytest.mark.parametrize("type_id", FEEDS)
def test_d_invalid_config_rejects_binding(type_id, config):
    fake = FakeIii()
    feed = _feed(fake, type_id)
    with pytest.raises(ValueError, match=re.escape(type_id)):
        fake.bind(type_id, config=config)
    assert feed.binding_count() == 0


def test_d_valid_config_shapes():
    assert validate_feed_config("t", {"session_id": "x" * 512}) == {"session_id": "x" * 512}
    assert validate_feed_config("t", {"session_id": "s", "metadata": {"k": 1}}) == {
        "session_id": "s",
        "metadata": {"k": 1},
    }


def test_d_binding_cap_rejects_new_ids_but_allows_replacement():
    fake = FakeIii()
    feed = _feed(fake, AGENT_EVENT_TYPE)
    for i in range(MAX_BINDINGS):
        fake.bind(AGENT_EVENT_TYPE, f"s{i}", binding_id=f"b{i}")
    assert feed.binding_count() == MAX_BINDINGS
    with pytest.raises(ValueError, match="binding limit"):
        fake.bind(AGENT_EVENT_TYPE, "extra", binding_id="b-new")
    # re-registering an existing id replaces it, even at the cap
    fake.bind(AGENT_EVENT_TYPE, "moved", binding_id="b0", function_id="f::moved")
    assert feed.binding_count() == MAX_BINDINGS
    asyncio.run(feed.emit("s0", {"type": "x"}))
    asyncio.run(feed.emit("moved", {"type": "x"}))
    assert [c["function_id"] for c in fake.void_calls()] == ["f::moved"]
    # the raw feed keeps its own table
    assert feeds_raw_count(fake) == 0


def feeds_raw_count(fake: FakeIii) -> int:
    return fake.trigger_types[RAW_EVENT_TYPE]["handler"].binding_count()


# e. seq contiguous from 0 per (feed, session); agent and raw counters independent
def test_e_sequence_per_feed_and_session():
    fake = FakeIii()
    feeds = create_feeds(fake)
    for type_id in FEEDS:
        fake.bind(type_id, "s1")
        fake.bind(type_id, "s2", function_id=f"f::s2::{type_id}")

    async def scenario():
        await feeds.raw.emit("s1", {"type": "r"})
        await feeds.agent.emit("s1", {"type": "a"})
        await feeds.agent.emit("s2", {"type": "a"})
        await feeds.agent.emit("s1", {"type": "a"})
        await feeds.raw.emit("s1", {"type": "r"})
        await feeds.agent.emit("s1", {"type": "a"})

    asyncio.run(scenario())
    agent = fake.feed_frames(AGENT_EVENT_TYPE)
    raw = fake.feed_frames(RAW_EVENT_TYPE)
    assert [f["seq"] for f in agent] == [0, 1, 2]
    assert [f["seq"] for f in raw] == [0, 1]
    assert [f["seq"] for f in fake.feed_frames(AGENT_EVENT_TYPE, f"f::s2::{AGENT_EVENT_TYPE}")] == [0]
    pattern = re.compile(r"^s1-[0-9a-f-]{36}-\d{8}$")
    for frame in agent + raw:
        assert pattern.match(frame["event_id"]), frame["event_id"]
        assert frame["event_id"] == f"s1-{frame['epoch']}-{frame['seq']:08d}"
    assert agent[2]["event_id"].endswith("-00000002")
    assert {f["epoch"] for f in agent} == {feeds.agent.epoch}


def test_e_seq_counts_frames_emitted_before_a_late_bind():
    # Ephemeral: a consumer bound late sees the current seq, not a restart at 0.
    fake = FakeIii()
    feed = _feed(fake, AGENT_EVENT_TYPE)
    asyncio.run(feed.emit("s1", {"type": "early"}))
    fake.bind(AGENT_EVENT_TYPE, "s1")
    asyncio.run(feed.emit("s1", {"type": "late"}))
    assert [(f["seq"], f["event"]["type"]) for f in fake.feed_frames(AGENT_EVENT_TYPE)] == [(1, "late")]


# f. a failing delivery is swallowed and the turn still succeeds
def test_f_failing_delivery_is_swallowed():
    fake = FakeIii()
    feed = _feed(fake, AGENT_EVENT_TYPE)
    fake.bind(AGENT_EVENT_TYPE, "s1", function_id="f::gone")
    fake.bind(AGENT_EVENT_TYPE, "s1", function_id="f::ok")
    fake.failing_functions.add("f::gone")
    assert asyncio.run(feed.emit("s1", {"type": "x"})) == 1
    assert len(fake.void_calls("f::gone")) == 1 and len(fake.void_calls("f::ok")) == 1


def test_f_turn_succeeds_when_consumer_is_gone(monkeypatch):
    fake = FakeIii()

    async def run_turn(*_a, **_kw):
        return "pong", ""

    monkeypatch.setattr(handlers_mod.hermes_cli, "run_turn", run_turn)
    monkeypatch.setattr(handlers_mod.hermes_cli, "read_latest_usage", lambda: None)
    h = create_handlers(fake, lambda: base_cfg(), FakeLogger())
    for type_id in FEEDS:
        fake.bind(type_id, "s1")
        fake.failing_functions.add(consumer_fn(type_id))
    res = asyncio.run(h["run"]({"prompt": "ping", "session_id": "s1"}))
    assert res == {"session_id": "s1", "result": "pong", "is_error": False, "stop_reason": "end"}
    assert fake.state["hermes_sessions/s1"]["status"] == "done"
    assert len(fake.void_calls()) == 3  # result, turn_end, agent_end were all attempted
