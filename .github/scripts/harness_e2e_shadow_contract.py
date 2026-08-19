#!/usr/bin/env python3
"""Validate and materialize Release Control shadow E2E contracts."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any
from uuid import UUID


SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
VERSION = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def load_object(path: Path, label: str) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be a JSON object")
    return value


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def require_uuid(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a UUID")
    parsed = UUID(value)
    if str(parsed) != value.lower():
        raise ValueError(f"{label} must use canonical UUID form")
    return str(parsed)


def require_text(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label} must be a non-empty string")
    return value


def require_digest(value: Any, label: str) -> str:
    value = require_text(value, label)
    if not SHA256.fullmatch(value):
        raise ValueError(f"{label} must be sha256:<64 lowercase hex>")
    return value


def validate_contract(contract: dict[str, Any]) -> dict[str, Any]:
    if contract.get("schema_version") != 1:
        raise ValueError("execution contract schema_version must be 1")
    require_uuid(contract.get("campaign_id"), "campaign_id")
    require_uuid(contract.get("execution_id"), "execution_id")
    attempt = contract.get("attempt")
    if not isinstance(attempt, int) or attempt < 1:
        raise ValueError("attempt must be a positive integer")
    key = require_text(contract.get("idempotency_key"), "idempotency_key")
    if not re.fullmatch(r"rc:d0:[0-9a-f]{64}", key):
        raise ValueError("idempotency_key must be rc:d0:<sha256>")

    target = contract.get("target")
    if not isinstance(target, dict) or target.get("application") != "harness":
        raise ValueError("target application must be harness")
    require_text(target.get("version"), "target.version")
    source_sha = require_text(target.get("source_sha"), "target.source_sha")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("target.source_sha must be a full lowercase git SHA")
    require_uuid(target.get("deployment_id"), "target.deployment_id")
    stack = target.get("stack_versions")
    if not isinstance(stack, dict) or not stack:
        raise ValueError("target.stack_versions must be a non-empty object")
    for worker, version in stack.items():
        if not isinstance(worker, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", worker):
            raise ValueError("target.stack_versions contains an invalid worker name")
        if not isinstance(version, str) or not VERSION.fullmatch(version):
            raise ValueError(f"target.stack_versions contains an invalid version for {worker}")
    if stack.get("harness") != target.get("version"):
        raise ValueError("target version must match stack_versions.harness")
    require_digest(target.get("stack_digest"), "target.stack_digest")

    plan = contract.get("plan")
    if not isinstance(plan, dict):
        raise ValueError("plan must be an object")
    require_uuid(plan.get("id"), "plan.id")
    revision = plan.get("revision")
    if not isinstance(revision, int) or revision < 1:
        raise ValueError("plan.revision must be a positive integer")
    require_digest(plan.get("sha256"), "plan.sha256")
    definition = plan.get("definition")
    if not isinstance(definition, dict):
        raise ValueError("plan.definition must be an object")
    if definition.get("mode") != "demonstrative" or definition.get("entrypoint") != "e2e::run":
        raise ValueError("plan must be demonstrative and use e2e::run")
    scenarios = definition.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios or len(set(scenarios)) != len(scenarios):
        raise ValueError("plan scenarios must be a non-empty unique list")
    if not all(isinstance(item, str) and item for item in scenarios):
        raise ValueError("plan scenarios must contain non-empty strings")
    for field in ("runs", "seed", "technicalRetries", "progressIntervalSeconds"):
        if not isinstance(definition.get(field), int) or definition[field] < 1:
            raise ValueError(f"plan.definition.{field} must be a positive integer")
    for role in ("subject", "judge"):
        identity = definition.get(role)
        if not isinstance(identity, dict):
            raise ValueError(f"plan.definition.{role} must be an object")
        require_text(identity.get("provider"), f"plan.definition.{role}.provider")
        require_text(identity.get("model"), f"plan.definition.{role}.model")

    runner = contract.get("runner")
    if not isinstance(runner, dict):
        raise ValueError("runner must be an object")
    if require_text(runner.get("registry_worker"), "runner.registry_worker") != "harness-e2e":
        raise ValueError("runner.registry_worker must be harness-e2e")
    runner_ref = require_text(runner.get("registry_ref"), "runner.registry_ref")
    if not re.fullmatch(r"[A-Za-z0-9._-]+", runner_ref):
        raise ValueError("runner.registry_ref is invalid")
    return contract


def materialize_request(contract: dict[str, Any], catalog: dict[str, Any]) -> dict[str, Any]:
    validate_contract(contract)
    if catalog.get("schema") != "e2e-scenario-catalog/v1":
        raise ValueError("unsupported scenario catalog schema")
    runner = catalog.get("runner")
    if not isinstance(runner, dict):
        raise ValueError("scenario catalog has no runner identity")
    for field in ("name", "version", "revision"):
        require_text(runner.get(field), f"catalog.runner.{field}")
    catalog_sha256 = require_digest(catalog.get("catalog_sha256"), "catalog.catalog_sha256")
    descriptors = catalog.get("scenarios")
    if not isinstance(descriptors, list):
        raise ValueError("scenario catalog scenarios must be a list")
    by_id: dict[str, dict[str, Any]] = {}
    for item in descriptors:
        if isinstance(item, dict) and isinstance(item.get("scenario_id"), str):
            by_id[item["scenario_id"]] = item

    definition = contract["plan"]["definition"]
    selected_cases: list[dict[str, Any]] = []
    for scenario_id in definition["scenarios"]:
        descriptor = by_id.get(scenario_id)
        if descriptor is None:
            raise ValueError(f"scenario catalog is missing {scenario_id}")
        scenario_version = descriptor.get("scenario_version")
        seed = descriptor.get("seed")
        if not isinstance(scenario_version, int) or scenario_version < 1:
            raise ValueError(f"scenario {scenario_id} has an invalid version")
        if seed != definition["seed"]:
            raise ValueError(f"scenario {scenario_id} seed does not match the plan")
        selected_cases.append(
            {
                "scenario_id": scenario_id,
                "scenario_version": scenario_version,
                "case_id": require_text(descriptor.get("case_id"), f"{scenario_id}.case_id"),
                "seed": seed,
                "inputs_sha256": require_digest(
                    descriptor.get("inputs_sha256"), f"{scenario_id}.inputs_sha256"
                ),
                "contract_sha256": require_digest(
                    descriptor.get("contract_sha256"), f"{scenario_id}.contract_sha256"
                ),
            }
        )

    run_contract = {
        "schema_version": 1,
        "mode": {"environment": "demonstration", "decision": "observe_only"},
        "target": {
            "application": "harness",
            "version": contract["target"]["version"],
            "stack": {
                "mode": "registry",
                "stack_versions": contract["target"]["stack_versions"],
                "stack_lock_digest": contract["target"]["stack_digest"],
            },
        },
        "plan": {
            "id": contract["plan"]["id"],
            "revision": str(contract["plan"]["revision"]),
            "sha256": contract["plan"]["sha256"],
            "catalog_sha256": catalog_sha256,
        },
        "runner": runner,
        "attempt": contract["attempt"],
        "selected_cases": selected_cases,
        "correlation": {
            "system": "release-control",
            "deployment_id": contract["target"]["deployment_id"],
            "operation_id": contract["campaign_id"],
        },
    }
    return {
        "idempotency_key": contract["idempotency_key"],
        "label": f"{definition['label']} · Harness {contract['target']['version']}",
        "lane": definition["lane"],
        "model": definition["subject"]["model"],
        "provider": definition["subject"]["provider"],
        "judge_model": definition["judge"]["model"],
        "judge_provider": definition["judge"]["provider"],
        "scenarios": definition["scenarios"],
        "runs": definition["runs"],
        "seed": definition["seed"],
        "rotating_seeds": [],
        "technical_retries": definition["technicalRetries"],
        "progress_interval_seconds": definition["progressIntervalSeconds"],
        "run_contract": run_contract,
    }


def package_bundle(root: Path, contract: dict[str, Any], workflow: dict[str, Any]) -> dict[str, Any]:
    validate_contract(contract)
    files = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.name == "bundle-manifest.json":
            continue
        relative = path.relative_to(root).as_posix()
        payload = path.read_bytes()
        files.append(
            {
                "path": relative,
                "sha256": f"sha256:{hashlib.sha256(payload).hexdigest()}",
                "size_bytes": len(payload),
            }
        )
    terminal = root / "results.json"
    failure = root / "failure.json"
    return {
        "schema": "e2e-observation-bundle/v1",
        "campaign_id": contract["campaign_id"],
        "execution_id": contract["execution_id"],
        "attempt": contract["attempt"],
        "workflow": workflow,
        "terminal_payload": "results.json" if terminal.is_file() else None,
        "failure_payload": "failure.json" if failure.is_file() else None,
        "files": files,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate")
    validate.add_argument("--contract", type=Path, required=True)
    materialize = commands.add_parser("materialize")
    materialize.add_argument("--contract", type=Path, required=True)
    materialize.add_argument("--catalog", type=Path, required=True)
    materialize.add_argument("--output", type=Path, required=True)
    package = commands.add_parser("package")
    package.add_argument("--root", type=Path, required=True)
    package.add_argument("--contract", type=Path, required=True)
    package.add_argument("--workflow", required=True)
    package.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        contract = validate_contract(load_object(args.contract, "execution contract"))
        if args.command == "validate":
            print(canonical(contract))
        elif args.command == "materialize":
            request = materialize_request(contract, load_object(args.catalog, "scenario catalog"))
            args.output.write_text(json.dumps(request, indent=2, sort_keys=True) + "\n")
        else:
            workflow = json.loads(args.workflow)
            if not isinstance(workflow, dict):
                raise ValueError("workflow must be a JSON object")
            manifest = package_bundle(args.root, contract, workflow)
            args.output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        return 0
    except (ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
