"""Contract tests for the owned feeds `hermes::on-agent-event` / `hermes::on-raw-event`.

Cases (P3-1 producer brief, section 6): a event shape and Void call, b session
matching, c no delivery after the observer is removed, d args format and
observer cap, e sequencing, f failed call swallowed. Case g (run frames via the
feed) lives in test_run.py.
"""

from __future__ import annotations

import asyncio
import re

import pytest
from iii import CallActionVoid, ObserveRejection

from src import handlers as handlers_mod
from src.agent_feed import (
    FEED_ARGS_FORMAT,
    FEED_PAYLOAD_FORMAT,
    MAX_OBSERVERS,
    MAX_SESSION_ID_LEN,
    OBSERVER_LIMIT,
    ON_AGENT_EVENT_FN,
    ON_RAW_EVENT_FN,
    create_feeds,
)
from src.handlers import create_handlers
from tests._helpers.fake_iii import FakeIii, FakeLogger, base_cfg

FEEDS = [ON_AGENT_EVENT_FN, ON_RAW_EVENT_FN]


def _feed(fake: FakeIii, function_id: str):
    feeds = create_feeds(fake)
    return feeds.agent if function_id == ON_AGENT_EVENT_FN else feeds.raw


def test_create_feeds_registers_both_private_functions_with_hooks():
    fake = FakeIii()
    create_feeds(fake)
    assert set(fake.registered) == {ON_AGENT_EVENT_FN, ON_RAW_EVENT_FN}
    for fid in FEEDS:
        registration = fake.registrations[fid]
        assert registration["private"] is True
        assert registration["args_format"] == FEED_ARGS_FORMAT
        assert registration["request_format"] == FEED_PAYLOAD_FORMAT
        assert registration["response_format"] == FEED_PAYLOAD_FORMAT
        assert registration["description"]
        assert fid in fake.added_hooks and fid in fake.removed_hooks and fid in fake.match_keys


@pytest.mark.parametrize("function_id", FEEDS)
def test_the_handler_returns_the_event_it_is_called_with(function_id):
    fake = FakeIii()
    create_feeds(fake)
    event = {"session_id": "s1", "seq": 0, "event": {"type": "x"}}
    assert asyncio.run(fake.registered[function_id](event)) is event


# a. one Void call of the feed function; the observer's target gets the event
@pytest.mark.parametrize("function_id", FEEDS)
def test_a_event_shape_and_void_call(function_id):
    fake = FakeIii()
    feed = _feed(fake, function_id)
    fake.observe_feed(function_id, "s1", target="acp::__on_event::c1")
    assert asyncio.run(feed.emit("s1", {"type": "agent_end", "messages": []})) is True
    [call] = fake.void_calls()
    assert call["function_id"] == function_id
    assert isinstance(call["action"], CallActionVoid)
    assert "namespace" not in call and "metadata" not in call
    expected = {
        "session_id": "s1",
        "event_id": f"s1-{feed.epoch}-00000000",
        "seq": 0,
        "epoch": feed.epoch,
        "source": "hermes",
        "event": {"type": "agent_end", "messages": []},
    }
    assert call["payload"] == expected
    assert fake.feed_frames(function_id, "acp::__on_event::c1") == [expected]


# b. an observer receives only its session's frames
@pytest.mark.parametrize("function_id", FEEDS)
def test_b_observer_only_receives_its_session(function_id):
    fake = FakeIii()
    feed = _feed(fake, function_id)
    fake.observe_feed(function_id, "A", target="f::a")
    fake.observe_feed(function_id, "B", target="f::b")
    asyncio.run(feed.emit("B", {"type": "x"}))
    assert asyncio.run(feed.emit("C", {"type": "x"})) is False
    assert fake.feed_frames(function_id, "f::a") == []
    assert [p["session_id"] for p in fake.feed_frames(function_id, "f::b")] == ["B"]
    assert len(fake.calls) == 1  # session C has no observer: no call at all


def test_b_match_keys_read_session_id():
    fake = FakeIii()
    create_feeds(fake)
    observer_keys, call_keys = fake.match_keys[ON_AGENT_EVENT_FN]

    class Obs:
        args = {"session_id": "s1"}

    class Call:
        input = {"session_id": "s1", "seq": 3}

    assert tuple(observer_keys(Obs())) == ("s1",)
    assert tuple(call_keys(Call())) == ("s1",)
    Obs.args = {}
    Call.input = None
    assert tuple(observer_keys(Obs())) == ()
    assert tuple(call_keys(Call())) == ()


# c. no delivery after the observer is removed
@pytest.mark.parametrize("function_id", FEEDS)
def test_c_no_delivery_after_the_observer_is_removed(function_id):
    fake = FakeIii()
    feed = _feed(fake, function_id)
    oid = fake.observe_feed(function_id, "s1")
    asyncio.run(feed.emit("s1", {"type": "x"}))
    fake.unobserve(function_id, oid)
    assert feed.observer_count() == 0
    assert asyncio.run(feed.emit("s1", {"type": "y"})) is False
    assert [p["event"]["type"] for p in fake.feed_frames(function_id)] == ["x"]


# d. args format and the observer cap
def test_d_args_format_requires_a_bounded_session_id_and_nothing_else():
    assert FEED_ARGS_FORMAT["required"] == ["session_id"]
    assert FEED_ARGS_FORMAT["additionalProperties"] is False
    session_id = FEED_ARGS_FORMAT["properties"]["session_id"]
    assert session_id["type"] == "string"
    assert session_id["minLength"] == 1
    assert session_id["maxLength"] == MAX_SESSION_ID_LEN == 512
    assert set(FEED_ARGS_FORMAT["properties"]) == {"session_id"}


