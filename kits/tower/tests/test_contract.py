import copy
import json
from pathlib import Path
import re
import unittest

from jsonschema import Draft7Validator, FormatChecker
import yaml


KIT = Path(__file__).resolve().parents[1]
READS = {
    "kanban::ticket::list", "kanban::activity::list", "kanban::release::list",
    "session::list", "session::messages-tail", "harness::status",
    "harness::ask::list", "github::pr::list", "github::pr::checks",
}


class TowerContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((KIT / "result.schema.json").read_text())
        cls.validator = Draft7Validator(cls.schema, format_checker=FormatChecker())
        cls.examples = [json.loads(block) for block in re.findall(
            r"```json\n(.*?)\n```", (KIT / "README.md").read_text(), re.S
        )]
        cls.result = next(example for example in cls.examples if "brief" in example)

    def test_exact_reads_without_inherited_tools_or_model(self):
        profile = yaml.safe_load(
            (KIT / "agents/tower-writer.md").read_text().split("---", 2)[1]
        )
        permissions = yaml.safe_load((KIT / "iii-permissions.yaml").read_text())
        policy = next(example for example in self.examples if "allow" in example)
        for functions in (profile["functions"], permissions["rules"], policy["allow"]):
            self.assertEqual(set(functions), READS)
            self.assertEqual(len(functions), len(READS))
        self.assertTrue(profile["hidden"])
        self.assertFalse(profile["skills_index"])
        self.assertNotIn("extends", profile)
        self.assertNotIn("model", profile)

    def test_changed_unchanged_and_empty_results(self):
        Draft7Validator.check_schema(self.schema)
        self.validator.validate(self.result)
        unchanged = dict(self.result, unchanged=True)
        self.validator.validate(unchanged)
        empty = dict(self.result, brief={"headline": "", "text": "", "refs": []},
                     diary=[], now={}, pr_waits={})
        self.validator.validate(empty)

    def test_every_prose_object_needs_nonempty_references(self):
        for path in (("brief",), ("diary", 0), ("now", "session:dispatch-kan635"),
                     ("pr_waits", "iii-hq/workers#1834")):
            for refs in (None, []):
                with self.subTest(path=path, refs=refs):
                    result = copy.deepcopy(self.result)
                    line = result
                    for key in path:
                        line = line[key]
                    if refs is None:
                        del line["refs"]
                    else:
                        line["refs"] = refs
                    self.assertFalse(self.validator.is_valid(result))

    def test_references_and_subject_keys(self):
        for reference in ("KAN-616", "PROJ2-23", "session:tower-all", "iii-hq/workers#1834"):
            with self.subTest(reference=reference):
                result = copy.deepcopy(self.result)
                result["brief"]["refs"] = [reference]
                self.validator.validate(result)
        for reference in ("https://github.com/iii-hq/workers/pull/1834", "release:r1",
                          "question:q1", "session:", "iii-hq/workers#0"):
            with self.subTest(reference=reference):
                result = copy.deepcopy(self.result)
                result["brief"]["refs"] = [reference]
                self.assertFalse(self.validator.is_valid(result))
        for field in ("now", "pr_waits"):
            result = copy.deepcopy(self.result)
            result[field] = {"KAN-616": next(iter(result[field].values()))}
            self.assertFalse(self.validator.is_valid(result))

    def test_diary_bound_timestamps_and_exact_shape(self):
        result = copy.deepcopy(self.result)
        result["diary"] *= 50
        self.validator.validate(result)
        result["diary"].append(result["diary"][0])
        self.assertFalse(self.validator.is_valid(result))
        for field in self.result:
            result = copy.deepcopy(self.result)
            del result[field]
            self.assertFalse(self.validator.is_valid(result))
        for field, value in (("checked_at", "yesterday"), ("unchanged", "true"),
                             ("extra", "instruction")):
            result = dict(self.result, **{field: value})
            self.assertFalse(self.validator.is_valid(result))
        result = copy.deepcopy(self.result)
        result["diary"][0]["at"] = "yesterday"
        self.assertFalse(self.validator.is_valid(result))

    def test_names_with_digits_remain_valid_prose(self):
        result = copy.deepcopy(self.result)
        result["brief"]["headline"] = "KAN-635 verified billing-v2 for R1 and round 23."
        self.validator.validate(result)


if __name__ == "__main__":
    unittest.main()
