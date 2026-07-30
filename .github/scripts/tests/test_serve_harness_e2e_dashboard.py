"""Tests for importing local Harness E2E reports into the static dashboard."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from publish_harness_e2e_dashboard import load_manifest
from serve_harness_e2e_dashboard import (
    build_local_dashboard,
    discover_results,
)


def scenario(scenario_id: str, score: int = 90, *, passed: bool = True) -> dict:
    return {
        "scenario_id": scenario_id,
        "threshold": 50,
        "execution_policy": {"max_turns": 4},
        "aggregate": {
            "runs": 1,
            "scored_runs": 1,
            "passed_runs": int(passed),
            "required_passes": 1,
            "pass_rate": float(passed),
            "median_score": score,
            "hard_gate_failures": 0 if passed else 1,
            "technical_failures": 0,
            "cost": {
                "subject_usd": 0.2,
                "judge_usd": 0.1,
                "total_usd": 0.3,
            },
        },
        "passed": passed,
        "runs": [
            {
                "run_id": f"{scenario_id}-run",
                "session_id": f"{scenario_id}-session",
                "prompt": f"Run {scenario_id}.",
                "wall_time_ms": 10_000,
                "score": score,
                "status": "passed" if passed else "hard_gate_failed",
                "hard_gates": [],
                "criteria": [],
                "transcript": {"messages": []},
                "metrics": {
                    "totals": {
                        "input_tokens": 100,
                        "output_tokens": 20,
                        "function_calls": 2,
                        "function_call_errors": 0,
                        "sessions": 1,
                        "turns": 2,
                    }
                },
                "cost": {
                    "subject_usd": 0.2,
                    "judge_usd": 0.1,
                    "total_usd": 0.3,
                },
                "retry_attempts": [],
                "failures": [],
            }
        ],
    }


def report(*scenarios: dict, passed: bool = True) -> dict:
    return {
        "subject": {"model": "glm-5.2", "provider": "zai"},
        "judge": {"model": "glm-5.2", "provider": "zai"},
        "passed": passed,
        "scenarios": list(scenarios),
    }


def write_report(path: Path, value: dict, timestamp: int = 1_750_000_000) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))
    os.utime(path, (timestamp, timestamp))
    return path


def fake_git_value(*arguments: str) -> str:
    return "a" * 40 if arguments[-1] == "HEAD" else "main"


class LocalDashboardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_discovers_direct_and_recursive_results_without_duplicates(
        self,
    ) -> None:
        direct = write_report(
            self.root / "direct/results.json",
            report(scenario("direct_answer")),
        )
        nested = write_report(
            self.root / "nested/run/results.json",
            report(scenario("security_review")),
            timestamp=1_750_000_001,
        )

        self.assertEqual(
            discover_results(
                [direct.parent, self.root / "nested", direct],
                site_dir=self.root / "site",
            ),
            [direct, nested],
        )

    @patch(
        "serve_harness_e2e_dashboard.repository_url",
        return_value="https://github.com/iii-hq/workers",
    )
    @patch(
        "serve_harness_e2e_dashboard.git_value",
        side_effect=fake_git_value,
    )
    def test_builds_idempotent_local_history_with_full_multi_scenario_detail(
        self,
        _git_value,
        _repository_url,
    ) -> None:
        results = write_report(
            self.root / "reports/results.json",
            report(
                scenario("direct_answer"),
                scenario("security_review", score=75),
            ),
        )
        site = self.root / "site"

        first_ids = build_local_dashboard([results], site_dir=site)
        second_ids = build_local_dashboard([results], site_dir=site)

        self.assertEqual(first_ids, second_ids)
        manifest = load_manifest(site / "executions.js")
        self.assertEqual(manifest["mode"], "local")
        self.assertEqual(len(manifest["executions"]), 1)
        execution = manifest["executions"][0]
        self.assertEqual(execution["event"], "local")
        self.assertEqual(execution["status"], "passed")
        self.assertEqual(execution["totals"]["total_tokens"], 240)
        detail = json.loads((site / execution["detail_path"]).read_text())
        self.assertEqual(
            [entry["scenario_id"] for entry in detail["reports"]],
            ["direct_answer", "security_review"],
        )
        self.assertTrue(
            (site / "data.js").read_text().startswith(
                "window.BENCHMARK_DATA = "
            )
        )
        self.assertNotIn(
            "HARNESS_BENCHMARK_PREVIEW",
            (site / "data.js").read_text(),
        )

    @patch(
        "serve_harness_e2e_dashboard.repository_url",
        return_value="https://github.com/iii-hq/workers",
    )
    @patch(
        "serve_harness_e2e_dashboard.git_value",
        side_effect=fake_git_value,
    )
    def test_accumulates_changed_reports_and_preserves_failures(
        self,
        _git_value,
        _repository_url,
    ) -> None:
        first = write_report(
            self.root / "first/results.json",
            report(scenario("direct_answer")),
        )
        second = write_report(
            self.root / "second/results.json",
            report(
                scenario("direct_answer", score=40, passed=False),
                passed=False,
            ),
            timestamp=1_750_000_001,
        )
        site = self.root / "site"

        build_local_dashboard([first], site_dir=site)
        build_local_dashboard([second], site_dir=site)

        manifest = load_manifest(site / "executions.js")
        self.assertEqual(len(manifest["executions"]), 2)
        self.assertEqual(manifest["executions"][0]["status"], "failed")
        self.assertEqual(manifest["executions"][1]["status"], "passed")


if __name__ == "__main__":
    unittest.main()
