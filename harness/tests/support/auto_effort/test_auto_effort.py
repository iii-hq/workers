"""Unit tests for the auto-effort phase 0 scripts. Synthetic lines only: corpus text never enters the repo."""

import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import common
import extract


def line(d: dict) -> str:
    return json.dumps(d)


def meta(sid: str, **metadata) -> dict:
    return {"type": "meta", "meta": {"session_id": sid, "kind": "user", "metadata": metadata}}


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


class ExtractTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name) / "src" / "data" / "session-manager"
        self.dir.mkdir(parents=True)

    def write(self, sid: str, lines: list[dict], **metadata) -> None:
        text = "".join(line(d) + "\n" for d in [meta(sid, **metadata), *lines])
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
