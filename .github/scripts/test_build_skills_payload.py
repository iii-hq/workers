#!/usr/bin/env python3
"""Unit tests for build_skills_payload.collect_skills."""

from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from build_skills_payload import TOP_SKILL_KEY, collect_skills  # noqa: E402


class CollectSkillsTests(unittest.TestCase):
    def test_single_skill_md_publishes_bundle_root_skill_md(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp) / "my-worker"
            (root / "skills").mkdir(parents=True)
            (root / "skills" / "SKILL.md").write_text("# My Worker\n", encoding="utf-8")
            skills = collect_skills(root)
            self.assertEqual(skills, {TOP_SKILL_KEY: "# My Worker\n"})

    def test_legacy_index_md_publishes_as_skill_md(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp) / "legacy-worker"
            (root / "skills").mkdir(parents=True)
            (root / "skills" / "index.md").write_text("# Legacy\n", encoding="utf-8")
            skills = collect_skills(root)
            self.assertEqual(skills, {TOP_SKILL_KEY: "# Legacy\n"})

    def test_skill_md_plus_nested_extra(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp) / "nested-worker"
            (root / "skills" / "extra").mkdir(parents=True)
            (root / "skills" / "SKILL.md").write_text("# Overview\n", encoding="utf-8")
            (root / "skills" / "extra" / "topic.md").write_text("# Topic\n", encoding="utf-8")
            skills = collect_skills(root)
            self.assertEqual(
                skills,
                {
                    TOP_SKILL_KEY: "# Overview\n",
                    "skills/extra/topic.md": "# Topic\n",
                },
            )

    def test_empty_skill_md_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp) / "empty-worker"
            (root / "skills").mkdir(parents=True)
            (root / "skills" / "SKILL.md").write_text("   \n", encoding="utf-8")
            skills = collect_skills(root)
            self.assertEqual(skills, {})


if __name__ == "__main__":
    unittest.main()
