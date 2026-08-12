"""Tests for the Harness E2E manifest preflight."""

from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import Mock, patch

from validate_harness_e2e_manifests import (
    CORE_SOURCE_WORKERS,
    Component,
    PreflightInputError,
    emit_annotations,
    resolve_components,
    validate_components,
)


SUBJECTS = json.dumps(
    [
        {"id": "glm", "model": "glm-5", "provider": "zai"},
        {"id": "codex", "model": "gpt-5", "provider": "openai"},
    ]
)


class ManifestPreflightTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_resolves_core_and_deduplicated_source_providers(self) -> None:
        components = resolve_components(
            self.root,
            stack_mode="source",
            subjects_json=SUBJECTS,
            judge_provider="openai",
        )

        self.assertEqual(
            [component.name for component in components],
            [
                *CORE_SOURCE_WORKERS,
                "provider-openai",
                "provider-zai",
                "harness",
            ],
        )
        self.assertEqual(
            components[-1].expected_binaries,
            ("harness", "harness-e2e"),
        )

    def test_registry_mode_only_validates_the_runner_workspace(self) -> None:
        components = resolve_components(
            self.root,
            stack_mode="registry",
            subjects_json=SUBJECTS,
            judge_provider="anthropic",
        )

        self.assertEqual([component.name for component in components], ["harness"])

    def test_rejects_unsafe_or_malformed_provider_inputs(self) -> None:
        for subjects, judge in (
            ('[{"provider":"../openai"}]', "anthropic"),
            (SUBJECTS, "../anthropic"),
            ("{}", "anthropic"),
        ):
            with self.subTest(subjects=subjects, judge=judge):
                with self.assertRaises(PreflightInputError):
                    resolve_components(
                        self.root,
                        stack_mode="source",
                        subjects_json=subjects,
                        judge_provider=judge,
                    )

    def test_accumulates_metadata_and_binary_failures(self) -> None:
        first = Component("one", self.root / "one/Cargo.toml", ("one",))
        second = Component("two", self.root / "two/Cargo.toml", ("two",))
        first.manifest.parent.mkdir()
        second.manifest.parent.mkdir()
        first.manifest.write_text("[package]\nname='one'\n")
        second.manifest.write_text("[package]\nname='two'\n")

        responses = [
            Mock(returncode=101, stdout="", stderr="lock file needs to be updated"),
            Mock(
                returncode=0,
                stdout=json.dumps(
                    {
                        "packages": [
                            {"targets": [{"name": "other", "kind": ["bin"]}]}
                        ]
                    }
                ),
                stderr="",
            ),
        ]
        with patch(
            "validate_harness_e2e_manifests.subprocess.run",
            side_effect=responses,
        ) as run:
            failures = validate_components([first, second], root=self.root)

        self.assertEqual(len(failures), 2)
        self.assertEqual(failures[0].message, "lock file needs to be updated")
        self.assertEqual(
            failures[1].message,
            "missing expected binary target(s): two",
        )
        self.assertEqual(run.call_count, 2)
        self.assertEqual(
            run.call_args_list[0].args[0][1:4],
            ["metadata", "--locked", "--no-deps"],
        )

    def test_missing_manifests_are_annotated_together(self) -> None:
        components = [
            Component("one", self.root / "one/Cargo.toml", ("one",)),
            Component("two", self.root / "two/Cargo.toml", ("two",)),
        ]

        failures = validate_components(components, root=self.root)
        output = io.StringIO()
        with redirect_stdout(output):
            emit_annotations(failures, root=self.root)

        rendered = output.getvalue()
        self.assertEqual(rendered.count("::error "), 2)
        self.assertIn("file=one/Cargo.toml", rendered)
        self.assertIn("file=two/Cargo.toml", rendered)


if __name__ == "__main__":
    unittest.main()
