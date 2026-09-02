#!/usr/bin/env python3
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from harness.scripts.search_eval_catalog import (
    collect,
    function_ids,
    main,
    run_iii,
    write_capture,
)


FIXTURES = Path(__file__).with_name("search_eval")


def load(name):
    return json.loads((FIXTURES / name).read_text())


class SearchEvalCatalogTests(unittest.TestCase):
    def setUp(self):
        self.normal = load("normal-functions.json")
        self.internal = load("internal-functions.json")
        self.details = load("info-responses.json")
        self.calls = []

    def trigger(self, function, payload):
        self.calls.append((function, payload))
        if function == "engine::functions::list":
            return self.internal if payload.get("include_internal") else self.normal
        if function == "engine::functions::info":
            return {
                "functions": [self.details[id] for id in payload["function_ids"]]
            }
        raise AssertionError(function)

    def test_collects_batched_filtered_and_stably_sorted_catalog(self):
        catalog, errors, raw = collect(self.trigger, batch_size=2)

        self.assertEqual(
            [entry["name"] for entry in catalog], ["alpha::first", "beta::second"]
        )
        self.assertEqual(catalog[0]["parameters"], self.details["alpha::first"]["request_schema"])
        self.assertEqual(errors, [{"function_id": "zeta::last", "error": "forbidden"}])
        self.assertEqual(
            [payload["function_ids"] for function, payload in self.calls if function == "engine::functions::info"],
            [["alpha::first", "beta::second"], ["hooks::hidden", "zeta::last"]],
        )
        self.assertEqual(raw["normal"], self.normal)
        self.assertEqual(raw["internal"], self.internal)

    def test_reads_production_function_list_shapes(self):
        rows = [
            "alpha::string",
            {"function_id": "beta::function-id"},
            {"id": "gamma::id"},
            {"name": "delta::name"},
        ]

        for response in (rows, {"functions": rows}, {"items": rows}):
            self.assertEqual(
                function_ids(response),
                ["alpha::string", "beta::function-id", "gamma::id", "delta::name"],
            )

    def test_trigger_routes_engine_controls_through_default_namespace(self):
        with patch(
            "harness.scripts.search_eval_catalog.subprocess.run",
            return_value=SimpleNamespace(stdout="{}"),
        ) as run:
            self.assertEqual(run_iii("engine::functions::list", {}), {})

        run.assert_called_once_with(
            ["iii", "trigger", "engine::functions::list", "--json", "{}"],
            check=True,
            text=True,
            capture_output=True,
            timeout=30,
        )

    def test_trigger_routes_to_the_requested_engine_port(self):
        with patch(
            "harness.scripts.search_eval_catalog.subprocess.run",
            return_value=SimpleNamespace(stdout="{}"),
        ) as run:
            self.assertEqual(
                run_iii("engine::functions::list", {}, port=49234),
                {},
            )

        run.assert_called_once_with(
            [
                "iii",
                "trigger",
                "--port",
                "49234",
                "engine::functions::list",
                "--json",
                "{}",
            ],
            check=True,
            text=True,
            capture_output=True,
            timeout=30,
        )

    def test_cli_uses_the_requested_port_for_catalog_capture(self):
        listed = SimpleNamespace(
            stdout=json.dumps(
                {
                    "functions": [
                        {
                            "function_id": "state::get",
                            "namespace": "minilm-production",
                        }
                    ]
                }
            )
        )
        info = SimpleNamespace(
            stdout=json.dumps(
                {
                    "functions": [
                        {
                            "function_id": "state::get",
                            "description": "Read state",
                            "request_schema": {"type": "object"},
                        }
                    ]
                }
            )
        )
        with tempfile.TemporaryDirectory() as temp, patch(
            "harness.scripts.search_eval_catalog.subprocess.run",
            side_effect=[listed, listed, info],
        ) as run:
            main(
                [
                    "--port",
                    "49234",
                    "--namespace",
                    "minilm-production",
                    "--output-root",
                    temp,
                ]
            )

        self.assertEqual(run.call_count, 3)
        for call in run.call_args_list:
            self.assertEqual(call.args[0][0:4], ["iii", "trigger", "--port", "49234"])

    def test_cli_refuses_to_accept_an_empty_capture(self):
        response = SimpleNamespace(stdout='{"functions": []}')
        with tempfile.TemporaryDirectory() as temp, patch(
            "harness.scripts.search_eval_catalog.subprocess.run",
            return_value=response,
        ):
            root = Path(temp)
            fixture = root / "committed-catalog.json"
            fixture.write_text('[{"name":"preserved"}]\n')

            with self.assertRaises(SystemExit):
                main(
                    [
                        "--port",
                        "49234",
                        "--namespace",
                        "minilm-production",
                        "--output-root",
                        str(root / "captures"),
                        "--fixture",
                        str(fixture),
                        "--accept",
                    ]
                )

            self.assertEqual(json.loads(fixture.read_text()), [{"name": "preserved"}])
            self.assertEqual(len(list((root / "captures").glob("*/catalog.json"))), 1)

    def test_cli_requires_an_explicit_engine_port_before_capture(self):
        with tempfile.TemporaryDirectory() as temp, patch(
            "harness.scripts.search_eval_catalog.subprocess.run"
        ) as run:
            with self.assertRaises(SystemExit):
                main(["--output-root", temp])

        run.assert_not_called()

    def test_collect_scopes_explicit_namespaces_and_info_requests(self):
        calls = []

        def trigger(function, payload):
            calls.append((function, payload))
            if function == "engine::functions::list":
                return {
                    "functions": [
                        {"function_id": "alpha::first", "namespace": "search-eval"},
                        {"function_id": "foreign::function", "namespace": "alternate"},
                        {"function_id": "legacy::function"},
                    ]
                }
            return {
                "functions": [
                    {
                        "function_id": function_id,
                        "description": function_id,
                        "request_schema": {"type": "object"},
                    }
                    for function_id in payload["function_ids"]
                ]
            }

        catalog, errors, _ = collect(trigger, namespace="search-eval")

        self.assertEqual([entry["name"] for entry in catalog], ["alpha::first", "legacy::function"])
        self.assertEqual(errors, [])
        self.assertEqual(
            [payload for function, payload in calls if function == "engine::functions::info"],
            [{"function_ids": ["alpha::first", "legacy::function"], "namespace": "search-eval"}],
        )

    def test_records_malformed_info_response_without_discarding_other_batches(self):
        def malformed_once(function, payload):
            if function == "engine::functions::info" and payload["function_ids"] == ["hooks::hidden", "zeta::last"]:
                return {"not_functions": []}
            return self.trigger(function, payload)

        catalog, errors, _ = collect(malformed_once, batch_size=2)

        self.assertEqual([entry["name"] for entry in catalog], ["alpha::first", "beta::second"])
        self.assertIn(
            {"batch": ["hooks::hidden", "zeta::last"], "error": "malformed response"},
            errors,
        )

    def test_only_accept_updates_committed_fixture(self):
        catalog, errors, raw = collect(self.trigger, batch_size=32)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture = root / "catalog.json"
            fixture.write_text('[{"name":"old"}]\n')

            artifact = write_capture(root, catalog, errors, raw, accept=False, fixture=fixture)
            self.assertEqual(json.loads(fixture.read_text()), [{"name": "old"}])
            self.assertTrue((artifact / "catalog.json").exists())
            self.assertTrue((artifact / "errors.json").exists())

            write_capture(root, catalog, errors, raw, accept=True, fixture=fixture)
            self.assertEqual(json.loads(fixture.read_text()), catalog)

    def test_capture_allowlists_raw_registry_evidence(self):
        raw = {
            "normal": {
                "functions": [
                    {
                        "function_id": "alpha::first",
                        "worker_name": "alpha",
                        "description": "First",
                        "metadata": {"internal": False, "api_key": "do-not-write"},
                        "config": {"token": "do-not-write"},
                    }
                ],
                "access_token": "do-not-write",
            },
            "internal": {"items": [{"id": "hooks::hidden", "secret": "do-not-write"}]},
            "info": [
                {
                    "function_ids": ["alpha::first"],
                    "response": {
                        "functions": [
                            {
                                "function_id": "alpha::first",
                                "description": "First",
                                "request_schema": {"type": "object"},
                                "response_schema": {"type": "object"},
                                "metadata": {"internal": False, "config": {"token": "do-not-write"}},
                                "credentials": {"password": "do-not-write"},
                            }
                        ],
                        "config": {"token": "do-not-write"},
                    },
                }
            ],
        }
        with tempfile.TemporaryDirectory() as temp:
            artifact = write_capture(Path(temp), [], [], raw, accept=False, fixture=Path(temp) / "fixture.json")

            self.assertEqual(
                json.loads((artifact / "normal-functions.json").read_text()),
                {
                    "functions": [
                        {
                            "function_id": "alpha::first",
                            "worker_name": "alpha",
                            "description": "First",
                            "metadata": {"internal": False},
                        }
                    ]
                },
            )
            self.assertEqual(
                json.loads((artifact / "internal-functions.json").read_text()),
                {"items": [{"id": "hooks::hidden"}]},
            )
            self.assertEqual(
                json.loads((artifact / "info-batches.json").read_text()),
                [
                    {
                        "function_ids": ["alpha::first"],
                        "response": {
                            "functions": [
                                {
                                    "function_id": "alpha::first",
                                    "description": "First",
                                    "request_schema": {"type": "object"},
                                    "response_schema": {"type": "object"},
                                    "metadata": {"internal": False},
                                }
                            ]
                        },
                    }
                ],
            )


if __name__ == "__main__":
    unittest.main()
