import json
import time

from src.shadow import Shadow


def user(text):
    return {"role": "user", "content": [{"type": "text", "text": text}]}


def function_result(function_id, text):
    return {"role": "function_result", "function_id": function_id, "content": [{"type": "text", "text": text}]}


def test_decision_input_latest_user_only():
    objective, observation = Shadow.decision_input([user("first"), user("second")])
    assert objective == "second"
    assert observation is None


def test_decision_input_observation_after_user():
    messages = [user("count sessions"), function_result("session::list", '{"sessions": 50}')]
    objective, observation = Shadow.decision_input(messages)
    assert objective == "count sessions"
    assert observation == 'session::list returned: {"sessions": 50}'


def test_extract_calls_unwraps_agent_trigger():
    message = {
        "content": [
            {"type": "text", "text": "calling"},
            {
                "type": "function_call",
                "id": "c1",
                "function_id": "agent_trigger",
                "arguments": {"function": "worker::list", "payload": {"running_only": True}},
            },
            {"type": "function_call", "id": "c2", "function_id": "state::get", "arguments": {"key": "k"}},
        ]
    }
    calls = Shadow.extract_calls(message)
    assert calls[0] == {"function": "worker::list", "payload": {"running_only": True}, "via": "agent_trigger"}
    assert calls[1] == {"function": "state::get", "payload": {"key": "k"}}


class StubRouter:
    def __init__(self, result):
        self.result = result
        self.requests = []

    def route(self, request):
        self.requests.append(request)
        return self.result


def wait_for_lines(path, count, timeout_s=3.0):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            lines = open(path).read().splitlines()
        except FileNotFoundError:
            lines = []
        if len(lines) >= count:
            return [json.loads(line) for line in lines]
        time.sleep(0.02)
    raise AssertionError(f"expected {count} shadow rows in {path}")


def test_pre_generate_logs_proposal_in_background(tmp_path):
    log = tmp_path / "shadow.jsonl"
    router = StubRouter({"type": "call", "calls": [{"function": "worker::list", "payload": {}}], "confidence": 0.9})
    shadow = Shadow(router, log_path=str(log))
    out = shadow.pre_generate(
        {
            "session_id": "s1",
            "turn_id": "t1",
            "step": 0,
            "generate": {"model": "m", "messages": [user("list workers")]},
        }
    )
    assert out == {}
    rows = wait_for_lines(log, 1)
    assert rows[0]["kind"] == "proposal"
    assert rows[0]["calls"][0]["function"] == "worker::list"
    assert router.requests == [{"objective": "list workers"}]


def test_pre_generate_without_user_text_is_noop(tmp_path):
    shadow = Shadow(StubRouter({}), log_path=str(tmp_path / "shadow.jsonl"))
    assert shadow.pre_generate({"generate": {"messages": []}}) == {}
    assert not (tmp_path / "shadow.jsonl").exists()


def test_report_turn_level_scoring(tmp_path):
    log = tmp_path / "shadow.jsonl"
    shadow = Shadow(StubRouter({}), log_path=str(log))
    rows = [
        {
            "kind": "proposal",
            "session_id": "s1",
            "turn_id": "t1",
            "step": 0,
            "calls": [{"function": "worker::list", "payload": {}}],
            "confidence": 0.7,
        },
        {
            "kind": "actual",
            "session_id": "s1",
            "turn_id": "t1",
            "step": 0,
            "calls": [{"function": "engine::functions::list", "payload": {}}],
        },
        {
            "kind": "actual",
            "session_id": "s1",
            "turn_id": "t1",
            "step": 1,
            "calls": [{"function": "worker::list", "payload": {}}],
        },
        {
            "kind": "proposal",
            "session_id": "s2",
            "turn_id": "t2",
            "step": 0,
            "calls": [],
            "confidence": 0.05,
        },
        {
            "kind": "actual",
            "session_id": "s2",
            "turn_id": "t2",
            "step": 0,
            "calls": [{"function": "state::get", "payload": {}}],
        },
    ]
    with open(log, "w") as fh:
        for row in rows:
            fh.write(json.dumps(row) + "\n")

    report = shadow.report()
    assert report["turns"] == 2
    assert report["proposals_scored"] == 2
    assert report["turns_with_discovery_steps"] == 1
    assert report["discovery_steps_total"] == 1
    assert report["buckets"]["0.6"]["match"] == 1
    assert report["buckets"]["0.0"]["abstained"] == 1
