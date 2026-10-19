"""Unit tests for the auto-effort phase 0 scripts. Synthetic lines only: corpus text never enters the repo."""

import hashlib
import io
import json
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

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


def user(eid: str, text: str, ts: int) -> dict:
    return entry("append", eid, 0, ts, {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": ts})


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

    def test_notification_ends_cost_window(self):
        u1 = {"input": 100, "output": 10, "cache_read": 0, "cache_write": 0}
        u2 = {"input": 7, "output": 1, "cache_read": 0, "cache_write": 0}
        lines = [
            user("u1", "do X", 0),
            *assistant("a1", 10, "done", u1),
            user("u2", "[notification] wake", 20),
            *assistant("a2", 30, "woke", u2),
        ]
        self.write("s1", lines)
        self.assertEqual([(r["message"], r["cost_units"]) for r in self.rows()], [("do X", common.units(u1))])


class TriggerTest(unittest.TestCase):
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

    def test_run_logs_misses_and_stops_after_five_in_a_row(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        out = Path(tmp.name)
        rows = [{"id": f"p{i}", "model": "m", "context": "", "message": "x"} for i in range(8)]
        common.write_jsonl(out / "prompts.jsonl", rows)
        timeout, ok, prose = common.TriggerError("timeout", "too slow"), reply('{"ok":["low"]}'), reply("y" * 300)
        failed = {"message": {"content": [], "stop_reason": "error", "error_message": "rate limited"}}
        # p0 throws, p1 is labelled; then p2 throws and p3..p6 get no usable reply: five misses in a row
        replies = [timeout, ok, ok, timeout, failed, *[prose] * 11]
        log = io.StringIO()
        with mock.patch.object(common, "trigger", side_effect=replies) as call, redirect_stderr(log):
            with self.assertRaises(SystemExit) as stop:
                label.main([str(out), "--run"])
        self.assertEqual(call.call_count, len(replies))  # p7 is never asked
        self.assertEqual(common.read_jsonl(out / "labels.jsonl"), [{"id": "p1", "ok": ["low"], "agreed": True}])
        self.assertIn("5 prompts in a row", str(stop.exception))
        lines = log.getvalue().splitlines()
        self.assertIn("p0: timeout: too slow", lines)
        self.assertIn("p6: no label", lines)
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

    def test_dry_run_then_stop_after_five_failed_calls_in_a_row(self):
        rows = [dict(self.row, id=f"r{i}") for i in range(12)]
        rows[7]["message"] = "ok"  # a follow-up makes no call, so it neither counts nor resets
        common.write_jsonl(self.out / "rows.jsonl", rows)
        path = self.out / "verdicts.jsonl"
        common.write_jsonl(path, [{"id": "r0", "arm": "typesafe", "level": "low"}])
        down = common.TriggerError("transport", "no engine")
        replies = [down] * 4 + [judge_reply("r4", "low", 0.9)] + [down] * 5
        with mock.patch.object(common, "trigger", side_effect=replies) as call:
            printed = self.judge_main("--arm", "typesafe", "keep", "--rows", str(self.out / "rows.jsonl"))
            self.assertEqual(call.call_count, 0)
            with self.assertRaises(SystemExit) as stop:
                self.judge_main("--arm", "typesafe", "--rows", str(self.out / "rows.jsonl"), "--run")
        counts = [p.split(", about")[0] for p in printed]
        self.assertEqual(counts, ["typesafe: 12 rows, 11 calls", "keep: 12 rows, 0 calls"])
        # r0-r3 fail, r4 answers, then r5, r6, r8, r9 and r10 fail; r11 is never asked
        self.assertEqual(call.call_count, 10)
        self.assertIn("5 calls in a row", str(stop.exception))
        self.assertEqual(common.read_jsonl(path), [{"id": "r0", "arm": "typesafe", "level": "low"}])
