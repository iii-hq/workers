#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402

def normalize_dependencies(raw_deps: Any) -> list[dict[str, Any]]:
    if raw_deps in (None, ""):
        return []
    if isinstance(raw_deps, dict):
        return [{"name": name, "version": version} for name, version in raw_deps.items()]
    if isinstance(raw_deps, list):
        normalized: list[dict[str, Any]] = []
        for dep in raw_deps:
            if isinstance(dep, str):
                normalized.append({"name": dep, "version": ""})
            elif isinstance(dep, dict) and isinstance(dep.get("name"), str):
                normalized.append({"name": dep["name"], "version": dep.get("version") or ""})
            else:
                raise ValueError(
                    "dependency list entries must be strings or {name, version} objects"
                )
        return normalized
    raise ValueError(f"`dependencies` must be a map or list, got {type(raw_deps).__name__}")


def normalize_tags(raw_tags: Any) -> list[str]:
    if raw_tags is None:
        return []
    if not isinstance(raw_tags, list):
        raise ValueError(f"`tags` must be a list, got {type(raw_tags).__name__}")

    normalized: list[str] = []
    seen: set[str] = set()
    for tag in raw_tags:
        if not isinstance(tag, str):
            raise ValueError("tags entries must be strings")
        value = tag.strip().lower()
        if value and value not in seen:
            normalized.append(value)
            seen.add(value)
    return normalized


def derive_registry_function_name(function_id: str, metadata: dict[str, Any] | None) -> str:
    metadata = metadata or {}
    for key in ("registry_name", "name"):
        value = metadata.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return function_id


def _extract_array(payload: dict[str, Any], key: str) -> list[dict[str, Any]]:
    value = payload.get(key, [])
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"`{key}` must be an array")
    return value


def _read_yaml(path: pathlib.Path) -> Any:
    import yaml

    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _schema_or_empty(value: Any) -> dict[str, Any]:
    if value is None:
        return {}
    if isinstance(value, dict):
        return value
    raise ValueError("function schema fields must be objects or null")


