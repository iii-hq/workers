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


def canonical_sha256(value: Any) -> str:
    return f"sha256:{hashlib.sha256(canonical(value).encode()).hexdigest()}"


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


def require_version_map(value: Any, label: str) -> dict[str, str]:
    if not isinstance(value, dict) or not value:
        raise ValueError(f"{label} must be a non-empty object")
    result: dict[str, str] = {}
    for worker, version in value.items():
        if not isinstance(worker, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", worker):
            raise ValueError(f"{label} contains an invalid worker name")
        if not isinstance(version, str) or not VERSION.fullmatch(version):
            raise ValueError(f"{label} contains an invalid version for {worker}")
        result[worker] = version
    return result


def require_positive_integer(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise ValueError(f"{label} must be a positive integer")
    return value


def v2_target_member(contract: dict[str, Any], target: dict[str, Any], name: str) -> Any:
    """Accept the early top-level draft while emitting target-scoped v2 contracts."""
    return target.get(name) if name in target else contract.get(name)


def validate_v2_contract(contract: dict[str, Any], target: dict[str, Any], runner: dict[str, Any]) -> None:
    stack = v2_target_member(contract, target, "stack")
    if not isinstance(stack, dict):
        raise ValueError("target.stack must be an object")
    requested = require_version_map(stack.get("requested_versions"), "target.stack.requested_versions")
    resolved = require_version_map(stack.get("resolved_versions"), "target.stack.resolved_versions")
    if requested != resolved:
        raise ValueError("target stack requested_versions must equal resolved_versions")
    if resolved != target["stack_versions"]:
        raise ValueError("target stack resolved_versions must match target.stack_versions")
    resolution_sha256 = require_digest(stack.get("resolution_sha256"), "target.stack.resolution_sha256")
    if resolution_sha256 != canonical_sha256(resolved):
        raise ValueError("target stack resolution_sha256 does not match resolved_versions")
    if resolution_sha256 != target["stack_digest"]:
        raise ValueError("target stack resolution_sha256 must match target.stack_digest")

    origin = v2_target_member(contract, target, "origin")
    origin_worker = None
    if origin is not None:
        if not isinstance(origin, dict):
            raise ValueError("target.origin must be an object or null")
        require_uuid(origin.get("operation_id"), "target.origin.operation_id")
        require_uuid(origin.get("step_id"), "target.origin.step_id")
        origin_worker = require_text(origin.get("worker"), "target.origin.worker")
        origin_version = require_text(origin.get("version"), "target.origin.version")
        if resolved.get(origin_worker) != origin_version:
            raise ValueError("target origin worker/version must be present in the resolved stack")
        origin_sha = require_text(origin.get("source_sha"), "target.origin.source_sha")
        if not re.fullmatch(r"[0-9a-f]{40}", origin_sha):
            raise ValueError("target.origin.source_sha must be a full lowercase git SHA")
        require_positive_integer(origin.get("release_run_id"), "target.origin.release_run_id")
        require_positive_integer(origin.get("release_run_attempt"), "target.origin.release_run_attempt")

    base = v2_target_member(contract, target, "base")
    if not isinstance(base, dict) or base.get("kind") not in {"deployment", "snapshot"}:
        raise ValueError("target.base.kind must be deployment or snapshot")
    require_uuid(base.get("id"), "target.base.id")

    provenance = stack.get("provenance")
    if not isinstance(provenance, list) or len(provenance) != len(resolved):
        raise ValueError("target.stack.provenance must describe every resolved worker")
    provenance_workers: list[str] = []
    for index, item in enumerate(provenance):
        if not isinstance(item, dict):
            raise ValueError(f"target.stack.provenance[{index}] must be an object")
        worker = require_text(item.get("worker"), f"target.stack.provenance[{index}].worker")
        version = require_text(item.get("version"), f"target.stack.provenance[{index}].version")
        if resolved.get(worker) != version:
            raise ValueError(f"target stack provenance does not match resolved version for {worker}")
        provenance_workers.append(worker)
        source_sha = item.get("source_sha")
        if source_sha is not None and (not isinstance(source_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", source_sha)):
            raise ValueError(f"target.stack.provenance[{index}].source_sha is invalid")
        for field in ("operation_id", "step_id"):
            if item.get(field) is not None:
                require_uuid(item[field], f"target.stack.provenance[{index}].{field}")
        run_id = item.get("release_run_id")
        run_attempt = item.get("release_run_attempt")
        if (run_id is None) != (run_attempt is None):
            raise ValueError("target stack provenance release run id and attempt must be paired")
        if run_id is not None:
            require_positive_integer(run_id, f"target.stack.provenance[{index}].release_run_id")
            require_positive_integer(run_attempt, f"target.stack.provenance[{index}].release_run_attempt")
    if provenance_workers != sorted(provenance_workers) or len(set(provenance_workers)) != len(provenance_workers):
        raise ValueError("target.stack.provenance must be unique and ordered by worker")
    if origin_worker is not None:
        origin_provenance = next((item for item in provenance if item.get("worker") == origin_worker), None)
        if not origin_provenance or any(
            origin_provenance.get(field) != origin.get(field)
            for field in (
                "worker",
                "version",
                "source_sha",
                "operation_id",
                "step_id",
                "release_run_id",
                "release_run_attempt",
            )
        ):
            raise ValueError("target origin must match its stack provenance entry")

    runtime = contract.get("runtime")
    if not isinstance(runtime, dict):
        raise ValueError("runtime must be an object")
    cli = runtime.get("cli")
    if not isinstance(cli, dict):
        raise ValueError("runtime.cli must be an object")
    cli_version = require_text(cli.get("version"), "runtime.cli.version")
    if not VERSION.fullmatch(cli_version):
        raise ValueError("runtime.cli.version must be an exact version")
    runtime_versions = require_version_map(runtime.get("stack_versions"), "runtime.stack_versions")
    runtime_digest = require_digest(runtime.get("stack_digest"), "runtime.stack_digest")
    if runtime_digest != canonical_sha256(runtime_versions):
        raise ValueError("runtime.stack_digest does not match runtime.stack_versions")
    conflicts = sorted(
        worker for worker in runtime_versions.keys() & resolved.keys() if runtime_versions[worker] != resolved[worker]
    )
    if conflicts:
        raise ValueError(f"runtime and target stack pins conflict: {', '.join(conflicts)}")

    runner_ref = require_text(runner.get("registry_ref"), "runner.registry_ref")
    if not VERSION.fullmatch(runner_ref):
        raise ValueError("runner.registry_ref must be an exact version for schema v2")
    runner_revision = runner.get("revision")
    if runner_revision is not None and (
        not isinstance(runner_revision, str) or not re.fullmatch(r"[0-9a-f]{40}", runner_revision)
    ):
        raise ValueError("runner.revision must be a full lowercase git SHA")
    if runner.get("catalog_sha256") is not None:
        require_digest(runner["catalog_sha256"], "runner.catalog_sha256")

    security = contract.get("security")
    if not isinstance(security, dict):
        raise ValueError("security must be an object")
    audience = require_text(security.get("oidc_audience"), "security.oidc_audience")
    if not re.fullmatch(r"[A-Za-z0-9._:/-]+", audience):
        raise ValueError("security.oidc_audience contains unsupported characters")


def validate_contract(contract: dict[str, Any]) -> dict[str, Any]:
    schema_version = contract.get("schema_version")
    if schema_version not in {1, 2}:
        raise ValueError("execution contract schema_version must be 1 or 2")
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
    if schema_version == 1:
        source_sha = require_text(target.get("source_sha"), "target.source_sha")
        if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
            raise ValueError("target.source_sha must be a full lowercase git SHA")
        require_uuid(target.get("deployment_id"), "target.deployment_id")
        stack = require_version_map(target.get("stack_versions"), "target.stack_versions")
        if stack.get("harness") != target.get("version"):
            raise ValueError("target version must match stack_versions.harness")
        require_digest(target.get("stack_digest"), "target.stack_digest")
    else:
        if target.get("source_sha") is not None and not re.fullmatch(r"[0-9a-f]{40}", target["source_sha"]):
            raise ValueError("target.source_sha must be a full lowercase git SHA when present")
        if target.get("deployment_id") is not None:
            require_uuid(target["deployment_id"], "target.deployment_id")
        if target.get("stack_versions") is not None:
            stack = require_version_map(target["stack_versions"], "target.stack_versions")
            if stack.get("harness") != target.get("version"):
                raise ValueError("target version must match stack_versions.harness")
        if target.get("stack_digest") is not None:
            require_digest(target["stack_digest"], "target.stack_digest")

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
    if schema_version == 2:
        validate_v2_contract(contract, target, runner)
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
    if contract["schema_version"] == 2:
        expected_runner = contract["runner"]
        if runner.get("name") != expected_runner["registry_worker"] or runner.get("version") != expected_runner["registry_ref"]:
            raise ValueError("scenario catalog runner does not match the exact runner pin")
        if expected_runner.get("revision") is not None and runner.get("revision") != expected_runner["revision"]:
            raise ValueError("scenario catalog runner revision does not match the contract")
        if expected_runner.get("catalog_sha256") is not None and catalog_sha256 != expected_runner["catalog_sha256"]:
            raise ValueError("scenario catalog digest does not match the contract")
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

    target_stack = contract["target"]["stack_versions"]
    target_stack_digest = contract["target"]["stack_digest"]
    if contract["schema_version"] == 2:
        stack = v2_target_member(contract, contract["target"], "stack")
        target_stack = stack["resolved_versions"]
        target_stack_digest = stack["resolution_sha256"]
    run_contract = {
        "schema_version": 1,
        "mode": {"environment": "demonstration", "decision": "observe_only"},
        "target": {
            "application": "harness",
            "version": contract["target"]["version"],
            "stack": {
                "mode": "registry",
                "stack_versions": target_stack,
                "stack_lock_digest": target_stack_digest,
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
            "deployment_id": contract["target"].get("deployment_id") or contract["campaign_id"],
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


def verify_lock(contract: dict[str, Any], lock_path: Path) -> dict[str, Any]:
    validate_contract(contract)
    try:
        import yaml
    except ImportError as error:  # pragma: no cover - CI installs PyYAML explicitly.
        raise ValueError("PyYAML is required to verify iii.lock") from error

    lock = yaml.safe_load(lock_path.read_text()) or {}
    workers = lock.get("workers") if isinstance(lock, dict) else None
    if not isinstance(workers, dict):
        raise ValueError("iii.lock workers must be an object")
    observed = {
        str(worker): str(record.get("version"))
        for worker, record in workers.items()
        if isinstance(record, dict) and isinstance(record.get("version"), str)
    }

    target = contract["target"]
    if contract["schema_version"] == 2:
        stack = v2_target_member(contract, target, "stack")
        expected_target = stack["resolved_versions"]
        target_digest = stack["resolution_sha256"]
        runtime = contract["runtime"]
        expected_runtime = runtime["stack_versions"]
        runtime_digest = runtime["stack_digest"]
    else:
        expected_target = target["stack_versions"]
        target_digest = target["stack_digest"]
        expected_runtime = {}
        runtime_digest = None

    expected = {**expected_runtime, **expected_target, contract["runner"]["registry_worker"]: contract["runner"]["registry_ref"]}
    mismatches = [
        f"{worker}: expected {version}, resolved {observed.get(worker, 'missing')}"
        for worker, version in sorted(expected.items())
        if observed.get(worker) != version
    ]
    if mismatches:
        raise ValueError("stack_version_mismatch: " + "; ".join(mismatches))

    target_stack = v2_target_member(contract, target, "stack") if contract["schema_version"] == 2 else None
    return {
        "schema": "e2e-stack-manifest/v1",
        "contract_schema_version": contract["schema_version"],
        "target": {
            "application": target["application"],
            "version": target["version"],
            "requested_versions": target_stack["requested_versions"] if target_stack else expected_target,
            "resolved_versions": expected_target,
            "observed_versions": {worker: observed[worker] for worker in sorted(expected_target)},
            "resolution_sha256": target_digest,
            "provenance": target_stack.get("provenance", []) if target_stack else [],
        },
        "runtime": {
            "cli": contract.get("runtime", {}).get("cli"),
            "stack_versions": expected_runtime,
            "observed_versions": {worker: observed[worker] for worker in sorted(expected_runtime)},
            "stack_digest": runtime_digest,
        },
        "runner": {
            **contract["runner"],
            "observed_version": observed[contract["runner"]["registry_worker"]],
        },
        "lock": {
            "sha256": f"sha256:{hashlib.sha256(lock_path.read_bytes()).hexdigest()}",
            "worker_count": len(observed),
            "resolved_versions": dict(sorted(observed.items())),
        },
        "origin": v2_target_member(contract, target, "origin") if contract["schema_version"] == 2 else None,
        "base": v2_target_member(contract, target, "base") if contract["schema_version"] == 2 else None,
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
        "execution_contract_sha256": canonical_sha256(contract),
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
    digest = commands.add_parser("digest")
    digest.add_argument("--contract", type=Path, required=True)
    materialize = commands.add_parser("materialize")
    materialize.add_argument("--contract", type=Path, required=True)
    materialize.add_argument("--catalog", type=Path, required=True)
    materialize.add_argument("--output", type=Path, required=True)
    lock = commands.add_parser("verify-lock")
    lock.add_argument("--contract", type=Path, required=True)
    lock.add_argument("--lock", type=Path, required=True)
    lock.add_argument("--output", type=Path, required=True)
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
        elif args.command == "digest":
            print(canonical_sha256(contract))
        elif args.command == "materialize":
            request = materialize_request(contract, load_object(args.catalog, "scenario catalog"))
            args.output.write_text(json.dumps(request, indent=2, sort_keys=True) + "\n")
        elif args.command == "verify-lock":
            manifest = verify_lock(contract, args.lock)
            args.output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
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
