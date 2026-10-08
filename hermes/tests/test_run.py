from __future__ import annotations

import asyncio

import pytest

from src import handlers as handlers_mod
from src.agent_feed import AGENT_EVENT_TYPE, RAW_EVENT_TYPE
from src.handlers import create_handlers
from src.iii_prompt import III_CONTEXT_PROMPT
from tests._helpers.fake_iii import FakeIii, FakeLogger, base_cfg, consumer_fn


def _scripted_run_turn(result: str = "done", *, raises: Exception | None = None):
    captured: dict[str, object] = {}

    async def fake(hermes: str, prompt: str, *, cwd: str = "", model: str = "", session: str = "", toolsets: str = ""):
        captured["hermes"] = hermes
        captured["prompt"] = prompt
        captured["cwd"] = cwd
        captured["model"] = model
        captured["session"] = session
        captured["toolsets"] = toolsets
        if raises is not None:
            raise raises
        return result, ""

    return fake, captured


def _make(monkeypatch, cfg=None, *, result="done", raises=None, usage=None, bind=("s1",)):
    cfg = cfg or base_cfg()
    fake = FakeIii()
    logger = FakeLogger()
    fn, captured = _scripted_run_turn(result, raises=raises)
    monkeypatch.setattr(handlers_mod.hermes_cli, "run_turn", fn)
    # Default: no usage row, so the envelope stays minimal and hermetic (the
    # real reader would hit ~/.hermes/state.db). Pass `usage` to exercise it.
    monkeypatch.setattr(handlers_mod.hermes_cli, "read_latest_usage", lambda: usage)
    h = create_handlers(fake, lambda: cfg, logger)
    # A consumer bound on both feeds for each session the test drives.
    for session_id in bind:
        fake.bind(AGENT_EVENT_TYPE, session_id)
        fake.bind(RAW_EVENT_TYPE, session_id)
    return fake, logger, h, captured


def test_run_returns_result_envelope(monkeypatch):
    fake, _, h, _ = _make(monkeypatch, result="pong")
    res = asyncio.run(h["run"]({"prompt": "ping", "session_id": "s1"}))
    assert res == {"session_id": "s1", "result": "pong", "is_error": False, "stop_reason": "end"}


def test_run_surfaces_usage_and_cost(monkeypatch):
    usage = {
        "usage": {
            "input_tokens": 6,
            "output_tokens": 2245,
            "cache_read_tokens": 16915,
            "cache_write_tokens": 22332,
            "reasoning_tokens": 0,
        },
        "total_cost_usd": 0.1225,
        "cost_source": "official_docs_snapshot",
    }
    fake, _, h, _ = _make(monkeypatch, usage=usage)
    res = asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert res["usage"]["output_tokens"] == 2245
    assert res["total_cost_usd"] == 0.1225
    assert res["cost_source"] == "official_docs_snapshot"
    # also on the session record and the agent_end frame
    assert fake.state["hermes_sessions/s1"]["total_cost_usd"] == 0.1225
    end = [e for e in fake.feed_events(AGENT_EVENT_TYPE) if e["type"] == "agent_end"][0]
    assert end["usage"]["output_tokens"] == 2245 and end["total_cost_usd"] == 0.1225


def test_run_omits_usage_when_unavailable(monkeypatch):
    fake, _, h, _ = _make(monkeypatch)  # usage None
    res = asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert "usage" not in res and "total_cost_usd" not in res


def test_run_persists_session_record_done(monkeypatch):
    fake, _, h, _ = _make(monkeypatch)
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    record = fake.state.get("hermes_sessions/s1")
    assert record["status"] == "done"
    assert record["turns"] == 1
    sets = [c for c in fake.calls if c["function_id"] == "state::set"]
    assert sets and sets[-1]["payload"]["scope"] == "hermes_sessions"


def test_run_emits_translated_agent_events(monkeypatch):
    fake, _, h, _ = _make(monkeypatch, result="hi")
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    events = fake.feed_events(AGENT_EVENT_TYPE)
    assert [e["type"] for e in events] == ["turn_end", "agent_end"]
    message = {"role": "assistant", "content": [{"type": "text", "text": "hi"}], "provider": "hermes"}
    assert events[0] == {"type": "turn_end", "message": message, "function_results": []}
    assert events[1] == {"type": "agent_end", "messages": [message]}
    frames = fake.feed_frames(AGENT_EVENT_TYPE)
    assert all(f["session_id"] == "s1" and f["source"] == "hermes" for f in frames)


