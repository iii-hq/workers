import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "harness_e2e_shadow_contract.py"
RUNNER_SCRIPT = Path(__file__).parents[3] / "harness" / "tests" / "e2e" / "run-shadow-control-ci.sh"
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


def manual_v2_contract():
    versions = {"harness": "1.9.0", "state": "0.22.1"}
    stack_digest = MODULE.canonical_sha256(versions)
    return {
        "schema_version": 2,
        "campaign_id": "11111111-1111-4111-8111-111111111111",
        "execution_id": "22222222-2222-4222-8222-222222222222",
        "attempt": 1,
        "idempotency_key": f"rc:d0:{'a' * 64}",
        "target": {
            "application": "harness",
            "version": "1.9.0",
            "source_sha": "b" * 40,
            "deployment_id": "33333333-3333-4333-8333-333333333333",
            "stack_versions": versions,
            "stack_digest": stack_digest,
            "origin": None,
            "base": {"kind": "deployment", "id": "33333333-3333-4333-8333-333333333333"},
            "stack": {
                "requested_versions": versions,
                "resolved_versions": versions,
                "resolution_sha256": stack_digest,
                "provenance": [
                    {
                        "worker": "harness",
                        "version": "1.9.0",
                        "source_sha": "b" * 40,
                        "operation_id": "33333333-3333-4333-8333-333333333333",
                    },
                    {"worker": "state", "version": "0.22.1"},
                ],
            },
        },
        "plan": {
            "id": "44444444-4444-4444-8444-444444444444",
            "revision": 2,
            "sha256": f"sha256:{'c' * 64}",
            "definition": {
                "mode": "demonstrative",
                "entrypoint": "e2e::run",
                "label": "Manual stack",
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
        "runner": {"registry_worker": "harness-e2e", "registry_ref": "0.1.0-experimental"},
        "workflow": {"repository": "iii-hq/workers", "file": "harness-e2e-shadow.yml", "ref": "main"},
        "runtime": {"cli": {"version": "0.22.1"}, "stack_versions": versions, "stack_digest": stack_digest},
        "security": {"oidc_audience": "release-control-harness-e2e"},
    }


def manual_v2_catalog():
    return {
        **catalog(),
        "runner": {"name": "harness-e2e", "version": "0.1.0-experimental", "revision": "e" * 40},
    }


def manual_v2_contract_with_versions(versions: dict[str, str]):
    changed = manual_v2_contract()
    stack_digest = MODULE.canonical_sha256(versions)
    changed["target"]["stack_versions"] = versions
    changed["target"]["stack_digest"] = stack_digest
    changed["target"]["stack"]["requested_versions"] = versions
    changed["target"]["stack"]["resolved_versions"] = versions
    changed["target"]["stack"]["resolution_sha256"] = stack_digest
    changed["target"]["stack"]["provenance"] = [
        {"worker": worker, "version": version} for worker, version in sorted(versions.items())
    ]
    changed["target"]["stack"]["provenance"][0].update(
        {
            "source_sha": "b" * 40,
            "operation_id": "33333333-3333-4333-8333-333333333333",
        }
    )
    changed["runtime"]["stack_versions"] = versions
    changed["runtime"]["stack_digest"] = stack_digest
    return changed


def write_lock(path: Path, workers: dict[str, dict[str, str]]):
    path.write_text(json.dumps({"version": 1, "workers": workers}))


class ShadowContractTest(unittest.TestCase):
    def test_ephemeral_runner_writes_workers_to_the_engine_config(self):
        runner = RUNNER_SCRIPT.read_text()
        self.assertIn('export III_CONFIG_PATH="$project_config"', runner)
        self.assertIn('iii.config.yaml config.yaml iii.lock workers.json', runner)

    def test_ephemeral_runner_waits_for_a_stable_persistence_plane_before_admission(self):
        runner = RUNNER_SCRIPT.read_text()
        self.assertIn('HARNESS_E2E_STACK_SETTLE_SECONDS', runner)
        self.assertIn('HARNESS_E2E_ADMISSION_TIMEOUT_SECONDS', runner)
        self.assertIn('state::list state::get state::set', runner)
        self.assertIn('storage::putObject storage::getObject database::execute database::query', runner)
        self.assertIn('trigger state::list --port "$engine_port"', runner)

    def test_materializes_observe_only_request(self):
        request = MODULE.materialize_request(contract(), catalog())
        self.assertEqual(request["run_contract"]["mode"]["decision"], "observe_only")
        self.assertEqual(request["run_contract"]["runner"]["revision"], "e" * 40)
        self.assertEqual(request["run_contract"]["selected_cases"][0]["scenario_version"], 2)
        self.assertEqual(request["idempotency_key"], MODULE.observation_idempotency_key(request))
        self.assertNotEqual(request["idempotency_key"], contract()["idempotency_key"])

    def test_rejects_catalog_seed_drift(self):
        changed = catalog()
        changed["scenarios"][0]["seed"] = 9
        with self.assertRaisesRegex(ValueError, "seed does not match"):
            MODULE.materialize_request(contract(), changed)

    def test_accepts_exact_registry_prerelease_versions(self):
        changed = contract()
        changed["target"]["stack_versions"]["database"] = "0.11.0-next.5"
        MODULE.validate_contract(changed)

    def test_materializes_the_release_control_manual_v2_contract_without_an_origin_step(self):
        request = MODULE.materialize_request(manual_v2_contract(), manual_v2_catalog())

        self.assertEqual(
            request["run_contract"]["target"]["stack"]["stack_versions"],
            {"harness": "1.9.0", "state": "0.22.1"},
        )
        self.assertEqual(request["run_contract"]["runner"]["version"], "0.1.0-experimental")
        self.assertEqual(request["run_contract"]["selected_cases"][0]["scenario_id"], "direct_answer")

    def test_rejects_a_manual_v2_contract_when_the_resolved_stack_digest_is_tampered(self):
        changed = manual_v2_contract()
        changed["target"]["stack"]["resolution_sha256"] = f"sha256:{'0' * 64}"

        with self.assertRaisesRegex(ValueError, "resolution_sha256 does not match"):
            MODULE.validate_contract(changed)

    def test_verify_lock_records_engine_managed_workers_from_the_pinned_runtime(self):
        versions = {"harness": "1.9.0", "iii-observability": "0.22.1", "state": "0.22.1"}
        changed = manual_v2_contract_with_versions(versions)
        with tempfile.TemporaryDirectory() as directory:
            lock_path = Path(directory) / "iii.lock"
            write_lock(
                lock_path,
                {
                    "harness": {"version": "1.9.0", "type": "binary"},
                    "iii-observability": {"version": "0.21.8", "type": "engine"},
                    "state": {"version": "0.22.1", "type": "binary"},
                    "harness-e2e": {"version": "0.1.0-experimental", "type": "binary"},
                },
            )
            manifest = MODULE.verify_lock(changed, lock_path)

        engine_worker = manifest["verification"]["engine_managed"]["workers"]["iii-observability"]
        self.assertEqual(engine_worker["declared_version"], "0.22.1")
        self.assertEqual(engine_worker["observed_version"], "0.21.8")
        self.assertEqual(manifest["verification"]["registry_pins"]["observed_versions"]["state"], "0.22.1")

    def test_verify_lock_still_rejects_a_registry_worker_version_drift(self):
        versions = {"harness": "1.9.0", "iii-observability": "0.22.1", "state": "0.22.1"}
        changed = manual_v2_contract_with_versions(versions)
        with tempfile.TemporaryDirectory() as directory:
            lock_path = Path(directory) / "iii.lock"
            write_lock(
                lock_path,
                {
                    "harness": {"version": "1.9.0", "type": "binary"},
                    "iii-observability": {"version": "0.21.8", "type": "engine"},
                    "state": {"version": "0.22.0", "type": "binary"},
                    "harness-e2e": {"version": "0.1.0-experimental", "type": "binary"},
                },
            )
            with self.assertRaisesRegex(ValueError, "stack_version_mismatch: state"):
                MODULE.verify_lock(changed, lock_path)

    def test_verify_lock_does_not_bypass_an_engine_managed_name_with_a_registry_record(self):
        versions = {"harness": "1.9.0", "iii-observability": "0.22.1", "state": "0.22.1"}
        changed = manual_v2_contract_with_versions(versions)
        with tempfile.TemporaryDirectory() as directory:
            lock_path = Path(directory) / "iii.lock"
            write_lock(
                lock_path,
                {
                    "harness": {"version": "1.9.0", "type": "binary"},
                    "iii-observability": {"version": "0.21.8", "type": "binary"},
                    "state": {"version": "0.22.1", "type": "binary"},
                    "harness-e2e": {"version": "0.1.0-experimental", "type": "binary"},
                },
            )
            with self.assertRaisesRegex(ValueError, "stack_version_mismatch: iii-observability"):
                MODULE.verify_lock(changed, lock_path)

    def test_packages_raw_file_digests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "results.json").write_text("{}\n")
            manifest = MODULE.package_bundle(root, contract(), {"run_id": 7, "run_attempt": 1})
        self.assertEqual(manifest["terminal_payload"], "results.json")
        self.assertRegex(manifest["files"][0]["sha256"], r"^sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
