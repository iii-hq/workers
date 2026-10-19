"""Unit tests for the auto-effort phase 0 scripts. Synthetic lines only: corpus text never enters the repo."""

import hashlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

import ab
import analyze
import cache_probe
import common
import extract
import label

import judge

HERE = Path(__file__).resolve().parent


def line(d: dict) -> str:
    return json.dumps(d)


def meta(sid: str, kind: str = "user", **metadata) -> dict:
    return {"type": "meta", "meta": {"session_id": sid, "kind": kind, "metadata": metadata}}


def entry(type_: str, eid: str, rev: int, ts: int, message: dict) -> dict:
    return {
        "type": type_,
        "entry": {"kind": "message", "id": eid, "revision": rev, "timestamp": ts, "message": message},
    }


def user(eid: str, text: str, ts: int, origin: dict | None = None) -> dict:
    """A user message; `origin` marks one the harness injected, as the real entries do."""
    d = entry("append", eid, 0, ts, {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": ts})
    if origin:
        d["entry"]["origin"] = origin
    return d


def assistant(eid: str, ts: int, text: str = "", usage: dict | None = None) -> list[dict]:
    """Appended empty, then completed by an entry revision, as the real files do."""
    empty = {"role": "assistant", "model": "claude-opus-5-5", "content": [], "timestamp": ts}
    done = dict(empty, content=[{"type": "text", "text": text}] if text else [], usage=usage or {})
    return [entry("append", eid, 0, ts, empty), entry("entry", eid, 1, ts, done)]


def fake_run(returncode: int, stdout: str = "", stderr: str = ""):
    done = subprocess.CompletedProcess([], returncode, stdout, stderr)
    return mock.patch.object(common.subprocess, "run", return_value=done)


def reply(text: str) -> dict:
    """A router::complete result whose assistant message holds one text block."""
    message = {"role": "assistant", "content": [{"type": "text", "text": text}], "stop_reason": "end"}
    return {"message": message, "provider": "anthropic", "model": "claude-opus-5-5"}


class ExtractTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name) / "src" / "data" / "session-manager"
        self.dir.mkdir(parents=True)

    def write(self, sid: str, lines: list[dict], kind: str = "user", **metadata) -> None:
        text = "".join(line(d) + "\n" for d in [meta(sid, kind, **metadata), *lines])
        (self.dir / f"{sid}.jsonl").write_text(text, encoding="utf-8")

    def rows(self) -> list[dict]:
        return extract.prompts([self.dir])

    def test_skips_notifications(self):
        lines = [user("u1", "[notification] wake expired", 0), *assistant("a1", 10, "ok"), user("u2", "do X", 20)]
        self.write("s1", lines)
        self.assertEqual([r["message"] for r in self.rows()], ["do X"])

    def test_child_first_message_goes_to_child_slice(self):
        lines = [user("u1", "build the parser", 0), *assistant("a1", 10, "done"), user("u2", "also add tests", 20)]
        self.write("c1", lines, parent_session_id="s1")
        self.assertEqual([(r["slice"], r["message"]) for r in self.rows()], [("child", "build the parser")])

    def test_keeps_only_user_sessions(self):
        self.write("a", [user("u1", "do X", 0)], kind="e2e")
        self.write("b", [user("u1", "do X", 0)])
        self.write("c", [user("u1", "do Y", 0)], kind="automation")
        (self.dir / "d.jsonl").write_text(line(user("u1", "do Z", 0)) + "\n", encoding="utf-8")  # no meta: kind user
        self.assertEqual([r["session_id"] for r in self.rows()], ["b", "d"])

    def test_dedupes_same_text_in_slice(self):
        self.write("s1", [user("u1", "same text", 0)])
        self.write("s2", [user("u1", "same text", 0)])
        self.write("s3", [user("u1", "same text", 0)], parent_session_id="s1")
        self.assertEqual(sorted(r["slice"] for r in self.rows()), ["child", "top"])

    def test_turn_cost_weights(self):
        usage = {"input": 1000, "cache_read": 10000, "cache_write": 2000, "output": 100}
        self.write("s1", [user("u1", "do X", 0), *assistant("a1", 10, "done", usage)])
        [row] = self.rows()
        self.assertEqual((row["cost_units"], row["steps"], row["first_read"]), (5000.0, 1, 10000))

    def test_row_fields(self):
        self.write("s1", [user("u1", "do X", 0), *assistant("a1", 10, "done")], thinking_level="high")
        [row] = self.rows()
        self.assertEqual(row["id"], hashlib.sha1(b"s1u1").hexdigest()[:12])
        fields = ("source", "slice", "session_id", "seq", "context", "model", "level", "gap_ms")
        self.assertEqual([row[k] for k in fields], ["src", "top", "s1", 0, "", "claude-opus-5-5", "high", None])

    def test_gap_cold_after_5min(self):
        self.write("s1", [user("u1", "first", 0), *assistant("a1", 1_000, "done"), user("u2", "second", 302_000)])
        self.assertEqual([(r["seq"], r["gap_ms"]) for r in self.rows()], [(0, None), (1, 301_000)])

    def test_truncates_long_message(self):
        self.write("s1", [user("u1", "x" * 5000, 0)])
        [row] = self.rows()
        self.assertEqual(len(row["message"]), 4000)

    def test_context_shape(self):
        lines = [
            user("u1", "q" * 100 + "u" * 600, 0),
            *assistant("a1", 10, "a" * 100 + "b" * 1500),
            *assistant("a2", 20),
            user("u2", "next", 30),
        ]
        self.write("s1", lines)
        self.assertEqual(self.rows()[1]["context"], "user: " + "u" * 600 + "\nassistant: " + "b" * 1500)

    def test_folds_entry_revisions(self):
        empty = {"role": "assistant", "model": "claude-opus-5-5", "content": [], "timestamp": 10}
        final = dict(
            empty,
            content=[{"type": "text", "text": "rev 2"}],
            usage={"input": 10, "output": 2, "cache_read": 0, "cache_write": 0},
        )
        lines = [
            user("u1", "do X", 0),
            entry("append", "a1", 0, 10, empty),
            entry("entry", "a1", 1, 10, dict(empty, usage={})),
            entry("entry", "a1", 2, 10, final),
        ]
        self.write("s1", lines)
        [row] = self.rows()
        self.assertEqual((row["cost_units"], row["steps"]), (20.0, 1))
        replies = [m for m in extract.messages([meta("s1"), *lines]) if m["role"] == "assistant"]
        self.assertEqual([(m["id"], m["content"]) for m in replies], [("a1", final["content"])])

    def test_wake_cost_counts_for_the_prompt_before(self):
        """A wake runs at the level picked for the prompt before it, so its steps join that row's cost up to the next
        prompt; steps and first_read stay the prompt's own."""
        u1, u2, u3 = ({"input": n, "output": 1, "cache_read": 10 * n, "cache_write": 0} for n in (100, 7, 50))
        lines = [
            user("u1", "do X", 0),
            *assistant("a1", 10, "done", u1),
            user("u2", "[notification] wake", 20),
            *assistant("a2", 30, "woke", u2),
            user("u3", "do Y", 40),
            *assistant("a3", 50, "ok", u3),
        ]
        self.write("s1", lines)
        got = [(r["message"], r["cost_units"], r["steps"], r["first_read"]) for r in self.rows()]
        want = [("do X", common.units(u1) + common.units(u2), 1, 1000), ("do Y", common.units(u3), 1, 500)]
        self.assertEqual(got, want)

    def test_injected_turns_are_no_prompt_rows(self):
        """A top row is a person's prompt, with no origin, and a child row its spawned task; a skill notice, a steer
        or a spawn into a top session is no row, and its steps count for the prompt before it."""
        u = {"input": 10, "output": 0, "cache_read": 0, "cache_write": 0}
        top = [
            user("u1", "do X", 0),
            *assistant("a1", 10, "done", u),
            user("u2", "skills changed", 20, origin={"skill_update": True}),
            *assistant("a2", 30, "noted", u),
            user("u3", "also Y", 40, origin={"turn_id": "t1"}),
            user("u4", "a scripted task", 50, origin={"spawn": True}),
        ]
        self.write("s1", top)
        task = [user("u1", "build it", 0, origin={"spawn": True}), *assistant("a1", 10, "done", u)]
        self.write("c1", task, parent_session_id="s1")
        got = [(r["slice"], r["message"], r["cost_units"]) for r in self.rows()]
        self.assertEqual(got, [("child", "build it", 10.0), ("top", "do X", 20.0)])

    def test_turn_levels_reads_embedded_records(self):
        path = self.dir.parent / "state" / "harness_turn.bin"
        path.parent.mkdir()
        records = (
            b'{"s_a":{"options":{"thinking_level":"high"}}}',
            b'{"console-x":{"options":{"thinking_level":"minimal"}}}',
            b'{"s_a":{"options":{"thinking_level":"medium"}}}',
        )
        path.write_bytes(b"\x00\x93junk".join([b"\xff\x01", *records, b"\x07"]))
        want = {"s_a": "medium", "console-x": "low"}
        self.assertEqual(extract.turn_levels(path, {"s_a", "console-x", "s_missing"}), want)
        # a store holds one map of every session's latest turn, where a record past the first follows a comma
        records = (
            b'"s_m":{"options":{"thinking_level":"off"}}',
            b'"s_n":{"options":{"thinking_level":null}}',
            b'"s_o":{"options":{"thinking_level":"high"}}',
        )
        path.write_bytes(b"{" + b",".join(records) + b"}\x05\x00")
        self.assertEqual(extract.turn_levels(path, {"s_m", "s_n", "s_o"}), {"s_m": "low", "s_o": "high"})
        self.assertEqual(extract.turn_levels(path.with_name("missing.bin"), {"s_a"}), {})

    def test_row_level_prefers_turn_record(self):
        state = self.dir.parent / "state"
        state.mkdir()
        (state / "harness_turn.bin").write_bytes(b'{"s1":{"options":{"thinking_level":"high"}}}\x00')
        self.write("s1", [user("u1", "do X", 0)], thinking_level="medium")
        self.write("s2", [user("u1", "do Y", 0)], thinking_level="medium")
        self.write("s3", [user("u1", "do Z", 0)])
        self.write("s4", [user("u1", "do W", 0)], thinking_level="minimal")  # runs as low
        got = [(r["session_id"], r["level"], r["level_source"]) for r in self.rows()]
        want = [("s1", "high", "turn_record"), ("s2", "medium", "metadata"), ("s3", None, None)]
        self.assertEqual(got, [*want, ("s4", "low", "metadata")])


class CommonTest(unittest.TestCase):
    def test_write_jsonl_keeps_the_old_file_when_stopped_mid_write(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "ab.jsonl"
        common.write_jsonl(path, [{"run": 1}])
        with self.assertRaises(TypeError):  # the second row fails after the first is written
            common.write_jsonl(path, [{"run": 1}, {"run": object()}])
        self.assertEqual(common.read_jsonl(path), [{"run": 1}])

    def test_trigger_outcomes(self):
        with fake_run(0, '{\n  "status": "ok"\n}\n') as run:
            self.assertEqual(common.trigger("judge::evaluate", {"a": 1}, 3), {"status": "ok"})
        argv = ["iii", "trigger", "-n", "my-project", "--timeout-ms", "3000", "judge::evaluate", "--json", '{"a": 1}']
        self.assertEqual(run.call_args.args[0], argv)
        with fake_run(0):
            self.assertEqual(common.trigger("f", {}, 1), {})
        remote = 'Error: {\n  "code": "timeout",\n  "message": "too slow",\n  "stacktrace": null\n}\n'
        with fake_run(1, stderr=remote), self.assertRaises(common.TriggerError) as err:
            common.trigger("f", {}, 1)
        self.assertEqual((err.exception.code, err.exception.message), ("timeout", "too slow"))
        no_engine = "Error: Timed out waiting for the engine\n"
        with fake_run(1, stderr=no_engine), self.assertRaises(common.TriggerError) as err:
            common.trigger("f", {}, 1)
        self.assertEqual(err.exception.code, "transport")


class LabelTest(unittest.TestCase):
    def test_parse_label_orders_levels(self):
        self.assertEqual(label.parse_label('{"ok":["high","medium"]}'), ["medium", "high"])

    def test_parse_label_rejects_unknown(self):
        self.assertIsNone(label.parse_label('{"ok":["max"]}'))
        self.assertIsNone(label.parse_label('{"ok":[]}'))

    def test_needs_adjudication(self):
        self.assertTrue(label.needs_adjudication(["low"], ["medium", "high"]))
        self.assertFalse(label.needs_adjudication(["low", "medium"], ["low"]))

    def test_effortless_rows(self):
        rows = label.effortless_rows()
        self.assertEqual((len(rows), sum(r["holdout"] for r in rows)), (75, 20))
        self.assertTrue(all(r["ok"] and set(r["ok"]) <= set(common.LEVELS) for r in rows))
        self.assertTrue(all(r["ok"] == sorted(r["ok"], key=common.LEVELS.index) for r in rows))  # cheapest first
        by_id = {r["id"]: r for r in rows}
        h12, o01 = by_id["eff-h12"], by_id["eff-o01"]  # the case file lists h12's ok as ["high","medium"]
        fields = ("slice", "holdout", "model", "level", "ok")
        want = ["effortless", True, "claude-sonnet-5-5", "medium", ["medium", "high"]]
        self.assertEqual([h12[k] for k in fields], want)
        self.assertEqual((h12["message"], bool(h12["context"])), ("the second one", True))
        self.assertEqual([o01[k] for k in ("holdout", "model", "context")], [False, "claude-opus-5-5", ""])

    def test_effort_question_criteria_are_levels(self):
        question = json.loads((HERE / "effort_question.json").read_text(encoding="utf-8"))
        self.assertEqual(tuple(question["criteria"]), common.LEVELS)
        rubric = (HERE / "rubric.md").read_text(encoding="utf-8")
        self.assertTrue(all(text in rubric for text in question["criteria"].values()))

    def test_label_row_adjudicates_disagreement(self):
        row = {"id": "r1", "model": "claude-sonnet-5-5", "context": "", "message": "do X"}
        agree = [reply('{"ok":["low","medium"]}'), reply('```json\n{"ok":["low"]}\n```')]
        with mock.patch.object(common, "trigger", side_effect=agree) as call:
            self.assertEqual(label.label_row(row, "rubric"), {"id": "r1", "ok": ["low", "medium"], "agreed": True})
        fn, payload = call.call_args.args
        request = (fn, payload["model"], payload["provider"], payload["thinking_level"], payload["system_prompt"])
        self.assertEqual(request, ("router::complete", "claude-opus-5-5", "anthropic", "medium", "rubric"))
        [message] = payload["messages"]
        shape = (message["role"], message["content"][0]["type"], type(message["timestamp"]))
        self.assertEqual(shape, ("user", "text", int))
        sent = json.loads(message["content"][0]["text"])
        self.assertEqual(sent, {"model": "claude-sonnet-5-5", "context": "", "message": "do X"})
        split = [reply('{"ok":["low"]}'), reply('{"ok":["medium","high"]}'), reply('{"ok":["medium"]}')]
        with mock.patch.object(common, "trigger", side_effect=split) as call:
            self.assertEqual(label.label_row(row, "rubric"), {"id": "r1", "ok": ["medium"], "agreed": False})
        third = json.loads(call.call_args.args[1]["messages"][0]["content"][0]["text"])
        self.assertEqual(third["answers"], [["low"], ["medium", "high"]])

    def test_dry_run_resume_then_stop_after_five_misses_in_a_row(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        out = Path(tmp.name)
        rows = [{"id": f"p{i}", "model": "m", "context": "", "message": "x"} for i in range(8)]
        common.write_jsonl(out / "prompts.jsonl", rows)
        seeded = {"id": "p0", "ok": ["medium"], "agreed": True}  # labelled by an earlier run
        printed = io.StringIO()
        with mock.patch.object(common, "trigger") as call, redirect_stdout(printed):
            label.main([str(out)])
            common.write_jsonl(out / "labels.jsonl", [seeded])
            label.main([str(out)])
        call.assert_not_called()  # no --run, no call
        counts = [p.split(", about")[0] for p in printed.getvalue().splitlines()]
        self.assertEqual(counts, ["8 prompts × 2–3 calls (16–24 calls)", "7 prompts × 2–3 calls (14–21 calls)"])
        timeout, ok, prose = common.TriggerError("timeout", "too slow"), reply('{"ok":["low"]}'), reply("y" * 300)
        failed = {"message": {"content": [], "stop_reason": "error", "error_message": "rate limited"}}
        # p0 is skipped; p1 throws, p2 is labelled; then p3 throws and p4..p7 get no usable reply: five misses in a row
        replies = [timeout, ok, ok, timeout, failed, *[prose] * 11]
        log = io.StringIO()
        with mock.patch.object(common, "trigger", side_effect=replies) as call, redirect_stderr(log):
            with self.assertRaises(SystemExit) as stop:
                label.main([str(out), "--run"])
        self.assertEqual(call.call_count, len(replies))
        labels = [seeded, {"id": "p2", "ok": ["low"], "agreed": True}]
        self.assertEqual(common.read_jsonl(out / "labels.jsonl"), labels)
        self.assertIn("5 prompts in a row", str(stop.exception))
        lines = log.getvalue().splitlines()
        self.assertIn("p1: timeout: too slow", lines)
        self.assertIn("p7: no label", lines)
        self.assertIn("no label in reply (rate limited): ''", lines)
        self.assertIn(f"no label in reply (end): {'y' * 200!r}", lines)


def judge_reply(rid: str, choice: str, confidence: float) -> dict:
    """A judge::evaluate ok reply for one evaluation."""
    answer = {"choice": choice, "probabilities": {choice: confidence}, "confidence": confidence}
    result = {"answers": {"effort": answer}, "usage": {"input_tokens": 90, "output_tokens": 3}}
    return {"status": "ok", "model": "jev", "results": {rid: result}}


class JudgeTest(unittest.TestCase):
    question = json.loads(judge.QUESTION.read_text(encoding="utf-8"))
    row = {"id": "r1", "model": "claude-opus-5-5", "level": "medium", "context": "", "message": "fix the parser"}

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.out = Path(tmp.name)

    def judge_main(self, *args: str) -> list[str]:
        printed = io.StringIO()
        with redirect_stdout(printed):
            judge.main([str(self.out), *args])
        return printed.getvalue().splitlines()

    def test_low_confidence_keeps_current(self):
        v = judge.verdict({"choice": "low", "confidence": 0.4}, None, "high")
        self.assertEqual((v["level"], v["kept"]), ("high", True))

    def test_error_counts_as_kept(self):
        v = judge.verdict(None, "timeout", None)
        self.assertEqual((v["level"], v["kept"], v["error"]), ("medium", True, "timeout"))

    def test_followup_keeps_without_call(self):
        self.assertTrue(judge.keeps_effort("ok", "user: x\nassistant: done."))

    def test_followup_answering_question_is_judged(self):
        self.assertFalse(judge.keeps_effort("sim", "user: x\nassistant: Should I refactor X?"))

    def test_payload_shape(self):
        e = judge.evaluation(self.row, self.question)
        effort = e["questions"]["effort"]
        self.assertEqual((e["id"], effort["type"], set(effort["criteria"])), ("r1", "choice", set(common.LEVELS)))
        self.assertIsInstance(e["state"], str)
        keys = ["task", "current_model", "current_effort", "recent_conversation", "next_message"]
        self.assertEqual(list(json.loads(e["state"])), keys)

    def test_paid_arms_read_replies(self):
        fields = ("id", "arm", "level", "confidence", "kept", "error", "tokens")
        followup = dict(self.row, message="ok")
        down = common.TriggerError("transport", "no engine")
        replies = [judge_reply("r1", "low", 0.8), {"status": "error", "code": "deadline"}, down]
        with mock.patch.object(common, "trigger", side_effect=replies) as call:
            got = [judge.judged("typesafe", r, self.question, "") for r in (self.row, followup, self.row, self.row)]
        self.assertEqual(list(got[0]), ["id", "arm", "level", "confidence", "kept", "error", "ms", "tokens"])
        expect = [
            ("r1", "typesafe", "low", 0.8, False, None, 93),
            ("r1", "typesafe", "medium", None, True, None, 0),  # the follow-up makes no call
            ("r1", "typesafe", "medium", None, True, "deadline", 0),
            ("r1", "typesafe", "medium", None, True, "transport", 0),
        ]
        self.assertEqual([tuple(v[k] for k in fields) for v in got], expect)
        self.assertEqual(got[1]["ms"], 0)
        evaluation = judge.evaluation(self.row, self.question)
        request = {"timeout_ms": 3000, "provider": "typesafe", "evaluations": [evaluation]}
        self.assertEqual(call.call_args_list[0].args, ("judge::evaluate", request))
        haiku = dict(reply('{"effort": "high", "sure": 0.7} (one edit {small})'), usage={"input": 900, "output": 10})
        with mock.patch.object(common, "trigger", side_effect=[haiku, reply("high, I think")]) as call:
            got = [judge.judged("haiku", self.row, self.question, "system") for _ in range(2)]
        expect = [
            ("r1", "haiku", "high", 0.7, False, None, 910),
            ("r1", "haiku", "medium", None, True, "invalid_response", 0),
        ]
        self.assertEqual([tuple(v[k] for k in fields) for v in got], expect)
        fn, payload = call.call_args_list[0].args
        [message] = payload.pop("messages")
        want = {
            "model": "claude-haiku-5-5",
            "provider": "anthropic",
            "system_prompt": "system",
            "max_output_tokens": 160,
        }
        self.assertEqual((fn, payload), ("router::complete", want))  # no thinking_level
        text = [{"type": "text", "text": evaluation["state"]}]
        self.assertEqual((message["role"], message["content"], type(message["timestamp"])), ("user", text, int))

    def test_verdicts_upsert_by_id_and_arm(self):
        rows = self.out / "ab_prompts.jsonl"
        common.write_jsonl(rows, [dict(self.row, level="high"), dict(self.row, id="r2", level=None)])
        common.write_jsonl(self.out / "prompts.jsonl", [dict(self.row, id="p1")])
        path = self.out / "verdicts.jsonl"
        stale = {"id": "r1", "arm": "keep", "level": "low", "confidence": None, "kept": True, "error": None}
        other = dict(stale, arm="haiku", ms=700, tokens=900)
        common.write_jsonl(path, [dict(stale, ms=0, tokens=0), other])
        with mock.patch.object(common, "trigger") as call:
            self.judge_main("--arm", "always-low", "keep", "--rows", str(rows), "--run")
            self.judge_main("--arm", "keep", "--rows", str(rows), "--run")
            self.judge_main("--arm", "always-high", "--run")  # default rows: prompts.jsonl plus the effortless cases
        call.assert_not_called()
        verdicts = common.read_jsonl(path)
        got = [(v["id"], v["arm"], v["level"], v["kept"]) for v in verdicts[:5]]
        expect = [
            ("r1", "keep", "high", True),
            ("r1", "haiku", "low", True),
            ("r1", "always-low", "low", False),
            ("r2", "always-low", "low", False),
            ("r2", "keep", "medium", True),
        ]
        self.assertEqual(got, expect)
        self.assertEqual(verdicts[1], other)
        high = [v["id"] for v in verdicts[5:] if v["arm"] == "always-high"]
        self.assertEqual((len(verdicts), len(high), high[0], "eff-h12" in high), (81, 76, "p1", True))
        self.assertEqual({(v["ms"], v["tokens"]) for v in verdicts[:1] + verdicts[2:]}, {(0, 0)})

    def test_dry_run_resume_then_stop_after_five_unreached_calls_in_a_row(self):
        rows = [dict(self.row, id=f"r{i}") for i in range(13)]
        rows[7]["message"] = "ok"  # a follow-up makes no call, so it neither counts nor resets
        common.write_jsonl(self.out / "rows.jsonl", rows)
        path = self.out / "verdicts.jsonl"
        earlier = {"arm": "typesafe", "level": "medium", "kept": True}
        errors = (None, "deadline", "transport")  # r0 answered, r1 a judge error, r2 never reached the judge
        common.write_jsonl(path, [dict(earlier, id=f"r{i}", error=e) for i, e in enumerate(errors)])
        down = common.TriggerError("transport", "no engine")
        deadline = {"status": "error", "code": "deadline"}  # the judge answered late: a kept row, and the stack is up
        replies = [judge_reply("r2", "low", 0.9), down, down, deadline, *[down] * 5]
        args = ("--rows", str(self.out / "rows.jsonl"))
        with mock.patch.object(common, "trigger", side_effect=replies) as call:
            before = self.judge_main("--arm", "typesafe", "keep", *args)
            self.assertEqual(call.call_count, 0)
            with self.assertRaises(SystemExit) as stop:
                self.judge_main("--arm", "typesafe", *args, "--run")
            after = self.judge_main("--arm", "typesafe", *args)
        counts = [p.split(", about")[0] for p in before + after]
        # only a transport error is asked again: r0's answer and r1's judge error stand, as r2's and r5's do after
        want = ["typesafe: 13 rows, 10 calls", "keep: 13 rows, 0 calls", "typesafe: 13 rows, 8 calls"]
        self.assertEqual(counts, want)
        # r2 answers; r3 and r4 fail, r5's deadline starts the count again, then r6, r8, r9, r10 and r11 fail; not r12
        self.assertEqual(call.call_count, 9)
        self.assertIn("could not reach the stack 5 times in a row", str(stop.exception))
        want = [("r0", None), ("r1", "deadline"), ("r2", None), ("r3", "transport"), ("r4", "transport")]
        want += [("r5", "deadline"), ("r6", "transport"), ("r7", None), *((f"r{i}", "transport") for i in range(8, 12))]
        self.assertEqual([(v["id"], v["error"]) for v in common.read_jsonl(path)], want)  # written through the stop


def verdict_row(rid: str, arm: str, level: str, ms: int = 0, tokens: int = 0) -> dict:
    """A verdicts.jsonl row that was not kept."""
    return dict(id=rid, arm=arm, level=level, confidence=None, kept=False, error=None, ms=ms, tokens=tokens)


def keyed(verdicts: list[dict]) -> dict:
    """Verdicts as analyze reads them: one per (id, arm), the last row winning."""
    return {(v["id"], v["arm"]): v for v in verdicts}


class AnalyzeTest(unittest.TestCase):
    def test_grade(self):
        self.assertEqual(analyze.grade("low", ["medium", "high"]), "low")
        self.assertEqual(analyze.grade("xhigh", ["medium", "high"]), "high")
        self.assertEqual(analyze.grade("high", ["medium", "high"]), "right")

    def test_grade_counts_kept_errors(self):
        right = verdict_row("r1", "haiku", "medium", ms=800, tokens=900)
        error = dict(verdict_row("r2", "haiku", "medium", ms=6000), kept=True, error="transport")
        got = analyze.stats([right, error], {"r1": ["medium"], "r2": ["high"]})
        fields = ("rows", "labelled", "right", "low", "high", "kept", "p50", "p95", "tokens")
        self.assertEqual([got[k] for k in fields], [2, 2, 0.5, 0.5, 0.0, 0.5, 800, 6000, 900])

    def test_p95(self):
        self.assertEqual(analyze.p95(list(range(1, 101))), 95)

    def test_flip_penalty_warm_only(self):
        row = {"cost_units": 100, "level": "medium", "first_read": 1000, "model": "m"}
        ratios = {"m": {"low": 0.7, "medium": 1.0}}
        self.assertEqual(analyze.modeled_cost(row, "low", ratios, flip_safe=set(), flipped_warm=False), 70.0)
        self.assertEqual(analyze.modeled_cost(row, "low", ratios, flip_safe=set(), flipped_warm=True), 70.0 + 1150.0)
        self.assertEqual(analyze.modeled_cost(row, "low", ratios, flip_safe={"m"}, flipped_warm=True), 70.0)
        prefixed = dict(row, model="claude-code/m")  # looked up by the last path segment, in ratios and in flip_safe
        self.assertEqual(analyze.modeled_cost(prefixed, "low", ratios, flip_safe={"m"}, flipped_warm=True), 70.0)
        ran_low = dict(row, level="low")  # relative to the level the session ran at
        self.assertEqual(analyze.modeled_cost(ran_low, "medium", ratios, set(), False), 100 / 0.7)

    def test_flips_warm_flag(self):
        levels = [("medium", None), ("low", 120_000), ("high", 400_000)]
        turns = [
            {"id": f"t{i}", "session_id": "s1", "seq": i, "level": lv, "gap_ms": gap}
            for i, (lv, gap) in enumerate(levels)
        ]
        other = {"id": "u0", "session_id": "s2", "seq": 0, "level": "xhigh", "gap_ms": 1_000}  # never compared with s1
        got = analyze.flips([other, *reversed(turns)])
        self.assertEqual([f["warm"] for f in got], [True, False])
        self.assertEqual([(f["id"], f["from"], f["to"]) for f in got], [("t1", "medium", "low"), ("t2", "low", "high")])

    def test_kept_turn_runs_at_the_previous_level(self):
        """A kept verdict holds the session's level, but Auto would keep the level of the judged turn before."""

        def kept(rid: str, level: str) -> dict:
            return dict(verdict_row(rid, "haiku", level), kept=True)

        turn = {"slice": "top", "model": "m", "cost_units": 10_000, "first_read": 50_000, "gap_ms": 60_000}
        s1 = [dict(turn, id=f"t{i}", session_id="s1", seq=i, level="medium") for i in range(3)]
        s2 = [dict(turn, id=f"u{i}", session_id="s2", seq=i, level="high") for i in range(4)]
        verdicts = [verdict_row("t0", "haiku", "low"), kept("t1", "medium"), verdict_row("t2", "haiku", "low")]
        verdicts += [kept("u0", "high"), verdict_row("u1", "haiku", "low"), kept("u2", "high"), kept("u3", "high")]
        got = analyze.flips(analyze.judged("haiku", [*reversed(s1 + s2)], keyed(verdicts)))
        # s1 stays at low; s2 starts at its own level, not at s1's last, and u3 inherits u2's inherited low
        self.assertEqual(got, [{"id": "u1", "from": "high", "to": "low", "warm": True}])
        c = analyze.costs("haiku", s1, keyed(verdicts), {"m": {"low": 0.7, "medium": 1.0}}, set())["m"]
        self.assertEqual((c["actual"], c["modeled"]), (30_000, 21_000))  # 0.7 × actual and no warm-flip penalty

    def test_judge_tokens_at_the_judge_price(self):
        """Judge tokens join the modeled cost in units of the row's model: Haiku's at $0.10 against Opus's $4 per
        million input tokens, and Jev's at Haiku's rate too, until TypeSafe publishes a price (R16)."""
        row = {"id": "t0", "session_id": "s1", "seq": 0, "model": "claude-code/claude-opus-5-5", "level": "medium"}
        row |= {"cost_units": 1000.0, "first_read": 0, "gap_ms": None}
        picks = [verdict_row("t0", "haiku", "medium", tokens=4000), verdict_row("t0", "typesafe", "medium", tokens=40)]
        ratios = {"claude-opus-5-5": {"medium": 1.0}}

        def modeled(arm: str) -> float:
            return analyze.costs(arm, [row], keyed(picks), ratios, set())["claude-opus-5-5"]["modeled"]

        self.assertEqual((modeled("haiku"), modeled("typesafe")), (1000.0 + 100.0, 1000.0 + 1.0))

    def test_g1_wants_a_judge_arm_on_labelled_rows(self):
        rows = [{"id": f"h{i}", "slice": "effortless-holdout", "ok": ["medium"]} for i in range(20)]
        rows += [{"id": f"t{i}", "slice": "top", "ok": ["medium"]} for i in range(9)]
        rows += [{"id": "t9", "slice": "top"}, {"id": "c0", "slice": "child", "ok": ["low"]}]  # top 90% labelled
        haiku = [verdict_row("h0", "haiku", "high", ms=3000)]  # one pick too high, and the one slow call of 20
        haiku += [verdict_row(f"h{i}", "haiku", "medium", ms=700) for i in range(1, 20)]
        typesafe = [verdict_row(f"h{i}", "typesafe", "medium", ms=1600) for i in range(20)]
        baseline = [verdict_row(f"h{i}", "always-medium", "medium") for i in range(20)]

        def g1(verdicts: list[dict], rows: list[dict] = rows) -> tuple[bool, str]:
            return analyze.gates(rows, keyed(verdicts), None, None, None)["G1"]

        passed, text = g1(typesafe + baseline + haiku)
        self.assertTrue(passed)
        self.assertTrue(text.startswith("PASS: winner haiku. "), text)
        self.assertIn("haiku PASS right 95.0% low 0.0% p95 700 ms on 20 labelled of 20 rows", text)
        self.assertIn("typesafe FAIL right 100.0% low 0.0% p95 1600 ms", text)
        self.assertFalse(g1(haiku + [verdict_row("h0", "haiku", "low", ms=3000)])[0])  # 5% too low

        def too_high(n: int) -> list[dict]:
            return [verdict_row(f"h{i}", "haiku", "high" if i < n else "medium", ms=700) for i in range(20)]

        self.assertTrue(g1(too_high(2))[0])  # right 90%: the bar holds at equality
        self.assertFalse(g1(too_high(3))[0])  # right 85%, with nothing too low and a fast p95
        fast = [dict(v, ms=1400) for v in typesafe]
        self.assertTrue(g1(haiku + fast)[1].startswith("PASS: winner typesafe. "))  # the most right of those that pass
        passed, text = g1(baseline)  # a baseline that clears the bars is shown, but never wins G1
        self.assertFalse(passed)
        self.assertIn("always-medium (baseline) PASS right 100.0%", text)
        unlabelled = [verdict_row("t9", "haiku", "medium"), verdict_row("c0", "haiku", "low")]  # child is no G1 slice
        want = (False, "FAIL: no judge arm passes. typesafe: no verdicts; haiku FAIL: no labelled rows")
        self.assertEqual(g1(unlabelled), want)
        fewer_top_labels = [r for r in rows if r["id"] != "t0"]  # 8 of 9: labels on part of top decide nothing (R18)
        passed, text = g1(typesafe + baseline + haiku, fewer_top_labels)
        self.assertFalse(passed)
        self.assertTrue(text.startswith("INCOMPLETE: labels cover 8/9 top rows (need 90%). typesafe right "), text)
        self.assertIn("haiku right 95.0% low 0.0% p95 700 ms on 20 labelled of 20 rows", text)
        self.assertFalse(g1(haiku, rows[:20])[0])  # nor with no top rows at all, before extract.py

    def test_g1_incomplete_below_label_coverage(self):
        """R18: G1 grades no arm until labels cover 90% of the top rows."""
        rows = [{"id": f"h{i}", "slice": "effortless-holdout", "ok": ["medium"]} for i in range(20)]
        top = [{"id": f"t{i}", "slice": "top"} for i in range(10)]
        haiku = keyed([verdict_row(r["id"], "haiku", "medium", ms=700) for r in rows + top])

        def g1(labelled: int) -> tuple[bool, str]:
            labels = [dict(r, ok=["medium"]) if i < labelled else r for i, r in enumerate(top)]
            return analyze.gates(rows + labels, haiku, None, None, None)["G1"]

        arm = "haiku{} right 100.0% low 0.0% p95 700 ms on {} labelled of 30 rows"
        want = "INCOMPLETE: labels cover 8/10 top rows (need 90%). typesafe: no verdicts; " + arm.format("", 28)
        self.assertEqual(g1(8), (False, want))
        self.assertEqual(g1(9), (True, "PASS: winner haiku. typesafe: no verdicts; " + arm.format(" PASS", 29)))

    def test_g2_counts_valid_pairs_only(self):
        def pair(model: str, retention: float | None, invalid: str | None = None) -> dict:
            return {"model": model, "pair": 0, "retention": retention, "invalid": invalid}

        cache = [pair("a", r) for r in (0.95, 0.92, 0.99)]
        cache += [pair("b", 0.95), pair("b", 0.97), pair("b", None, "turn failed")]  # 2 valid pairs are too few
        cache += [pair("c", r) for r in (0.95, 0.99, 0.85)]
        self.assertEqual(analyze.safe_models(cache), {"a"})
        want = "PASS: a flip-safe (retention 0.95 0.92 0.99); b not flip-safe (retention 0.95 0.97 invalid);"
        want += " c not flip-safe (retention 0.95 0.99 0.85)"
        self.assertEqual(analyze.gates([], {}, cache, None, None)["G2"], (True, want))
        self.assertFalse(analyze.gates([], {}, cache[3:], None, None)["G2"][0])
        want = (False, "INCOMPLETE: no cache.jsonl yet, so no model is flip-safe")
        self.assertEqual(analyze.gates([], {}, None, None, None)["G2"], want)

    @mock.patch.dict("common.INPUT_USD", {"m": 0.10})  # priced as Haiku, so its judge tokens count 1:1
    def test_g3_value_needs_every_input(self):
        rows = [{"id": f"h{i}", "slice": "effortless-holdout", "ok": ["low"]} for i in range(20)]
        turn = {"slice": "top", "session_id": "s1", "model": "claude-code/m", "level": "medium", "cost_units": 1000.0}
        rows.append(dict(turn, id="c0", seq=0, gap_ms=None, first_read=0, ok=["low"]))  # G1 needs labelled top rows
        rows.append(dict(turn, id="c1", seq=1, gap_ms=60_000, first_read=500, ok=["low"]))
        rows.append(dict(turn, id="g0", seq=2, gap_ms=60_000, first_read=500, model="gpt-x", ok=["high"]))  # no ratios
        picks = {"c0": "low", "c1": "low", "g0": "high", "ab-m-a": "low", "ab-m-c": "high"}
        picks |= {f"h{i}": "low" for i in range(20)}
        verdicts = [verdict_row(rid, "haiku", level, ms=500, tokens=10) for rid, level in picks.items()]
        ratios = {"m": {"low": 0.7, "medium": 1.0, "high": 1.3}}

        def runs(low_a: int = 3, high_c: int = 3, medium_c: int = 1) -> list[dict]:
            """3 A/B runs per (level, task), of which the arguments say how many pass; c is a hard task."""
            plan = (("low", "a", low_a), ("medium", "a", 3), ("high", "c", high_c), ("medium", "c", medium_c))
            return [
                {"model": "m", "level": lv, "task": t, "run": k, "pass": k < n} for lv, t, n in plan for k in range(3)
            ]

        ab = runs()

        def g3(vs=verdicts, cache=None, ratios=ratios, ab=ab) -> tuple[bool, str]:
            return analyze.gates(rows, keyed(vs), cache, ratios, ab)["G3"]

        passed, text = g3()  # 2 × (1000 × 0.7 + 10 judge tokens) against 2 × 1000
        self.assertTrue(passed)
        want = "PASS: haiku: modeled cost -29.0% on 2 rows (left out, no ratios: gpt-x 1); "
        self.assertTrue(text.startswith(want), text)
        self.assertIn("A/B passes 6/6 at the picked levels vs 4/6 at medium; hard tasks +2 of 3", text)
        self.assertFalse(g3(ab=runs(low_a=2, medium_c=3))[0])  # cheaper, but 5/6 passes against 6/6 at medium
        warm_flip = verdicts + [verdict_row("c1", "haiku", "medium", tokens=10)]  # c0 low, then c1 medium 60 s later
        self.assertFalse(g3(warm_flip)[0])  # 710 + 1000 + 500 × 1.15 + 10 against 2000: +14.75%, over the cost cap
        safe = [{"model": "m", "pair": k, "retention": 0.95, "invalid": None} for k in range(3)]
        self.assertTrue(g3(warm_flip, cache=safe)[0])  # no cache rewrite on a flip-safe model: -14.0%
        cold = [dict(r, gap_ms=400_000) if r["id"] == "c1" else r for r in rows]  # c1 400 s after c0's last step
        passed, text = analyze.gates(cold, keyed(warm_flip), None, ratios, ab)["G3"]
        self.assertTrue(passed)
        self.assertIn("modeled cost -14.0%", text)  # a cold flip rewrites nothing either
        small_saving = {"m": dict(ratios["m"], low=0.95)}  # -4%
        self.assertTrue(g3(ratios=small_saving)[0])  # 2 more hard-task passes, within +10%
        self.assertFalse(g3(ratios=small_saving, ab=runs(high_c=2))[0])  # 1 more is not enough
        self.assertEqual(g3(ab=None), (False, "INCOMPLETE: no ab.jsonl"))
        no_pick = [v for v in verdicts if v["id"] != "ab-m-c"]
        self.assertTrue(g3(no_pick)[1].startswith("INCOMPLETE: no haiku verdict for ab-m-c. "))
        xhigh = verdicts + [verdict_row("c1", "haiku", "xhigh")]  # the A/B measured no xhigh cost
        self.assertTrue(g3(xhigh)[1].startswith("INCOMPLETE: m: 1 rows at a level without a ratio. "))
        with mock.patch.dict(common.INPUT_USD):  # dollars pool models, so a model with no price leaves G3 open
            del common.INPUT_USD["m"]
            self.assertTrue(g3()[1].startswith("INCOMPLETE: m: no input price; no priced corpus cost. "))

    def test_g3_skips_unknown_level_rows(self):
        """A row whose session level is unknown is left out of G3, never assumed medium, and the report counts it."""
        turn = {"slice": "top", "model": "claude-opus-5-5", "cost_units": 1_000_000.0, "first_read": 0, "gap_ms": None}
        corpus = [dict(turn, id="t0", session_id="s1", seq=0, level="medium")]
        corpus.append(dict(turn, id="u0", session_id="s2", seq=0, level=None))
        picks = [verdict_row("t0", "haiku", "low"), verdict_row("u0", "haiku", "high")]
        verdicts = keyed([*picks, verdict_row("ab-claude-opus-5-5-a", "haiku", "medium")])
        ratios = {"claude-opus-5-5": {"low": 0.7, "medium": 1.0, "high": 1.3}}
        ab = [{"model": "claude-opus-5-5", "level": "medium", "task": "a", "run": 0, "pass": True}]
        _, text = analyze.g3("haiku", corpus, verdicts, set(), ratios, ab)
        self.assertTrue(text.startswith("PASS: haiku: modeled cost -30.0% on 1 rows; "), text)  # u0 at medium: +0.0%
        report = analyze.report(corpus, verdicts, None, ratios, ab)
        self.assertIn("| top | claude-opus-5-5 | 1 | 1,000,000 |", report)

    def test_g3_pools_models_in_dollars(self):
        """G3 pools models at their input prices (R16): opus at -20% outweighs sonnet at +20% on equal units."""
        turn = {"slice": "top", "seq": 0, "level": "medium", "cost_units": 1_000_000.0, "first_read": 0, "gap_ms": None}
        corpus = [dict(turn, id="o0", session_id="s1", model="claude-opus-5-5")]
        corpus.append(dict(turn, id="n0", session_id="s2", model="claude-code/claude-sonnet-5-5"))
        picks = [verdict_row("o0", "haiku", "low"), verdict_row("n0", "haiku", "high")]
        verdicts = keyed([*picks, verdict_row("ab-claude-opus-5-5-a", "haiku", "medium")])
        ratios = {"claude-opus-5-5": {"low": 0.8, "medium": 1.0}, "claude-sonnet-5-5": {"medium": 1.0, "high": 1.2}}
        ab = [{"model": "claude-opus-5-5", "level": "medium", "task": "a", "run": 0, "pass": True}]
        _, text = analyze.g3("haiku", corpus, verdicts, set(), ratios, ab)
        pooled = (-0.2 * 4.0 + 0.2 * 2.0) / (4.0 + 2.0)  # -6.67%, where pooling units would give 0%
        self.assertTrue(text.startswith(f"FAIL: haiku: modeled cost {pooled:+.1%} on 2 rows; "), text)
        report = analyze.report(corpus, verdicts, None, ratios, ab)
        self.assertIn("| haiku | claude-opus-5-5 | 1 | 1 | $4.00 | $3.20 | -20.0% |", report)
        self.assertIn("| haiku | claude-sonnet-5-5 | 1 | 1 | $2.00 | $2.40 | +20.0% |", report)
        self.assertIn("Jev priced at Haiku 5.5 input rate (assumption)", report)

    def test_report_skips_missing_inputs(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        out = Path(tmp.name)

        def report() -> str:
            with redirect_stdout(io.StringIO()):
                analyze.main([str(out)])
            return (out / "report.md").read_text(encoding="utf-8")

        self.assertIn("Skipped: no verdicts.jsonl yet", report())
        turn = {"slice": "top", "session_id": "s1", "model": "m", "level": None, "cost_units": 1.0, "first_read": 0}
        prompts = [dict(turn, id="t0", seq=0, gap_ms=None), dict(turn, id="t1", seq=1, gap_ms=1_000)]
        common.write_jsonl(out / "prompts.jsonl", prompts)
        stale = verdict_row("t0", "always-medium", "low")  # R5: the later row for (t0, always-medium) wins
        verdicts = [stale, *(verdict_row(rid, "always-medium", "medium") for rid in ("t0", "t1", "eff-h15"))]
        common.write_jsonl(out / "verdicts.jsonl", verdicts)
        text = report()
        self.assertIn("No labels for: top.", text)
        self.assertIn("| always-medium | top | 2 | 0 | – | – | – | 0.0% | 0 | 0 | 0 |", text)
        self.assertIn("- G1 INCOMPLETE: labels cover 0/2 top rows (need 90%). ", text)
        self.assertIn("always-medium (baseline) right 0.0% low 100.0%", text)  # eff-h15 wants high
        self.assertIn("- G2 INCOMPLETE: ", text)
        self.assertIn("- G3 INCOMPLETE: no judge arm passes G1; no ratios.json; no ab.jsonl", text)
        self.assertIn("| always-medium | 0 | 0 |", text)  # no flips: the stale low row for t0 is gone
        self.assertIn("Skipped: no ratios.json yet", text)
        common.write_jsonl(out / "labels.jsonl", [{"id": rid, "ok": ["low"], "agreed": True} for rid in ("t0", "t1")])
        text = report()
        self.assertNotIn("No labels for", text)
        self.assertIn("- G1 FAIL: no judge arm passes. ", text)
        self.assertIn("| always-medium | top | 2 | 2 | 0.0% | 0.0% | 100.0% |", text)


class CacheProbeTest(unittest.TestCase):
    def test_first_step_usage_pick(self):
        a, b, c = ({"input": 10, "output": 5, "cache_read": n, "cache_write": 0} for n in (1000, 2000, 3000))
        result = {"role": "function_result", "content": [{"type": "text", "text": "42"}], "timestamp": 20}
        lines = [
            meta("s1", "e2e"),
            user("u1", "do X", 0),
            *assistant("a1", 10, usage=a),
            entry("append", "f1", 0, 20, result),
            *assistant("a2", 30, "done", b),
            user("u2", "do Y", 40),
            *assistant("a3", 50, "ok", c),
        ]
        self.assertEqual(cache_probe.first_step_usage(lines, 1), c)
        self.assertEqual(cache_probe.first_step_usage(lines, 0), a)
        self.assertEqual(cache_probe.first_step_usage(lines[:-1], 1), {})  # a3 is still empty: the caller re-reads
        self.assertEqual(cache_probe.first_step_usage(lines, 2), {})  # no third turn yet

    def test_retention_ratio(self):
        self.assertEqual(cache_probe.retention(20000, 19000), 0.95)
        self.assertEqual(cache_probe.retention(0, 5), 0.0)
        self.assertEqual(cache_probe.retention(4_000 + 16_000, 4_000), 0.2)  # a flip that reads the system prefix only

    def test_wait_rides_out_two_status_failures_in_a_row(self):
        down = common.TriggerError("transport", "no engine")
        done = {"session_id": "s1", "turn_id": "t1", "status": "completed"}
        running = dict(done, status="running")
        with mock.patch.object(cache_probe.time, "sleep"):
            with mock.patch.object(common, "trigger", side_effect=[down, down, done]):
                self.assertIsNone(cache_probe.wait("s1", "t1"))
            with mock.patch.object(common, "trigger", side_effect=[down, running, down, down, down]) as call:
                with self.assertRaises(common.TriggerError):
                    cache_probe.wait("s1", "t1")
        self.assertEqual(call.call_count, 5)  # a reply in between starts the count again

    def test_dry_run_then_a_row_per_pair(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        out, data = Path(tmp.name) / "out", Path(tmp.name) / "data"
        (data / "session-manager").mkdir(parents=True)
        # each turn's first step: the history caches the system prefix only, so the control reads that and writes the
        # history, which a flip that keeps the cache then reads
        usages = {
            1: {"cache_write": 4_000},
            2: {"cache_read": 4_000, "cache_write": 16_000},
            3: {"cache_read": 19_000, "cache_write": 1_000},
            4: {"cache_read": 19_600, "cache_write": 500},
        }
        sends, turns, polls, failing = [], {}, {}, {"m-fail"}  # m-fail's first flip fails

        def stack(fn: str, payload: dict, timeout_s: float) -> dict:
            """A send writes its turn to the session file; status shows the turn before, then running, then the end."""
            if fn == "harness::send":
                if payload["model"] == "m-down":
                    raise common.TriggerError("transport", "no engine")
                sends.append(payload)
                sid = payload.get("session_id") or f"s{len(turns)}"
                n = turns[sid] = turns.get(sid, 0) + 1
                polls[sid] = 0
                written = [user(f"u{n}", "msg", n), *assistant(f"a{n}", n, "ok", usages[n])]
                with open(data / "session-manager" / f"{sid}.jsonl", "a", encoding="utf-8") as f:
                    f.writelines(line(d) + "\n" for d in written)
                return {"accepted": True, "session_id": sid, "turn_id": f"t{n}"}
            sid = payload["session_id"]
            n = turns[sid]
            polls[sid] += 1
            if polls[sid] == 1:  # the turn before is still the latest
                return {"session_id": sid, "turn_id": f"t{n - 1}", "status": "completed"}
            if polls[sid] == 2:
                return {"session_id": sid, "turn_id": f"t{n}", "status": "running"}
            if sends[-1]["model"] in failing and n == 3:
                failing.clear()
                return {"session_id": sid, "turn_id": "t3", "status": "failed", "result_error": "overloaded"}
            return {"session_id": sid, "turn_id": f"t{n}", "status": "completed"}

        def probe_main(*args: str) -> list[str]:
            printed = io.StringIO()
            with redirect_stdout(printed):
                cache_probe.main([str(out), "--data", str(data), *args])
            return printed.getvalue().splitlines()

        with mock.patch.object(common, "trigger", side_effect=stack) as call:
            dry, dry_openai = probe_main(), probe_main("--with-openai")
            self.assertEqual(call.call_count, 0)
            models = {"m-ok": "anthropic", "m-fail": "anthropic", "m-down": "anthropic", "m-late": "anthropic"}
            with (
                mock.patch.dict(cache_probe.MODELS, models, clear=True),
                mock.patch.object(cache_probe.time, "sleep") as sleep,
            ):
                with self.assertRaises(SystemExit) as stop:
                    probe_main("--run")
                resume = probe_main()
        self.assertTrue(dry[0].startswith("4 turns × 3 pairs × 3 models = 36 turns, 9 pairs still to run"), dry[0])
        self.assertTrue(dry_openai[0].startswith("4 turns × 3 pairs × 4 models = 48 turns"), dry_openai[0])
        self.assertEqual([p.split(":")[0] for p in dry_openai[1:]], ["anthropic", "openai"])  # never summed
        history, *follow_ups = sends[:4]
        text = history.pop("message")
        self.assertTrue(text.startswith("Summarise each file below in three lines.\n\n### harness/src/judge.rs\n"))
        self.assertIn("\n\n### harness/src/functions/send.rs\n", text)
        self.assertLess(len(json.dumps(dict(history, message=text))), 131_072)  # one exec argument
        want = {
            "model": "m-ok",
            "provider": "anthropic",
            "session": {"kind": "e2e"},
            "options": {"thinking_level": "medium"},
        }
        self.assertEqual(history, want)  # no functions: a plain chat loop
        got = [(s["session_id"], s["options"]["thinking_level"], s["message"]) for s in follow_ups]
        want = [
            ("s0", "medium", "List the public functions in judge.rs above."),
            ("s0", "low", "List the public functions in send.rs above."),
            ("s0", "medium", "Which of those functions touch thinking_level?"),
        ]
        self.assertEqual(got, want)
        # m-fail's first pair stops at the flip; m-down's three invalid pairs in a row stop the run before m-late
        self.assertEqual((len(sends), "session_id" in sends[4]), (3 * 4 + 3 + 2 * 4, False))
        self.assertIn("stopped: 3 pairs in a row invalid (turn 1: transport: no engine)", str(stop.exception))
        self.assertEqual({c.args for c in sleep.call_args_list}, {(2,)})  # status every 2 s; usage was there at once
        rows = common.read_jsonl(out / "cache.jsonl")
        keys = ["model", "pair", "control_read", "control_write", "flip_read", "back_read", "retention"]
        self.assertEqual(list(rows[0]), [*keys, "retention_back", "invalid"])
        valid = {"model": "m-ok", "control_read": 4_000, "control_write": 16_000, "flip_read": 19_000}
        valid |= {"back_read": 19_600, "retention": 0.95, "retention_back": 0.98, "invalid": None}  # over 20,000
        failed = dict(valid, model="m-fail", flip_read=None, back_read=None, retention=None, retention_back=None)
        failed["invalid"] = "turn 3: failed (overloaded)"
        down = dict(failed, model="m-down", control_read=None, control_write=None)
        down["invalid"] = "turn 1: transport: no engine"
        want = [dict(valid, pair=k) for k in range(3)] + [dict(failed, pair=0)]
        want += [dict(valid, model="m-fail", pair=k) for k in (1, 2)] + [dict(down, pair=k) for k in range(3)]
        self.assertEqual(rows, want)
        # the next --run skips the valid pairs: m-fail's first, m-down's and m-late's are left
        self.assertIn("48 turns, 7 pairs still to run", resume[0])


class AbTest(unittest.TestCase):
    def test_ratios_relative_to_medium(self):
        runs = [("m", "low", 60, True), ("m", "low", 70, False), ("m", "low", 80, False)]  # failing runs count alike
        runs += [("m", "medium", 100, True), ("m", "high", 130, False), ("n", "low", 50, True)]  # n has no medium
        rows = [{"model": m, "level": lv, "cost_units": c, "pass": p, "error": None} for m, lv, c, p in runs]
        rows.append({"model": "m", "level": "high", "cost_units": 900, "pass": False, "error": "failed (overloaded)"})
        self.assertEqual(ab.ratios(rows), {"m": {"low": 0.7, "medium": 1.0, "high": 1.3}})

    def test_check_lookup(self):
        self.assertTrue(ab.check("a", "it returns None", Path()))
        self.assertFalse(ab.check("a", "it returns the key", Path()))

    def test_check_investigation(self):
        both = "send.rs:408 asks reasoning_fields_omitted, then send.rs:411 calls inherit_prior_reasoning"
        self.assertTrue(ab.check("d", both, Path()))
        self.assertFalse(ab.check("d", "send.rs:1453 inherit_prior_reasoning", Path()))
        self.assertFalse(ab.check("d", "send.rs:1445 reasoning_fields_omitted", Path()))

    def test_plant_then_check_fix(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        repo = Path(tmp.name) / "repo"  # plant writes repo.test-sha256 beside it
        pkg = repo / ab.PKG
        shutil.copytree(HERE, pkg, ignore=shutil.ignore_patterns("__pycache__"))  # the test module imports them all
        ab.plant("a", repo)
        self.assertEqual((pkg / "analyze.py").read_bytes(), (HERE / "analyze.py").read_bytes())
        self.assertFalse(Path(f"{repo}.test-sha256").exists())
        ab.plant("c", repo)
        self.assertFalse(ab.check("c", "", repo))  # test_p95 fails
        shutil.copy(HERE / "analyze.py", pkg / "analyze.py")
        self.assertTrue(ab.check("c", "", repo))
        with open(pkg / "test_auto_effort.py", "a", encoding="utf-8") as f:
            f.write("\n")
        self.assertFalse(ab.check("c", "", repo))  # the tests pass, but the test file changed

    def test_a_turn_that_does_not_end_is_stopped(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        sessions = Path(tmp.name)
        usage = {"input": 10, "output": 1}
        written = [meta("s1", "e2e"), user("u1", "do X", 0), *assistant("a1", 10, usage=usage)]
        (sessions / "s1.jsonl").write_text("".join(line(d) + "\n" for d in written), encoding="utf-8")
        replies = {
            "harness::send": {"accepted": True, "session_id": "s1", "turn_id": "t1"},
            "harness::status": {"session_id": "s1", "turn_id": "t1", "status": "running"},
            "harness::stop": {"stopping": True},
        }
        with (
            mock.patch.object(common, "trigger", side_effect=lambda fn, payload, timeout_s: replies[fn]) as call,
            mock.patch.object(ab, "TURN_S", 0),
        ):
            steps, why = ab.converse({"message": "do X"}, sessions)
        self.assertEqual(why, "not ended after 0 s")
        self.assertEqual(call.call_args.args[:2], ("harness::stop", {"session_id": "s1", "turn_id": "t1"}))
        self.assertEqual([m["usage"] for m in steps], [usage])  # what it ran up still counts

        def interrupted(fn: str, payload: dict, timeout_s: float) -> dict:
            """A Ctrl-C while the runner waits on the turn."""
            if fn == "harness::status":
                raise KeyboardInterrupt
            return replies[fn]

        with (
            mock.patch.object(common, "trigger", side_effect=interrupted) as call,
            self.assertRaises(KeyboardInterrupt),
        ):
            ab.converse({"message": "do X"}, sessions)
        self.assertEqual(call.call_args.args[:2], ("harness::stop", {"session_id": "s1", "turn_id": "t1"}))
        down = common.TriggerError("transport", "no engine")
        with mock.patch.object(common, "trigger", side_effect=down) as call:
            self.assertEqual(ab.converse({"message": "do X"}, sessions), ([], "transport: no engine"))
        self.assertEqual(call.call_count, 1)  # no turn, nothing to stop

    def test_cost_waits_for_the_final_usage(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "s1.jsonl"
        empty = {"role": "assistant", "content": [], "timestamp": 10}
        written = [meta("s1", "e2e"), user("u1", "do X", 0), entry("append", "a1", 0, 10, empty)]
        path.write_text("".join(line(d) + "\n" for d in written), encoding="utf-8")
        usage = {"input": 10, "output": 1}
        final = entry("entry", "a1", 1, 10, dict(empty, content=[{"type": "text", "text": "done"}], usage=usage))

        def late(seconds: float) -> None:
            """The final revision lands while the runner waits."""
            with open(path, "a", encoding="utf-8") as f:
                f.write(line(final) + "\n")

        replies = {
            "harness::send": {"accepted": True, "session_id": "s1", "turn_id": "t1"},
            "harness::status": {"session_id": "s1", "turn_id": "t1", "status": "completed"},
        }
        with (
            mock.patch.object(common, "trigger", side_effect=lambda fn, payload, timeout_s: replies[fn]),
            mock.patch.object(cache_probe.time, "sleep", side_effect=late) as sleep,
        ):
            steps, why = ab.converse({"message": "do X"}, path.parent)
        self.assertEqual((why, [m["usage"] for m in steps], sleep.call_args_list), (None, [usage], [mock.call(0.5)]))

    def test_pilot_run_then_resume(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        out, sessions = Path(tmp.name) / "out", Path(tmp.name) / "data" / "session-manager"
        sessions.mkdir(parents=True)
        inputs = {"low": 700, "medium": 1000, "high": 1300}
        answers = {"low": "It returns the key.", "medium": "It returns None.", "high": "It returns None."}
        sends, stops, calls, failing = [], [], [], {"high"}  # high's first turn fails; its retry completes

        def stack(fn: str, payload: dict, timeout_s: float) -> dict:
            """A send writes its session file: one function step, then the answer unless the turn fails."""
            if fn == "harness::stop":
                stops.append(payload)
                return {"stopping": False}
            if fn == "harness::send":
                sends.append(payload)
                sid, level = f"s{len(sends)}", payload["options"]["thinking_level"]
                usage = {"input": inputs[level], "output": 0, "cache_read": 0, "cache_write": 0}
                steps = assistant("a1", 10, usage=usage)
                if level not in failing:
                    steps += assistant("a2", 20, answers[level], usage)
                written = [meta(sid, "e2e"), user("u1", payload["message"], 0), *steps]
                (sessions / f"{sid}.jsonl").write_text("".join(line(d) + "\n" for d in written), encoding="utf-8")
                return {"accepted": True, "session_id": sid, "turn_id": "t1"}
            sid = payload["session_id"]
            if sends[int(sid[1:]) - 1]["options"]["thinking_level"] in failing:
                failing.clear()
                return {"session_id": sid, "turn_id": "t1", "status": "failed", "result_error": "overloaded"}
            return {"session_id": sid, "turn_id": "t1", "status": "completed"}

        def git(*args: str) -> str:
            calls.append(args)
            if args[0] == "rev-parse":
                return "f" * 40 + "\n"
            if args[0] == "cat-file":  # the sha predates ab.py
                raise subprocess.CalledProcessError(128, args)
            if args[1] == "add":
                Path(args[3]).mkdir(parents=True)
            else:
                Path(args[3]).rmdir()
            return ""

        def ab_main(*args: str) -> list[str]:
            printed = io.StringIO()
            with redirect_stdout(printed):
                ab.main([os.path.relpath(out), "--sha", "HEAD", *args])  # the worktrees still get absolute paths
            return printed.getvalue().splitlines()

        data = ("--data", str(sessions.parent))
        with (
            mock.patch.object(common, "trigger", side_effect=stack) as call,
            mock.patch.object(ab, "git", side_effect=git),
        ):
            pilot, full = ab_main("--pilot"), ab_main()
            self.assertEqual(call.call_count, 0)
            with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                ab_main("--pilot", "--run")  # no --data
            self.assertEqual(call.call_count, 0)
            ab_main("--pilot", "--run", *data)
            first = common.read_jsonl(out / "ab.jsonl")
            first_ratios = json.loads((out / "ratios.json").read_text(encoding="utf-8"))
            ab_main("--pilot", "--run", *data)  # only the run that errored goes again
            after = ab_main()
        with (
            mock.patch.object(ab, "git", return_value="f" * 40),  # cat-file finds ab.py in the sha's tree
            redirect_stderr(io.StringIO()) as refused,
            self.assertRaises(SystemExit),
        ):
            ab.main([str(out), "--sha", "HEAD"])
        self.assertIn("holds ab.py", refused.getvalue())
        self.assertEqual(pilot[0], f"3 runs at {'f' * 12}, 3 still to run")
        self.assertEqual(full[0], f"72 runs at {'f' * 12}, 72 still to run")
        self.assertEqual([p.split(":")[0] for p in full[1:]], list(ab.MODELS))  # units per model, never summed
        self.assertEqual(after[0], f"72 runs at {'f' * 12}, 69 still to run")  # the pilot's runs are the first three
        self.assertEqual(after[1], "claude-opus-5-5: 33 runs, about 3,300,000 units if a run costs about 100,000")
        trees = [out.resolve() / "worktrees" / f"ab-claude-opus-5-5-{level}-a-0" for level in ab.LEVELS]
        allow = ["coder::read-file", "coder::search", "coder::tree", "coder::list-folder", "coder::update-file"]
        allow += ["shell::exec", "engine::functions::info"]  # file read and edit, the tests, and contracts
        options = {
            "thinking_level": "low",
            "functions": {"allow": allow},
            "metadata": {"fs_scope": {"root": str(trees[0])}},
        }
        want = {
            "message": "What does `confident_choice` in harness/src/reconcile.rs return when the choice is `none`?",
            "model": "claude-opus-5-5",
            "provider": "anthropic",
            "session": {"kind": "e2e"},
            "options": options,
        }
        self.assertEqual(sends[0], want)
        self.assertEqual([s["options"]["thinking_level"] for s in sends], ["low", "medium", "high", "high"])
        worktree = [c for c in calls if c[0] == "worktree"]
        lifecycle = [
            (("worktree", "add", "--detach", str(t), "f" * 40), ("worktree", "remove", "--force", str(t)))
            for t in [*trees, trees[2]]
        ]
        self.assertEqual(worktree, [c for pair in lifecycle for c in pair])  # each removed before the next is added
        self.assertEqual(stops, [{"session_id": "s3", "turn_id": "t1"}])  # the failed turn
        keys = ["model", "level", "task", "run", "pass", "cost_units", "steps", "error"]
        self.assertEqual(list(first[0]), keys)
        got = [(r["level"], r["pass"], r["cost_units"], r["steps"], r["error"]) for r in first]
        want = [
            ("low", False, 1400.0, 2, None),
            ("medium", True, 2000.0, 2, None),
            ("high", False, 1300.0, 1, "failed (overloaded)"),
        ]
        self.assertEqual(got, want)
        self.assertEqual(first_ratios, {"claude-opus-5-5": {"low": 0.7, "medium": 1.0}})  # no cost from an error
        retried = [(r["level"], r["pass"], r["error"]) for r in common.read_jsonl(out / "ab.jsonl")]
        self.assertEqual(retried, [("low", False, None), ("medium", True, None), ("high", True, None)])
        ratios = json.loads((out / "ratios.json").read_text(encoding="utf-8"))
        self.assertEqual(ratios, {"claude-opus-5-5": {"low": 0.7, "medium": 1.0, "high": 1.3}})
        prompts = common.read_jsonl(out / "ab_prompts.jsonl")
        self.assertEqual([p["id"] for p in prompts], [f"ab-{m}-{t}" for m in ab.MODELS for t in "abcd"])
        want = {"id": "ab-claude-opus-5-5-a", "slice": "ab", "model": "claude-opus-5-5", "level": "medium"}
        self.assertEqual(prompts[0], dict(want, context="", message=ab.TASKS["a"]))
        down = common.TriggerError("transport", "no engine")
        with (
            mock.patch.object(common, "trigger", side_effect=down) as call,
            mock.patch.object(ab, "git", side_effect=git),
            redirect_stdout(io.StringIO()),
            self.assertRaises(SystemExit) as stop,
        ):
            ab.main([str(out), "--sha", "HEAD", "--run", *data])  # the full run on a stack that is down
        self.assertIn("stopped: 3 runs in a row errored (transport: no engine)", str(stop.exception))
        self.assertEqual(call.call_count, 3)  # one send a run, and no turn to stop
        self.assertEqual(len(common.read_jsonl(out / "ab.jsonl")), 3 + 3)  # the pilot's runs, then the errored ones