def test_run_mirrors_raw_result_event(monkeypatch):
    fake, _, h, _ = _make(monkeypatch, result="hi")
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    raw = fake.feed_frames(RAW_EVENT_TYPE)
    assert [f["event"] for f in raw] == [{"type": "result", "text": "hi", "is_error": False}]
    assert raw[0]["session_id"] == "s1"


def test_run_emits_raw_frame_before_normalized_frames(monkeypatch):
    fake, _, h, _ = _make(monkeypatch, result="hi")
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    order = [(c["function_id"], c["payload"]["event"]["type"]) for c in fake.void_calls()]
    assert order == [
        (consumer_fn(RAW_EVENT_TYPE), "result"),
        (consumer_fn(AGENT_EVENT_TYPE), "turn_end"),
        (consumer_fn(AGENT_EVENT_TYPE), "agent_end"),
    ]


def test_run_without_bindings_makes_no_feed_calls(monkeypatch):
    fake, _, h, _ = _make(monkeypatch, bind=())
    res = asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert res["result"] == "done"
    assert fake.void_calls() == []
    assert {c["function_id"] for c in fake.calls} == {"state::get", "state::set"}


def test_run_prepends_iii_context_on_fresh_session(monkeypatch):
    _, _, h, captured = _make(monkeypatch)
    asyncio.run(h["run"]({"prompt": "do it", "session_id": "s1"}))
    assert captured["prompt"].startswith(III_CONTEXT_PROMPT)
    assert captured["prompt"].endswith("do it")


def test_run_skips_iii_context_on_resume(monkeypatch):
    fake, _, h, captured = _make(monkeypatch)
    fake.state["hermes_sessions/s1"] = {"session_id": "s1", "turns": 1, "status": "done"}
    asyncio.run(h["run"]({"prompt": "again", "session_id": "s1"}))
    assert captured["prompt"] == "again"


def test_run_iii_context_false_disables_block(monkeypatch):
    _, _, h, captured = _make(monkeypatch, cfg=base_cfg(iii_context=False))
    asyncio.run(h["run"]({"prompt": "do it", "session_id": "s1"}))
    assert captured["prompt"] == "do it"


def test_run_per_turn_iii_context_override(monkeypatch):
    _, _, h, captured = _make(monkeypatch)
    asyncio.run(h["run"]({"prompt": "do it", "session_id": "s1", "iii_context": False}))
    assert captured["prompt"] == "do it"


def test_run_passes_cwd_and_model(monkeypatch):
    _, _, h, captured = _make(monkeypatch)
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1", "cwd": "/repo", "model": "anthropic/claude"}))
    assert captured["cwd"] == "/repo"
    assert captured["model"] == "anthropic/claude"


def test_run_defaults_cwd_model_from_config(monkeypatch):
    _, _, h, captured = _make(monkeypatch, cfg=base_cfg(defaults={"cwd": "/d", "model": "m"}))
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert captured["cwd"] == "/d"
    assert captured["model"] == "m"


def test_run_error_envelope_when_cli_raises(monkeypatch):
    fake, _, h, _ = _make(monkeypatch, raises=RuntimeError("spawn failed"))
    res = asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert res["is_error"] is True
    assert res["stop_reason"] == "error"
    assert "spawn failed" in res["result"]
    assert fake.state["hermes_sessions/s1"]["status"] == "error"
    types = [e["type"] for e in fake.feed_events(AGENT_EVENT_TYPE)]
    assert "turn_end" in types and "agent_end" in types


def test_run_resume_increments_turns(monkeypatch):
    fake, _, h, _ = _make(monkeypatch)
    fake.state["hermes_sessions/s1"] = {"session_id": "s1", "turns": 2, "status": "done"}
    res = asyncio.run(h["run"]({"prompt": "again", "session_id": "s1"}))
    assert fake.state["hermes_sessions/s1"]["turns"] == 3
    assert res["session_id"] == "s1"