def test_d_observer_cap_rejects_new_ids_and_frees_on_removal():
    fake = FakeIii()
    feed = _feed(fake, ON_AGENT_EVENT_FN)
    for i in range(MAX_OBSERVERS):
        fake.observe_feed(ON_AGENT_EVENT_FN, f"s{i}", observer_id=f"o{i}")
    assert feed.observer_count() == MAX_OBSERVERS
    with pytest.raises(ObserveRejection) as rejected:
        fake.observe_feed(ON_AGENT_EVENT_FN, "extra", observer_id="o-new")
    assert rejected.value.code == OBSERVER_LIMIT
    assert "observer limit (256)" in str(rejected.value)
    # the engine re-announcing a counted observer is accepted, even at the cap
    fake.observe_feed(ON_AGENT_EVENT_FN, "moved", observer_id="o0", target="f::moved")
    assert feed.observer_count() == MAX_OBSERVERS
    asyncio.run(feed.emit("moved", {"type": "x"}))
    assert [p["session_id"] for p in fake.feed_frames(ON_AGENT_EVENT_FN, "f::moved")] == ["moved"]
    # a removal frees a slot; the raw feed keeps its own count
    fake.unobserve(ON_AGENT_EVENT_FN, "o1")
    fake.observe_feed(ON_AGENT_EVENT_FN, "extra", observer_id="o-new")
    assert feed.observer_count() == MAX_OBSERVERS
    assert create_feeds(FakeIii()).raw.observer_count() == 0


# e. seq contiguous from 0 per (feed, session); agent and raw counters independent
def test_e_sequence_per_feed_and_session():
    fake = FakeIii()
    feeds = create_feeds(fake)
    for fid in FEEDS:
        fake.observe_feed(fid, "s1")
        fake.observe_feed(fid, "s2", target=f"f::s2::{fid}")

    async def scenario():
        await feeds.raw.emit("s1", {"type": "r"})
        await feeds.agent.emit("s1", {"type": "a"})
        await feeds.agent.emit("s2", {"type": "a"})
        await feeds.agent.emit("s1", {"type": "a"})
        await feeds.raw.emit("s1", {"type": "r"})
        await feeds.agent.emit("s1", {"type": "a"})

    asyncio.run(scenario())
    agent = fake.feed_frames(ON_AGENT_EVENT_FN)
    raw = fake.feed_frames(ON_RAW_EVENT_FN)
    assert [f["seq"] for f in agent] == [0, 1, 2]
    assert [f["seq"] for f in raw] == [0, 1]
    assert [f["seq"] for f in fake.feed_frames(ON_AGENT_EVENT_FN, f"f::s2::{ON_AGENT_EVENT_FN}")] == [0]
    pattern = re.compile(r"^s1-[0-9a-f-]{36}-\d{8}$")
    for frame in agent + raw:
        assert pattern.match(frame["event_id"]), frame["event_id"]
        assert frame["event_id"] == f"s1-{frame['epoch']}-{frame['seq']:08d}"
    assert agent[2]["event_id"].endswith("-00000002")
    assert {f["epoch"] for f in agent} == {feeds.agent.epoch}


def test_e_seq_counts_frames_emitted_before_a_late_observer():
    # Ephemeral: an observer added late sees the current seq, not a restart at 0.
    fake = FakeIii()
    feed = _feed(fake, ON_AGENT_EVENT_FN)
    asyncio.run(feed.emit("s1", {"type": "early"}))
    fake.observe_feed(ON_AGENT_EVENT_FN, "s1")
    asyncio.run(feed.emit("s1", {"type": "late"}))
    assert [(f["seq"], f["event"]["type"]) for f in fake.feed_frames(ON_AGENT_EVENT_FN)] == [(1, "late")]


# f. a failing call is swallowed and the turn still succeeds
def test_f_failing_call_is_swallowed():
    fake = FakeIii()
    feed = _feed(fake, ON_AGENT_EVENT_FN)
    fake.observe_feed(ON_AGENT_EVENT_FN, "s1")
    fake.failing_functions.add(ON_AGENT_EVENT_FN)
    assert asyncio.run(feed.emit("s1", {"type": "x"})) is False
    fake.failing_functions.clear()
    assert asyncio.run(feed.emit("s1", {"type": "y"})) is True
    assert [f["seq"] for f in fake.feed_frames(ON_AGENT_EVENT_FN)] == [1]


def test_f_turn_succeeds_when_the_feed_calls_fail(monkeypatch):
    fake = FakeIii()

    async def run_turn(*_a, **_kw):
        return "pong", ""

    monkeypatch.setattr(handlers_mod.hermes_cli, "run_turn", run_turn)
    monkeypatch.setattr(handlers_mod.hermes_cli, "read_latest_usage", lambda: None)
    h = create_handlers(fake, lambda: base_cfg(), FakeLogger())
    for fid in FEEDS:
        fake.observe_feed(fid, "s1")
        fake.failing_functions.add(fid)
    res = asyncio.run(h["run"]({"prompt": "ping", "session_id": "s1"}))
    assert res == {"session_id": "s1", "result": "pong", "is_error": False, "stop_reason": "end"}
    assert fake.state["hermes_sessions/s1"]["status"] == "done"
    assert len(fake.void_calls()) == 3  # result, turn_end, agent_end were all attempted