def _metadata_or_empty(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _string_or_empty(value: Any) -> str:
    return value if isinstance(value, str) else ""


def _normalize_registry_function(function: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": function.get("name"),
        "description": _string_or_empty(function.get("description")),
        "request_schema": _schema_or_empty(function.get("request_schema")),
        "response_schema": _schema_or_empty(function.get("response_schema")),
        "metadata": _metadata_or_empty(function.get("metadata")),
    }


def _normalize_registry_trigger(trigger: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": trigger.get("name"),
        "description": _string_or_empty(trigger.get("description")),
        "invocation_schema": _schema_or_empty(trigger.get("invocation_schema")),
        "return_schema": _schema_or_empty(trigger.get("return_schema")),
        "metadata": _metadata_or_empty(trigger.get("metadata")),
    }


def _worker_identity(worker: dict[str, Any]) -> str:
    name = worker.get("name")
    if isinstance(name, str) and name.strip():
        return name.strip()
    worker_id = worker.get("id")
    if isinstance(worker_id, str) and worker_id.strip():
        return worker_id.strip()
    return ""


def _baseline_worker_identities(baseline_workers_json: dict[str, Any] | None) -> set[str]:
    if not baseline_workers_json:
        return set()
    return {
        identity
        for worker in _extract_array(baseline_workers_json, "workers")
        if (identity := _worker_identity(worker))
    }


# Workers the engine itself hosts (enabled via the engine config, not
# installed from the registry). A candidate install can flip one on mid-boot
# (e.g. harness enables `iii-stream` for console streaming), which lands it in
# the workers-baseline diff even though its interface is not part of the
# released worker's surface — and its schemas are not this repo's to fix.
ENGINE_BUILTIN_WORKERS = frozenset(
    {
        "configuration",
        "iii-observability",
        "iii-state",
        "iii-stream",
        "iii-worker-manager",
        "iii-worker-ops",
    }
)


def _resolve_target_worker_names(
    *,
    workers: list[dict[str, Any]],
    worker_name: str,
    baseline_workers_json: dict[str, Any] | None,
) -> set[str]:
    """Return engine worker names whose bus functions belong in the publish payload."""
    baseline = _baseline_worker_identities(baseline_workers_json)
    if baseline:
        new_names = {
            identity
            for worker in workers
            if (identity := _worker_identity(worker))
            and identity not in baseline
            and identity not in ENGINE_BUILTIN_WORKERS
        }
        if new_names:
            return new_names

    worker = _match_worker(workers, worker_name)
    matched = _worker_identity(worker)
    if not matched:
        raise ValueError(f"matched worker for {worker_name!r} has no identity")
    return {matched}


def _function_ids_for_workers(
    functions: list[dict[str, Any]], worker_names: set[str]
) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for fn in functions:
        function_id = fn.get("function_id")
        worker_name = fn.get("worker_name")
        if not isinstance(function_id, str) or not function_id:
            continue
        if not isinstance(worker_name, str) or worker_name not in worker_names:
            continue
        if function_id in seen:
            continue
        seen.add(function_id)
        ordered.append(function_id)
    return ordered


def _match_worker(workers: list[dict[str, Any]], worker_name: str) -> dict[str, Any]:
    by_name = [w for w in workers if w.get("name") == worker_name or w.get("id") == worker_name]
    if len(by_name) == 1:
        return by_name[0]

    summary = [
        {"id": w.get("id"), "name": w.get("name"), "internal": w.get("internal")}
        for w in workers
    ]
    raise ValueError(
        f"could not match worker {worker_name!r} exactly: "
        f"{len(by_name)} by name/id, workers={summary}"
    )


def _normalize_registry_trigger_type(trigger_type: dict[str, Any]) -> dict[str, Any]:
    # `engine::triggers::info` exposes the typed schemas under
    # `configuration_schema` (the binding config, registered via the SDK's
    # `.trigger_request_format::<T>()`) and `request_schema` (the delivered-event
    # payload, registered via `.call_request_format::<T>()`). `engine::triggers::list`
    # rows carry neither — they must be enriched from `::info` first (see
    # collect_worker_interface.enrich_trigger_types_with_schemas) or these
    # collapse to the empty `{}` that renders as 'unknown' in the registry.
    return {
        "name": _string_or_empty(trigger_type.get("id")),
        "description": _string_or_empty(trigger_type.get("description")),
        "invocation_schema": _schema_or_empty(trigger_type.get("configuration_schema")),
        "return_schema": _schema_or_empty(trigger_type.get("request_schema")),
        "metadata": {},
    }


def normalize_worker_interface(
    *,
    worker_name: str,
    workers_json: dict[str, Any],
    functions_json: dict[str, Any],
    trigger_types_json: dict[str, Any] | None = None,
    baseline_trigger_types_json: dict[str, Any] | None = None,
    baseline_workers_json: dict[str, Any] | None = None,
) -> dict[str, list[dict[str, Any]]]:
    workers = _extract_array(workers_json, "workers")
    all_functions = _extract_array(functions_json, "functions")
    target_worker_names = _resolve_target_worker_names(
        workers=workers,
        worker_name=worker_name,
        baseline_workers_json=baseline_workers_json,
    )

    worker_function_ids = _function_ids_for_workers(all_functions, target_worker_names)

    functions_by_id = {
        f.get("function_id"): f for f in all_functions if f.get("function_id")
    }

    missing_function_ids = [fid for fid in worker_function_ids if fid not in functions_by_id]
    if missing_function_ids:
        raise ValueError(
            "missing function details for worker functions: "
            + ", ".join(str(fid) for fid in missing_function_ids)
        )

    functions = []
    for function_id in worker_function_ids:
        details = functions_by_id[function_id]
        metadata = details.get("metadata") or {}
        functions.append(
            {
                "name": derive_registry_function_name(function_id, metadata),
                "description": _string_or_empty(details.get("description")),
                "request_schema": _schema_or_empty(details.get("request_schema")),
                "response_schema": _schema_or_empty(details.get("response_schema")),
                "metadata": _metadata_or_empty(metadata),
            }
        )

    baseline_ids = {
        tt["id"]
        for tt in _extract_array(baseline_trigger_types_json or {}, "triggers")
        if isinstance(tt.get("id"), str)
    }

    triggers = []
    if trigger_types_json:
        for trigger_type in _extract_array(trigger_types_json, "triggers"):
            tt_id = trigger_type.get("id")
            if not isinstance(tt_id, str) or tt_id.startswith("engine::"):
                continue
            if tt_id in baseline_ids:
                continue
            triggers.append(_normalize_registry_trigger_type(trigger_type))

    return {"functions": functions, "triggers": triggers}


def build_payload(
    *,
    package_descriptor: dict[str, Any],
    descriptor_sha256: str,
    channel: str,
    repo_url: str,
    interface: dict[str, Any],
    artifacts: dict[str, Any],
    readme: str | None = None,
) -> dict[str, Any]:
    """Build only the strict descriptor-native Registry request.

    Publication callers must consume the immutable compiler output and
    prepared artifact inventory.  This helper deliberately has no catalog,
    source-tree, or legacy manifest fallback.
    """
    if not isinstance(package_descriptor, dict):
        raise ValueError("package_descriptor must be an object")
    if not isinstance(descriptor_sha256, str) or len(descriptor_sha256) != 64:
        raise ValueError("descriptor_sha256 must be a 64-character digest")
    if channel != "next":
        raise ValueError("candidate publication channel must be next")
    kind = (package_descriptor.get("artifact") or {}).get("kind")
    if artifacts.get("kind") != kind:
        raise ValueError("artifacts.kind must match package_descriptor.artifact.kind")
    payload: dict[str, Any] = {
        "package_descriptor": package_descriptor,
        "descriptor_sha256": descriptor_sha256,
        "channel": channel,
        "repo": repo_url,
        "interface": {
            "functions": [
                _normalize_registry_function(function)
                for function in interface.get("functions") or []
            ],
            "triggers": [
                _normalize_registry_trigger(trigger)
                for trigger in interface.get("triggers") or []
            ],
        },
        "artifacts": artifacts,
    }
    if readme is not None:
        payload["readme"] = readme
    return payload


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--descriptor", required=True)
    parser.add_argument("--descriptor-sha256", required=True)
    parser.add_argument("--channel", choices=("next",), default="next")
    parser.add_argument("--repo-url", required=True)
    parser.add_argument("--interface-json", required=True)
    parser.add_argument("--artifacts-json", required=True)
    parser.add_argument("--readme")
    parser.add_argument("--out", default="payload.json")
    args = parser.parse_args()

    package_descriptor = json.loads(pathlib.Path(args.descriptor).read_text(encoding="utf-8"))
    interface = json.loads(pathlib.Path(args.interface_json).read_text(encoding="utf-8"))
    artifacts = json.loads(pathlib.Path(args.artifacts_json).read_text(encoding="utf-8"))
    readme = pathlib.Path(args.readme).read_text(encoding="utf-8") if args.readme else None

    payload = build_payload(
        package_descriptor=package_descriptor,
        descriptor_sha256=args.descriptor_sha256,
        channel=args.channel,
        repo_url=args.repo_url,
        interface=interface,
        artifacts=artifacts,
        readme=readme,
    )
    pathlib.Path(args.out).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in payload.items() if k != "readme"}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