def test_run_resumed_turns_emit_distinct_frames(monkeypatch):
    # Two runs on the same session_id must not collide event ids: the per-session
    # seq keeps counting, so turn 2's frames follow turn 1's.
    fake, _, h, _ = _make(monkeypatch, bind=("dup",))
    asyncio.run(h["run"]({"prompt": "one", "session_id": "dup"}))
    asyncio.run(h["run"]({"prompt": "two", "session_id": "dup"}))
    frames = fake.feed_frames(AGENT_EVENT_TYPE)
    ids = [f["event_id"] for f in frames]
    assert len(ids) == len(set(ids)) == 4, "resumed run reused event ids"
    assert [f["seq"] for f in frames] == [0, 1, 2, 3]
    assert [f["seq"] for f in fake.feed_frames(RAW_EVENT_TYPE)] == [0, 1]


def test_run_generates_session_id_when_absent(monkeypatch):
    _, _, h, _ = _make(monkeypatch)
    res = asyncio.run(h["run"]({"prompt": "x"}))
    assert isinstance(res["session_id"], str) and len(res["session_id"]) > 0


def test_run_messages_payload_extracts_last_user(monkeypatch):
    _, _, h, captured = _make(monkeypatch, cfg=base_cfg(iii_context=False))
    asyncio.run(
        h["run"](
            {
                "session_id": "s1",
                "messages": [
                    {"role": "user", "content": [{"type": "text", "text": "first"}]},
                    {"role": "assistant", "content": [{"type": "text", "text": "reply"}]},
                    {"role": "user", "content": [{"type": "text", "text": "second"}]},
                ],
            }
        )
    )
    assert captured["prompt"] == "second"


def test_run_threads_session_id_for_resume(monkeypatch):
    _, _, h, captured = _make(monkeypatch)
    asyncio.run(h["run"]({"prompt": "x", "session_id": "sess-abc"}))
    assert captured["session"] == "sess-abc"


def test_run_passes_default_toolset_profile(monkeypatch):
    _, _, h, captured = _make(monkeypatch, cfg=base_cfg(defaults={"tools": "terminal,file"}))
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert captured["toolsets"] == "terminal,file"


def test_run_per_turn_toolsets_override(monkeypatch):
    _, _, h, captured = _make(monkeypatch, cfg=base_cfg(defaults={"tools": "terminal"}))
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1", "tools": "web,file"}))
    assert captured["toolsets"] == "web,file"


def test_run_uses_configured_executable(monkeypatch):
    _, _, h, captured = _make(monkeypatch, cfg=base_cfg(hermes_executable="/opt/hermes"))
    asyncio.run(h["run"]({"prompt": "x", "session_id": "s1"}))
    assert captured["hermes"] == "/opt/hermes"


def test_start_returns_immediately_and_run_lands(monkeypatch):
    fake, _, h, _ = _make(monkeypatch)
    res = asyncio.run(_start_then_drain(h, {"prompt": "bg", "session_id": "s1"}))
    assert res["started"] is True
    assert res["session_id"] == "s1"
    assert fake.state.get("hermes_sessions/s1", {}).get("status") == "done"


async def _start_then_drain(h, data):
    res = await h["start"](data)
    # let the background run() task complete
    await asyncio.sleep(0)
    await asyncio.sleep(0)
    return res


def test_start_generates_session_id(monkeypatch):
    _, _, h, _ = _make(monkeypatch)
    res = asyncio.run(h["start"]({"prompt": "x"}))
    assert isinstance(res["session_id"], str) and res["started"] is True


@pytest.mark.parametrize("fn_name", ["run", "start"])
def test_busy_guard_blocks_concurrent_same_session(monkeypatch, fn_name):
    cfg = base_cfg()
    fake = FakeIii()
    logger = FakeLogger()
    gate = asyncio.Event()

    async def slow(hermes, prompt, *, cwd="", model="", session="", toolsets=""):
        await gate.wait()
        return "done", ""

    monkeypatch.setattr(handlers_mod.hermes_cli, "run_turn", slow)
    h = create_handlers(fake, lambda: cfg, logger)

    async def scenario():
        first = asyncio.create_task(h["run"]({"prompt": "a", "session_id": "busy"}))
        await asyncio.sleep(0)
        second = await h[fn_name]({"prompt": "b", "session_id": "busy"})
        gate.set()
        await first
        return second

    second = asyncio.run(scenario())
    assert second["busy"] is True
    if fn_name == "start":
        assert second["started"] is False
