import json
from pathlib import Path
import re
import unittest


KIT = Path(__file__).resolve().parents[1]
NEW_FUNCTIONS = ("harness::ask::withdraw", "harness::ask::list", "harness::link-card",
                 "harness::status", "kanban::question::record")


class ProfileContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.profile = (KIT / "agents/workflow.md").read_text()
        cls.verifier = (KIT / "agents/workflow-verifier.md").read_text()
        cls.readme = (KIT / "README.md").read_text()
        cls.manifest = (KIT / "iii.kit.yaml").read_text()
        dl = cls.profile.split("Lead deny list DL:", 1)[1].split("```json\n", 1)[1]
        cls.dl = json.loads(dl.split("\n", 1)[0])

    def ask(self, header):
        """The ask's text, from its JSON to the backtick that closes its code span."""
        self.assertTrue(f'"header":"{header}"' in self.profile, f"no {header} ask")
        at = self.profile.index(f'"header":"{header}"')
        start = self.profile.rindex("harness::ask {", 0, at) + len("harness::ask ")
        self.assertNotIn("`", self.profile[start:at], f"{header} is not inside a harness::ask")
        return self.profile[start:self.profile.index("`", at)]

    def assertHasAll(self, text, parts):
        # Names only the missing parts: a plain assertIn would print the whole document.
        self.assertEqual([part for part in parts if part not in text], [])

    def test_frontmatter_preloads_the_new_calls_and_drops_state_set(self):
        lines = self.profile.split("\n---\n", 1)[0].splitlines()
        self.assertHasAll(lines, [f"  - {function}"
                                  for function in NEW_FUNCTIONS + ("state::compare-and-set",)])
        self.assertEqual(lines.count("  - state::set"), 0)

    def test_no_recommended_label_and_state_set_only_denied(self):
        self.assertEqual(self.profile.count("(Recommended)\""), 0)
        self.assertEqual(self.profile.count("state::set"), 1)
        self.assertIn("state::set", self.dl)

    def test_run_health_card_decodes_with_its_expiry_and_options(self):
        ask = json.JSONDecoder().raw_decode(self.ask("Run health"))[0]
        self.assertEqual(set(ask["about"]), {"ticket", "blocks", "class", "why"})
        self.assertEqual(ask["about"]["class"], "run_health")
        self.assertEqual(ask["expires_in_min"], 30)
        self.assertEqual(ask["on_expiry"], {"answer": "retry only the unfinished checks"})
        options = ask["questions"][0]["options"]
        self.assertEqual([option["label"] for option in options],
                         ["retry only the unfinished checks", "retry with a longer deadline",
                          "stop the run"])
        self.assertEqual([option.get("recommended", False) for option in options],
                         [True, False, False])

    def test_plan_and_finish_cards_block_on_expiry_with_a_recommended_option(self):
        for header in ("Plan", "Finish"):
            with self.subTest(header=header):
                self.assertHasAll(self.ask(header), ['"expires_in_min":4320,"on_expiry":"block"',
                                                     '"recommended":true'])

    def test_nodes_cannot_link_cards_or_record_questions(self):
        self.assertHasAll(self.dl, ["harness::link-card", "kanban::question::record"])

    def test_pointer_kinds_and_dispatcher_line(self):
        self.assertIsNotNone(re.search(r'"kind":<[^>]*"recheck"\|"wait"', self.profile),
                             'no "recheck"|"wait" in the kind list')
        self.assertHasAll({line.strip() for line in self.profile.splitlines()},
                          ["Orchestrator session: <own>"])

    def test_manifest_raises_the_worker_floors(self):
        self.assertHasAll([line.strip() for line in self.manifest.splitlines()],
                          ["version: 0.1.14", "harness: ^1.10.0", "kanban: ^0.2.7"])

    def test_readme_names_the_new_calls_and_no_longer_leaves_out_ci(self):
        self.assertHasAll(self.readme, [f"`{function}`" for function in NEW_FUNCTIONS])
        self.assertEqual(self.readme.count("watching CI"), 0)

    def test_verifier_writes_the_check_line_forms(self):
        self.assertHasAll(self.verifier, ["env: <cause>", "flaky: <test>", "skipped: <command>",
                                          "reused: <line>", "matches too much",
                                          "is a criterion of its card", "decides nothing"])


    def test_worker_and_verifier_load_a_named_skill(self):
        call = ['directory::skills::get {"id":"<id>"}']
        self.assertHasAll((KIT / "agents/workflow-worker.md").read_text(), call)
        self.assertHasAll(self.verifier, call)
        self.assertNotIn("directory::skills::get", self.dl)

    def test_a_lost_pointer_write_reads_the_owner_and_the_resume_guard_fails_closed(self):
        self.assertHasAll(self.profile, [
            "Every document names its writer in `session`",
            '`{"item":"batch","session":"<own>","keys":[…]',
            '`{"item":"baseline","session":"<own>","reasons":[…]}`',
            "then the same for `batch` and `baseline` when they exist",
            "when the reply's `current` names another `session`, a resume took the run",
            "Otherwise, and before your first `active` of this run, send it once more",
            "A refused or failed read is no answer and blocks the same way",
            "only a `null` result (no such session) or another status lets the resume go on"])
        self.assertEqual(self.profile.count("a refused or failed read is neither"), 0)

    def test_land_never_reads_missing_checks_as_green(self):
        self.assertHasAll(self.profile, ["Output saying no checks were reported is never green",
                                         "CI: no checks reported, PR open"])
        self.assertEqual(self.profile.count("statusCheckRollup"), 0)
        self.assertEqual(self.profile.count("(the repository runs none: green)"), 0)

    def test_listener_sweep_stops_only_what_a_job_of_the_run_started(self):
        mark = '"env":{"WF_JOB":"<Worktree>"}'
        self.assertHasAll((KIT / "agents/workflow-worker.md").read_text(), [mark])
        self.assertHasAll(self.verifier, [mark,
            "/proc/$p/environ 2>/dev/null | grep -qxF \"WF_JOB=$W\" && kill $p", "done; true`"])
        self.assertEqual(self.verifier.count("/proc/$p/stat"), 0)
        self.assertEqual(self.verifier.count("readlink /proc/$p/cwd"), 0)

if __name__ == "__main__":
    unittest.main()
