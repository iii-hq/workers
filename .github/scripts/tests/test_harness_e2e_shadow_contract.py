import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "harness_e2e_shadow_contract.py"
SPEC = importlib.util.spec_from_file_location("shadow_contract", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def contract():
    return {
        "schema_version": 1,
        "campaign_id": "11111111-1111-4111-8111-111111111111",
        "execution_id": "22222222-2222-4222-8222-222222222222",
        "attempt": 1,
        "idempotency_key": f"rc:d0:{'a' * 64}",
        "target": {
            "application": "harness",
            "version": "1.9.0",
            "source_sha": "b" * 40,
            "deployment_id": "33333333-3333-4333-8333-333333333333",
            "stack_versions": {"harness": "1.9.0", "database": "0.5.1"},
            "stack_digest": f"sha256:{'c' * 64}",
        },
        "plan": {
            "id": "44444444-4444-4444-8444-444444444444",
            "revision": 1,
            "sha256": f"sha256:{'d' * 64}",
            "definition": {
                "mode": "demonstrative",
                "entrypoint": "e2e::run",
                "label": "Shadow",
                "lane": "release-control-shadow",
                "subject": {"provider": "anthropic", "model": "claude-sonnet-4-6"},
                "judge": {"provider": "anthropic", "model": "claude-sonnet-4-6"},
                "scenarios": ["direct_answer"],
                "runs": 1,
                "seed": 4404,
                "technicalRetries": 1,
                "progressIntervalSeconds": 15,
            },
        },
        "runner": {"registry_worker": "harness-e2e", "registry_ref": "next"},
    }


def catalog():
    return {
        "schema": "e2e-scenario-catalog/v1",
        "runner": {"name": "harness-e2e", "version": "0.4.0", "revision": "e" * 40},
        "catalog_sha256": f"sha256:{'f' * 64}",
        "scenarios": [
            {
                "scenario_id": "direct_answer",
                "scenario_version": 2,
                "case_id": "direct_answer:4404",
                "seed": 4404,
                "inputs_sha256": f"sha256:{'1' * 64}",
                "contract_sha256": f"sha256:{'2' * 64}",
            }
        ],
    }


class ShadowContractTest(unittest.TestCase):
    def test_materializes_observe_only_request(self):
        request = MODULE.materialize_request(contract(), catalog())
        self.assertEqual(request["run_contract"]["mode"]["decision"], "observe_only")
        self.assertEqual(request["run_contract"]["runner"]["revision"], "e" * 40)
        self.assertEqual(request["run_contract"]["selected_cases"][0]["scenario_version"], 2)

    def test_rejects_catalog_seed_drift(self):
        changed = catalog()
        changed["scenarios"][0]["seed"] = 9
        with self.assertRaisesRegex(ValueError, "seed does not match"):
            MODULE.materialize_request(contract(), changed)

    def test_accepts_exact_registry_prerelease_versions(self):
        changed = contract()
        changed["target"]["stack_versions"]["database"] = "0.11.0-next.5"
        MODULE.validate_contract(changed)

    def test_packages_raw_file_digests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "results.json").write_text("{}\n")
            manifest = MODULE.package_bundle(root, contract(), {"run_id": 7, "run_attempt": 1})
        self.assertEqual(manifest["terminal_payload"], "results.json")
        self.assertRegex(manifest["files"][0]["sha256"], r"^sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
