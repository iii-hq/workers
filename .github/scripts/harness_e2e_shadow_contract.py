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
CAMPAIGN_EXECUTION_KINDS = {
    "harness_turn",
    "scripted_dialogue",
    "composite_flow",
    "adaptive_flow",
    "fault_injection",
}
DIFFICULTY_WEIGHTS = {
    "L0": 1,
    "L1": 1,
    "L2": 2,
    "L3": 3,
    "L4": 4,
    "L5": 5,
}

# These workers are hosted by the pinned iii engine rather than installed from
# the Registry. `iii worker add` intentionally reports them as built-in, so a
# Registry resolution of a transitive dependency cannot force their internal
# version. Their observed lock entries remain evidence, while the exact CLI
# pin is the reproducibility boundary for their runtime implementation.
ENGINE_MANAGED_WORKERS = frozenset(
    {
        "configuration",
        "iii-observability",
        "iii-state",
        "iii-stream",
        "iii-worker-manager",
        "iii-worker-ops",
    }
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


def require_nonnegative_integer(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError(f"{label} must be a non-negative integer")
    return value


def validate_identity(definition: dict[str, Any], role: str) -> None:
    identity = definition.get(role)
    if not isinstance(identity, dict):
        raise ValueError(f"plan.definition.{role} must be an object")
    require_text(identity.get("provider"), f"plan.definition.{role}.provider")
    require_text(identity.get("model"), f"plan.definition.{role}.model")


def validate_campaign_definition(definition: dict[str, Any]) -> None:
    if definition.get("entrypoint") != "e2e::run":
        raise ValueError("campaign plan must use e2e::run")
    require_text(definition.get("label"), "plan.definition.label")
    lane = require_text(definition.get("lane"), "plan.definition.lane")
    if lane not in {"manual", "daily", "weekly", "post-release"}:
        raise ValueError("campaign lane must be manual, daily, weekly, or post-release")
    if definition.get("failurePolicy") != "advisory":
        raise ValueError("campaign failurePolicy must be advisory")
    validate_identity(definition, "subject")
    validate_identity(definition, "judge")

    manifest = definition.get("manifest")
    if not isinstance(manifest, dict):
        raise ValueError("plan.definition.manifest must be an object")
    require_text(manifest.get("id"), "plan.definition.manifest.id")
    require_digest(manifest.get("sha256"), "plan.definition.manifest.sha256")

    scoring = definition.get("scoring")
    if not isinstance(scoring, dict) or scoring.get("profile") != "difficulty-weighted-v1":
        raise ValueError("campaign scoring profile must be difficulty-weighted-v1")
    require_digest(scoring.get("sha256"), "plan.definition.scoring.sha256")

    catalog = definition.get("catalog")
    if not isinstance(catalog, dict):
        raise ValueError("plan.definition.catalog must be an object")
    require_text(catalog.get("revision"), "plan.definition.catalog.revision")
    require_digest(catalog.get("sha256"), "plan.definition.catalog.sha256")
    require_positive_integer(catalog.get("seed"), "plan.definition.catalog.seed")

    groups = definition.get("groups")
    if not isinstance(groups, list) or not groups:
        raise ValueError("campaign groups must be a non-empty array")
    seen: set[str] = set()
    for index, group in enumerate(groups):
        label = f"plan.definition.groups[{index}]"
        if not isinstance(group, dict):
            raise ValueError(f"{label} must be an object")
        group_id = require_text(group.get("id"), f"{label}.id")
        if not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", group_id) or group_id in seen:
            raise ValueError("campaign group ids must be unique kebab-case values")
        seen.add(group_id)
        execution_kind = require_text(group.get("executionKind"), f"{label}.executionKind")
        if execution_kind not in CAMPAIGN_EXECUTION_KINDS:
            raise ValueError(f"{label}.executionKind is unsupported")
        runs = require_positive_integer(group.get("runs"), f"{label}.runs")
        retries = require_nonnegative_integer(
            group.get("technicalRetries"), f"{label}.technicalRetries"
        )
        tier = require_text(group.get("difficultyTier"), f"{label}.difficultyTier")
        expected_weight = DIFFICULTY_WEIGHTS.get(tier)
        if expected_weight is None or group.get("difficultyWeight") != expected_weight:
            raise ValueError(f"{label}.difficultyWeight does not match {tier}")
        scenarios = group.get("scenarios")
        if execution_kind == "fault_injection":
            if scenarios not in (None, []):
                raise ValueError(f"{label}.scenarios must be empty for fault injection")
            if retries != 0 or runs < 3 or group.get("soakMinutes") != 60:
                raise ValueError(
                    f"{label} fault injection requires runs>=3, technicalRetries=0, and soakMinutes=60"
                )
            require_text(group.get("faultProfile"), f"{label}.faultProfile")
            require_text(group.get("faultScenario"), f"{label}.faultScenario")
        else:
            if not isinstance(scenarios, list) or not scenarios or len(set(scenarios)) != len(scenarios):
                raise ValueError(f"{label}.scenarios must be a non-empty unique array")
            if not all(isinstance(item, str) and item for item in scenarios):
                raise ValueError(f"{label}.scenarios must contain non-empty strings")


def campaign_matrix(contract: dict[str, Any]) -> dict[str, Any]:
    validate_contract(contract)
    definition = contract["plan"]["definition"]
    if definition["mode"] != "campaign":
        return {
            "include": [
                {
                    "group_id": "legacy",
                    "execution_kind": "demonstrative",
                    "runs_on": ["ubuntu-latest"],
                }
            ]
        }
    include = []
    for group in definition["groups"]:
        is_fault = group["executionKind"] == "fault_injection"
        include.append(
            {
                "group_id": group["id"],
                "execution_kind": group["executionKind"],
                "runs_on": ["self-hosted", "harness-e2e"] if is_fault else ["ubuntu-latest"],
            }
        )
    return {"include": include}


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
    if schema_version not in {1, 2, 3}:
        raise ValueError("execution contract schema_version must be 1, 2, or 3")
    require_uuid(contract.get("campaign_id"), "campaign_id")
    require_uuid(contract.get("execution_id"), "execution_id")
    attempt = contract.get("attempt")
    if not isinstance(attempt, int) or attempt < 1:
        raise ValueError("attempt must be a positive integer")
    key = require_text(contract.get("idempotency_key"), "idempotency_key")
    if not re.fullmatch(r"rc:(?:d0|e2e):[0-9a-f]{64}", key):
        raise ValueError("idempotency_key must be rc:d0:<sha256> or rc:e2e:<sha256>")

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
    if schema_version == 3:
        if definition.get("mode") != "campaign":
            raise ValueError("schema v3 plan mode must be campaign")
        validate_campaign_definition(definition)
    else:
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
        validate_identity(definition, "subject")
        validate_identity(definition, "judge")

    runner = contract.get("runner")
    if not isinstance(runner, dict):
        raise ValueError("runner must be an object")
    if require_text(runner.get("registry_worker"), "runner.registry_worker") != "harness-e2e":
        raise ValueError("runner.registry_worker must be harness-e2e")
    runner_ref = require_text(runner.get("registry_ref"), "runner.registry_ref")
    if not re.fullmatch(r"[A-Za-z0-9._-]+", runner_ref):
        raise ValueError("runner.registry_ref is invalid")
    if schema_version in {2, 3}:
        validate_v2_contract(contract, target, runner)
    if schema_version == 3:
        definition = contract["plan"]["definition"]
        require_digest(runner.get("catalog_sha256"), "runner.catalog_sha256")
        require_digest(runner.get("manifest_sha256"), "runner.manifest_sha256")
        require_digest(
            runner.get("scoring_profile_sha256"),
            "runner.scoring_profile_sha256",
        )
        require_digest(runner.get("assets_sha256"), "runner.assets_sha256")
        if runner.get("catalog_sha256") != definition["catalog"]["sha256"]:
            raise ValueError("runner catalog digest must match the campaign definition")
        if runner.get("manifest_sha256") != definition["manifest"]["sha256"]:
            raise ValueError("runner manifest digest must match the campaign definition")
        if runner.get("scoring_profile_sha256") != definition["scoring"]["sha256"]:
            raise ValueError("runner scoring digest must match the campaign definition")
    return contract


def materialize_request(
    contract: dict[str, Any], catalog: dict[str, Any], group_id: str | None = None
) -> dict[str, Any]:
    validate_contract(contract)
    if catalog.get("schema") != "e2e-scenario-catalog/v1":
        raise ValueError("unsupported scenario catalog schema")
    runner = catalog.get("runner")
    if not isinstance(runner, dict):
        raise ValueError("scenario catalog has no runner identity")
    for field in ("name", "version", "revision"):
        require_text(runner.get(field), f"catalog.runner.{field}")
    catalog_sha256 = require_digest(catalog.get("catalog_sha256"), "catalog.catalog_sha256")
    if contract["schema_version"] in {2, 3}:
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
    group = None
    scenarios = definition.get("scenarios")
    runs = definition.get("runs")
    technical_retries = definition.get("technicalRetries")
    plan_seed = definition.get("seed")
    label = definition["label"]
    if definition["mode"] == "campaign":
        group = next(
            (item for item in definition["groups"] if item["id"] == group_id), None
        )
        if group is None:
            raise ValueError("a valid campaign group id is required")
        if group["executionKind"] == "fault_injection":
            raise ValueError("fault injection groups are executed by the protected supervisor")
        scenarios = group["scenarios"]
        runs = group["runs"]
        technical_retries = group["technicalRetries"]
        plan_seed = definition["catalog"]["seed"]
        label = f"{definition['label']} · {group['id']}"
    selected_cases: list[dict[str, Any]] = []
    for scenario_id in scenarios:
        descriptor = by_id.get(scenario_id)
        if descriptor is None:
            raise ValueError(f"scenario catalog is missing {scenario_id}")
        scenario_version = descriptor.get("scenario_version")
        descriptor_seed = descriptor.get("seed")
        if not isinstance(scenario_version, int) or scenario_version < 1:
            raise ValueError(f"scenario {scenario_id} has an invalid version")
        if descriptor_seed != plan_seed:
            raise ValueError(f"scenario {scenario_id} seed does not match the plan")
        selected_cases.append(
            {
                "scenario_id": scenario_id,
                "scenario_version": scenario_version,
                "case_id": require_text(descriptor.get("case_id"), f"{scenario_id}.case_id"),
                "seed": descriptor_seed,
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
    if contract["schema_version"] in {2, 3}:
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
            "group_id": group_id,
            "manifest_sha256": definition.get("manifest", {}).get("sha256"),
            "scoring_profile": definition.get("scoring"),
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
    request = {
        "label": f"{label} · Harness {contract['target']['version']}",
        "lane": definition["lane"],
        "model": definition["subject"]["model"],
        "provider": definition["subject"]["provider"],
        "judge_model": definition["judge"]["model"],
        "judge_provider": definition["judge"]["provider"],
        "scenarios": scenarios,
        "runs": runs,
        "seed": plan_seed,
        "rotating_seeds": [],
        "technical_retries": technical_retries,
        "progress_interval_seconds": definition.get("progressIntervalSeconds", 15),
        "run_contract": run_contract,
    }
    # The runner validates a D0 key over the fully materialized request,
    # including cases and their contract fingerprints. Those fields only exist
    # after scenarios-list, so the GitHub dispatch key cannot be reused here.
    # This remains deterministic for transport retries of the same catalog.
    request["idempotency_key"] = observation_idempotency_key(request)
    return request


def observation_idempotency_key(request: dict[str, Any]) -> str:
    intent = {
        "run_contract": request["run_contract"],
        "lane": request["lane"],
        "model": request["model"],
        "provider": request["provider"],
        "judge_model": request["judge_model"],
        "judge_provider": request["judge_provider"],
        "scenarios": request["scenarios"],
        "runs": request["runs"],
        "seed": request["seed"],
        "rotating_seeds": request["rotating_seeds"],
        "technical_retries": request["technical_retries"],
    }
    return f"rc:d0:{canonical_sha256(intent).removeprefix('sha256:')}"


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
    if contract["schema_version"] in {2, 3}:
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
    engine_managed = {
        worker
        for worker in expected
        if worker in ENGINE_MANAGED_WORKERS
        and isinstance(workers.get(worker), dict)
        and workers[worker].get("type") == "engine"
        and worker != contract["runner"]["registry_worker"]
    }
    mismatches = [
        f"{worker}: expected {version}, resolved {observed.get(worker, 'missing')}"
        for worker, version in sorted(expected.items())
        if observed.get(worker) != version and worker not in engine_managed
    ]
    if mismatches:
        raise ValueError("stack_version_mismatch: " + "; ".join(mismatches))

    registry_pins = {worker: version for worker, version in expected.items() if worker not in engine_managed}
    engine_managed_evidence = {
        worker: {
            "declared_version": expected[worker],
            "observed_version": observed[worker],
            "lock_type": workers[worker]["type"],
        }
        for worker in sorted(engine_managed)
    }

    target_stack = (
        v2_target_member(contract, target, "stack")
        if contract["schema_version"] in {2, 3}
        else None
    )
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
        "verification": {
            "registry_pins": {
                "expected_versions": dict(sorted(registry_pins.items())),
                "observed_versions": {worker: observed[worker] for worker in sorted(registry_pins)},
            },
            "engine_managed": {
                "runtime_cli": contract.get("runtime", {}).get("cli"),
                "workers": engine_managed_evidence,
            },
        },
        "lock": {
            "sha256": f"sha256:{hashlib.sha256(lock_path.read_bytes()).hexdigest()}",
            "worker_count": len(observed),
            "resolved_versions": dict(sorted(observed.items())),
        },
        "origin": (
            v2_target_member(contract, target, "origin")
            if contract["schema_version"] in {2, 3}
            else None
        ),
        "base": (
            v2_target_member(contract, target, "base")
            if contract["schema_version"] in {2, 3}
            else None
        ),
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
    materialize.add_argument("--group-id")
    matrix = commands.add_parser("matrix")
    matrix.add_argument("--contract", type=Path, required=True)
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
            request = materialize_request(
                contract,
                load_object(args.catalog, "scenario catalog"),
                group_id=args.group_id,
            )
            args.output.write_text(json.dumps(request, indent=2, sort_keys=True) + "\n")
        elif args.command == "matrix":
            print(canonical(campaign_matrix(contract)))
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
