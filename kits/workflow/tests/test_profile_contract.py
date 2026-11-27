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
        cls.planner = (KIT / "agents/workflow-planner.md").read_text()
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
                          ["version: 0.1.16", "harness: ^1.10.0", "kanban: ^0.2.7"])

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

    def test_defaults_raise_the_round_cap_and_set_a_token_budget(self):
        defaults = self.profile.split("Defaults:\n`", 1)[1].split("`", 1)[0].split()
        self.assertHasAll(defaults, ["max_rounds=12", "budget_tokens=4000000"])
        self.assertHasAll(self.profile, ["`budget_tokens=` with no value turns it off",
                                         "the stop reason is `budget: <Tok> tokens of budget_tokens=<n>`"])
        self.assertHasAll(self.readme, ["| `max_rounds` | 12 |", "| `budget_tokens` | 4000000 |"])

    def test_moving_target_card_recommends_accepting_the_cards(self):
        ask = json.JSONDecoder().raw_decode(self.ask("Moving target"))[0]
        self.assertEqual(set(ask["about"]), {"ticket", "blocks", "class", "why"})
        self.assertEqual(ask["about"]["class"], "scope")
        self.assertEqual((ask["expires_in_min"], ask["on_expiry"]), (4320, "block"))
        options = ask["questions"][0]["options"]
        self.assertEqual([option["label"] for option in options],
                         ["accept the cards", "split the cards", "keep repairing", "stop the run"])
        self.assertEqual([option.get("recommended", False) for option in options],
                         [True, False, False, False])
        self.assertHasAll(self.profile, ['`"moving":<n>`', "ask Step E's Moving target card",
                                         "Step E's Moving target card (it takes its recommendation)",
                                         "`split the cards`: for each B card",
                                         "its few-tickets rule does not merge them back",
                                         "would keep only items whose Verify calls are project commands",
                                         "`moved` kept as stored",
                                         "also gets the new keys on that bullet",
                                         "A card whose every Acceptance sentence a reason breaks, or that",
                                         "Moved: <new KEY>: <the sentences it took>",
                                         '`"moved":["<new KEY>: <the sentences it took>",…]`',
                                         "or have none while it holds `moved` (a split left it to be checked again)",
                                         "The Moving target card's `split the cards` is such an answer"])
        self.assertHasAll(self.verifier, ["A `Moved: <KEY>: <sentences>` line in your task",
                                          "a finding that breaks only those criteria",
                                          "a `note: moved to <KEY>: …` line in `checks`, never a reason, and it concerns no Batch card"])

    def test_findings_never_rewrite_acceptance(self):
        self.assertHasAll(self.profile, [
            "**Acceptance is the person's.** Never add, widen or reword an existing card's Acceptance items",
            "from a finding of a check, the review or a refuter",
            "when an answer changes a card, write only what the answer says",
            "under the card's `## Scope` and nothing else",
            "becomes a subtask of its own (Step D; a finding never rewrites a card: Acceptance is the person's, Step A.8)"])
        self.assertEqual(self.profile.count("when it is that ticket's work (rewrite its card)"), 0)

    def test_each_acceptance_item_is_one_behaviour(self):
        rule = ["Each Acceptance item is one behaviour a person can check on its own",
                'never two joined by "and" or "both", with its own Verify calls that exercise that item and nothing else',
                "a one-liner that prints two functions' results becomes one call in each item",
                "An item states the input and the expected result, or the observable effect",
                'Words that decide nothing ("handles edge cases", "robust", "correct") are not an item',
                "neither is a run of the project's suite, build or lint: the batch check runs them"]
        for name, text in (("workflow", self.profile), ("planner", self.planner)):
            with self.subTest(profile=name):
                self.assertHasAll(text, rule)
        self.assertHasAll(self.profile, [
            "each behaviour it delivers is some card's Acceptance item",
            "Every card you write has this shape, here or mid-run (added work, Step D's new items)",
            "the cards `split the cards` makes copy their Acceptance sentences word for word, each followed by the Verify calls of the item it came from"])

    def test_cards_are_checked_once_before_their_first_dispatch(self):
        ask = json.JSONDecoder().raw_decode(self.ask("Edge cases"))[0]
        self.assertEqual((ask["about"]["class"], ask["expires_in_min"], ask["on_expiry"]),
                         ("scope", 4320, "block"))
        self.assertEqual([option.get("recommended", False) for option in ask["questions"][0]["options"]],
                         [True, False])
        self.assertIsNotNone(re.search(r'"kind":<"plan"\|"cards"\|', self.profile), "no cards kind")
        self.assertHasAll(self.profile, [
            "Every card is checked once, before its first dispatch",
            "more than four for the person go on the next Edge cases card",
            '"task":"Mode: cards. Run ticket <R>',
            "no comment by `workflow` on the card opens `**Card check**`, or `Split from`",
            "one comment, opening `Split from <KEY>.`, holds the reasons it took",
            "a card check never holds a card back",
            "A reason that would add a behaviour, widen the scope or change what the person decided",
            "Step C's card check is planning, not a refuter's finding",
            "once a card has been dispatched, this rule holds",
            "Step C's Edge cases card (it takes its recommendations)",
            "`cards` as Step C says, only for its cards with no comment by `workflow` that opens `**Card check**`",
            "so a resume or a continue finds it unchecked",
            "goes on the Edge cases card below, whatever its tag"])
        self.assertHasAll(self.verifier, [
            "**Cards** (the task says `Mode: cards`)",
            "Never propose a new behaviour, feature or scope",
            "at most five per card",
            "`<KEY>: input: item <n>: <input> → <expected result>: <person or orchestrator>: <why, one clause>`",
            "An input an Acceptance item decides is judged against that item",
            "in cards mode an input no Acceptance item decides is a reason, never a note"])

    def test_pull_requests_code_and_commits_name_no_ticket_key(self):
        self.assertHasAll(self.profile, [
            "The title is the commit a squash merge keeps: `<type>(<scope>): <what changed>`",
            "but with no ticket key (`feat(cli): add a --json output flag`)",
            "**The pull request's report**, the body of every `gh pr` call of item 6",
            "`- **<title>**: <outcome>, <one short clause>`",
            "no `[KEY]` citation; no `resume=<R>` line and no line that names the run ticket",
            "R's comment and the final message keep the keys",
            "**Ticket keys stay on the board.** Never name a ticket key",
            "is data, not a reference to the work: write it as the card gives it.",
            "when no `PR:` comment on R names its `url`, comment `PR: <its url>` on R",
            "in a card's **Commit** bullet (a Fix CI card's included)",
            "(subject only, no trailer, no ticket key)",
            "kebab-case, never a ticket key,",
            "`- **KAN-12** <title>: <outcome>, <one short clause>`",
            "then, as the last line, `resume=<R>`"])
        self.assertEqual(self.profile.count("--body <the pull request's report (item 3)>"), 2)
        for gone in ("--body <the report, in English>", "<R> <what changed>",
                     "KAN-7 add a --json output flag", "board off: no key"):
            self.assertEqual(self.profile.count(gone), 0, gone)
        self.assertHasAll((KIT / "agents/workflow-worker.md").read_text(), [
            "Never name a ticket key (yours, the run ticket's or any other) in a commit message, "
            "in a card's **Commit** bullet or in anything you write into a file"])
        self.assertHasAll(self.verifier, [
            "A key of this run (the run ticket on your task's first line, a `Batch:`, `In range:` or `Moved:` key) on a line the Range adds is a reason",
            "expected the code to say what it does instead, observed <KEY>",
            "already finds in the tree, in either spelling, or that a Batch card itself gives as a value to write",
            "never a token of the same shape that is not one of those keys (`UTF-8`, `ISO-8601`, `SHA-256`)",
            "is a `note: commit <sha>: names <KEY>` line in `checks`, never a reason",
            "With the board off (your task says `Run <run>`, not `Run ticket <R>`)"])
        self.assertEqual(self.verifier.count("ticket key in a comment or test name"), 0)
        self.assertHasAll(self.readme, ["Its title is `<type>(<scope>): <what changed>`",
                                        "the run ticket keeps the link with its `PR:` comment"])
        self.assertEqual(self.readme.count("<run ticket> <what changed>"), 0)
        self.assertEqual(self.readme.count("so the control plane links the pull request"), 0)

if __name__ == "__main__":
    unittest.main()
