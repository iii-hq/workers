import json
from pathlib import Path
import re
import unittest


KIT = Path(__file__).resolve().parents[1]


class ControlPlaneContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.orchestrator = (KIT / "agents/deepwork.md").read_text()
        cls.active_docs = "\n".join(
            path.read_text() for path in [KIT / "README.md", *sorted((KIT / "agents").glob("*.md"))]
        )

    def test_questions_use_durable_metadata_and_expiry(self):
        asks = [json.JSONDecoder().raw_decode(self.orchestrator[match.end():])[0]
                for match in re.finditer(r"harness::ask (?=\{)", self.orchestrator)]
        self.assertEqual({ask["questions"][0]["header"] for ask in asks},
                         {"Plan", "Run health", "Finish"})
        for ask in asks:
            with self.subTest(header=ask["questions"][0]["header"]):
                self.assertEqual(set(ask["about"]), {"ticket", "blocks", "class", "why"})
                self.assertEqual(ask["about"]["ticket"], "<R>")
                self.assertIsInstance(ask["about"]["blocks"], list)
                self.assertTrue(ask["about"]["why"].strip())
                for question in ask["questions"]:
                    self.assertIn(len(question["options"]), range(2, 5))
                    self.assertEqual(sum(option.get("recommended", False)
                                         for option in question["options"]), 1)
                if ask["about"]["class"] == "run_health":
                    self.assertEqual(ask["expires_in_min"], 30)
                    self.assertEqual(ask["on_expiry"], {"answer": "retry only the unfinished checks"})
                    self.assertEqual([option["label"] for option in ask["questions"][0]["options"]],
                                     ["retry only the unfinished checks",
                                      "retry with a longer deadline", "stop the run"])
                    self.assertTrue(ask["questions"][0]["options"][0]["recommended"])
                else:
                    self.assertEqual(ask["expires_in_min"], 4320)
                    self.assertEqual(ask["on_expiry"], "block")

    def test_active_docs_do_not_restore_board_protocol_or_question_pills(self):
        for retired in ("ask=board", "ask_wait_min", "board question", "board answer"):
            self.assertNotIn(retired, self.active_docs.lower())
        self.assertNotRegex(self.active_docs, r'"text"\s*:\s*"needs you"')
        for function in ("harness::ask::withdraw", "harness::link-card", "kanban::question::record"):
            self.assertIn(f"  - {function}\n", self.orchestrator)
            self.assertIn(f"`{function}`", (KIT / "README.md").read_text())

    def test_assumption_and_link_payloads_match_worker_contracts(self):
        records = [json.JSONDecoder().raw_decode(self.orchestrator[match.end():])[0]
                   for match in re.finditer(r"kanban::question::record (?=\{)", self.orchestrator)]
        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(set(record), {"ticket_id", "questions", "answer_text", "recorded_by", "about"})
        self.assertEqual(record["ticket_id"], "<R>")
        self.assertEqual(record["recorded_by"], "deepwork")
        self.assertTrue(record["answer_text"].strip())
        self.assertTrue(record["questions"][0]["question"].strip())
        links = [json.JSONDecoder().raw_decode(self.orchestrator[match.end():])[0]
                 for match in re.finditer(r"harness::link-card (?=\{)", self.orchestrator)]
        self.assertEqual(links, [{"ticket": "<R>"}, {"ticket": "<R>"}])

    def test_resume_checks_the_original_roots_questions_before_restarting(self):
        resume = self.orchestrator.split("## Step A-R:", 1)[1]
        guard = resume.split("\n0.", 1)[0]
        self.assertIn("Orchestrator session:", guard)
        self.assertIn("other than your own", guard)
        self.assertIn('harness::status {"session_id":"<old>"}', guard)
        self.assertIn("`open_asks` above 0", guard)
        self.assertIn("stop without changing R, its pointer or its questions", guard)


if __name__ == "__main__":
    unittest.main()
