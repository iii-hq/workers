"""Tests for importing local Harness E2E reports into the static dashboard."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from publish_harness_e2e_dashboard import load_manifest
from serve_harness_e2e_dashboard import (
    LocalDashboardError,
    LocalRunController,
    build_run_command,
    build_local_dashboard,
    discover_results,
    harness_e2e_command,
    initialize_local_dashboard,
    load_local_catalog,
    validate_run_request,
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
        self.assertEqual(manifest["executions"][0]["status"], "hard_gate_failed")
        self.assertEqual(manifest["executions"][1]["status"], "passed")

    @patch(
        "serve_harness_e2e_dashboard.repository_url",
        return_value="https://github.com/iii-hq/workers",
    )
    @patch(
        "serve_harness_e2e_dashboard.git_value",
        side_effect=fake_git_value,
    )
    def test_keeps_quality_only_local_result_advisory(
        self,
        _git_value,
        _repository_url,
    ) -> None:
        quality = scenario("direct_answer", score=40, passed=False)
        quality["aggregate"]["hard_gate_failures"] = 0
        quality["runs"][0]["status"] = "quality_failed"
        results = write_report(
            self.root / "quality/results.json",
            report(quality, passed=False),
        )

        build_local_dashboard([results], site_dir=self.root / "site")

        manifest = load_manifest(self.root / "site/executions.js")
        self.assertEqual(manifest["executions"][0]["status"], "quality_advisory")

    @patch(
        "serve_harness_e2e_dashboard.repository_url",
        return_value="https://github.com/iii-hq/workers",
    )
    @patch(
        "serve_harness_e2e_dashboard.git_value",
        side_effect=fake_git_value,
    )
    def test_ui_run_identity_and_label_remain_distinct_from_report_digest(
        self,
        _git_value,
        _repository_url,
    ) -> None:
        results = write_report(
            self.root / "run/results.json",
            report(scenario("direct_answer")),
        )
        site = self.root / "site"

        execution_ids = build_local_dashboard(
            [results],
            site_dir=site,
            labels_by_path={results.resolve(): "After prompt change"},
            run_ids_by_path={results.resolve(): "local-explicit-run"},
        )

        self.assertEqual(execution_ids, ["local-explicit-run-1"])
        execution = load_manifest(site / "executions.js")["executions"][0]
        self.assertEqual(execution["id"], "local-explicit-run-1")
        self.assertEqual(execution["label"], "After prompt change")

    @patch(
        "serve_harness_e2e_dashboard.repository_url",
        return_value="https://github.com/iii-hq/workers",
    )
    def test_initializes_an_empty_local_history_without_a_results_file(
        self,
        _repository_url,
    ) -> None:
        site = self.root / "site"

        initialize_local_dashboard(site)

        manifest = load_manifest(site / "executions.js")
        self.assertEqual(manifest["mode"], "local")
        self.assertEqual(manifest["executions"], [])
        self.assertTrue((site / ".harness-e2e-local-dashboard").is_file())

    def test_validates_run_request_and_reuses_prebuilt_runner(self) -> None:
        runner = self.root / "harness-e2e"
        runner.write_text("prebuilt runner")
        runner.chmod(0o755)
        request = validate_run_request(
            {
                "label": "Prompt B",
                "scenarios": ["direct_answer", "direct_answer", "security_review"],
                "runs": 3,
                "technical_retries": 0,
            },
            environment={
                "III_URL": "ws://127.0.0.1:49134",
                "HARNESS_E2E_MODEL": "glm-5.2",
                "HARNESS_E2E_PROVIDER": "zai",
            },
        )

        with patch.dict(os.environ, {"HARNESS_E2E_BIN": str(runner)}):
            command = build_run_command(request, self.root / "output")

        self.assertEqual(request["scenarios"], ["direct_answer", "security_review"])
        self.assertEqual(command[0], str(runner))
        self.assertNotIn("cargo", command)
        self.assertEqual(command.count("--scenario"), 2)
        self.assertEqual(command[command.index("--runs") + 1], "3")

    def test_runner_build_is_explicit_when_no_binary_exists(self) -> None:
        missing = self.root / "missing-harness-e2e"
        with self.assertRaisesRegex(LocalDashboardError, "build it once"):
            harness_e2e_command(
                "list",
                environment={"HARNESS_E2E_BIN": str(missing)},
            )

        with (
            patch("serve_harness_e2e_dashboard.DEFAULT_RUNNER_DEBUG", missing),
            patch("serve_harness_e2e_dashboard.DEFAULT_RUNNER_RELEASE", missing),
            patch("serve_harness_e2e_dashboard.shutil.which", return_value=None),
        ):
            command = harness_e2e_command(
                "list",
                environment={"HARNESS_E2E_ALLOW_BUILD": "1"},
            )
        self.assertEqual(command[0], "cargo")
        self.assertIn("--locked", command)

    def test_rejects_unsafe_or_unbounded_run_requests(self) -> None:
        environment = {
            "HARNESS_E2E_MODEL": "model",
            "HARNESS_E2E_PROVIDER": "provider",
        }
        with self.assertRaisesRegex(LocalDashboardError, "ws:// or wss://"):
            validate_run_request(
                {"url": "https://example.com"},
                environment=environment,
            )
        with self.assertRaisesRegex(LocalDashboardError, "runs must be between"):
            validate_run_request(
                {"url": "ws://127.0.0.1:49134", "runs": 21},
                environment=environment,
            )
        with self.assertRaisesRegex(LocalDashboardError, "invalid scenario id"):
            validate_run_request(
                {
                    "url": "ws://127.0.0.1:49134",
                    "scenarios": ["../escape"],
                },
                environment=environment,
            )
        with self.assertRaisesRegex(LocalDashboardError, "technical_retries must be"):
            validate_run_request(
                {
                    "url": "ws://127.0.0.1:49134",
                    "technical_retries": 4,
                },
                environment=environment,
            )

    @patch("serve_harness_e2e_dashboard.subprocess.run")
    def test_loads_scenarios_and_registered_models_from_runner_catalog(
        self,
        run,
    ) -> None:
        runner = self.root / "harness-e2e"
        runner.write_text("prebuilt runner")
        runner.chmod(0o755)
        with patch.dict(os.environ, {"HARNESS_E2E_BIN": str(runner)}):
            run.side_effect = [
                subprocess.CompletedProcess(
                    harness_e2e_command("list"),
                    0,
                    stdout='["direct_answer","shell_coder_sandbox"]\n',
                    stderr="",
                ),
                subprocess.CompletedProcess(
                    harness_e2e_command(
                        "models",
                        "--url",
                        "ws://127.0.0.1:49134",
                        "--json",
                    ),
                    0,
                    stdout=(
                        "2026-08-07T12:38:29Z INFO connected\n"
                        '[{"provider":"zai","id":"glm-5.2"}]\n'
                    ),
                    stderr="",
                ),
            ]

            catalog = load_local_catalog("ws://127.0.0.1:49134")

        self.assertEqual(
            catalog,
            {
                "url": "ws://127.0.0.1:49134",
                "models": [{"provider": "zai", "model": "glm-5.2"}],
                "scenarios": ["direct_answer", "shell_coder_sandbox"],
            },
        )
        self.assertEqual(run.call_count, 2)

    @patch("serve_harness_e2e_dashboard.load_local_catalog")
    def test_caches_catalog_per_harness_url(self, load_catalog) -> None:
        expected = {
            "url": "ws://127.0.0.1:49134",
            "models": [{"provider": "zai", "model": "glm-5.2"}],
            "scenarios": ["direct_answer"],
        }
        load_catalog.return_value = expected
        controller = LocalRunController(self.root / "site", self.root / "runs")

        self.assertEqual(controller.catalog(expected["url"]), expected)
        self.assertEqual(controller.catalog(expected["url"]), expected)
        self.assertEqual(
            controller.catalog(expected["url"], refresh=True),
            expected,
        )
        self.assertEqual(load_catalog.call_count, 2)

    @patch(
        "serve_harness_e2e_dashboard.repository_url",
        return_value="https://github.com/iii-hq/workers",
    )
    @patch(
        "serve_harness_e2e_dashboard.git_value",
        side_effect=fake_git_value,
    )
    def test_controller_runs_in_background_and_imports_generated_results(
        self,
        _git_value,
        _repository_url,
    ) -> None:
        payload = json.dumps(report(scenario("direct_answer")))

        def fake_command(_request: dict, output_dir: Path) -> list[str]:
            script = (
                "import pathlib,sys; "
                "path=pathlib.Path(sys.argv[1]); "
                "path.mkdir(parents=True,exist_ok=True); "
                "(path/'results.json').write_text(sys.argv[2]); print('simulated E2E')"
            )
            return [sys.executable, "-c", script, str(output_dir), payload]

        controller = LocalRunController(
            self.root / "site",
            self.root / "runs",
        )
        with patch(
            "serve_harness_e2e_dashboard.build_run_command",
            side_effect=fake_command,
        ):
            controller.start(
                {
                    "url": "ws://127.0.0.1:49134",
                    "model": "glm-5.2",
                    "provider": "zai",
                    "label": "Generated by controller",
                }
            )
            for _ in range(100):
                snapshot = controller.snapshot()
                if snapshot["job"]["status"] not in {"starting", "running"}:
                    break
                time.sleep(0.01)

        job = snapshot["job"]
        self.assertEqual(job["status"], "completed")
        self.assertEqual(job["returncode"], 0)
        self.assertIn("simulated E2E", job["log"])
        execution = load_manifest(self.root / "site/executions.js")["executions"][0]
        self.assertEqual(execution["id"], job["execution_id"])
        self.assertEqual(execution["label"], "Generated by controller")


if __name__ == "__main__":
    unittest.main()
