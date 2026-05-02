#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys
from typing import Any

import yaml


def normalize_dependencies(raw_deps: Any) -> list[dict[str, Any]]:
    if raw_deps in (None, ""):
        return []
    if isinstance(raw_deps, dict):
        return [{"name": name, "version": version} for name, version in raw_deps.items()]
    if isinstance(raw_deps, list):
        return raw_deps
    raise ValueError(f"`dependencies` must be a map or list, got {type(raw_deps).__name__}")


def derive_registry_function_name(function_id: str, metadata: dict[str, Any] | None) -> str:
    metadata = metadata or {}
    for key in ("registry_name", "name"):
        value = metadata.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    if "::" in function_id:
        return function_id.rsplit("::", 1)[1]
    return function_id


def _extract_array(payload: dict[str, Any], key: str) -> list[dict[str, Any]]:
    value = payload.get(key, [])
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"`{key}` must be an array")
    return value


def normalize_worker_interface(
    *,
    worker_name: str,
    workers_json: dict[str, Any],
    functions_json: dict[str, Any],
    triggers_json: dict[str, Any] | None = None,
) -> dict[str, list[dict[str, Any]]]:
    workers = _extract_array(workers_json, "workers")
    matches = [w for w in workers if w.get("name") == worker_name or w.get("id") == worker_name]
    if len(matches) != 1:
        raise ValueError(f"expected exactly one worker matching {worker_name!r}, found {len(matches)}")

    worker_function_ids = matches[0].get("functions") or []
    if not isinstance(worker_function_ids, list):
        raise ValueError("worker `functions` must be an array")

    functions_by_id = {
        f.get("function_id"): f
        for f in _extract_array(functions_json, "functions")
        if f.get("function_id")
    }

    functions = []
    for function_id in worker_function_ids:
        details = functions_by_id.get(function_id, {})
        metadata = details.get("metadata") or {}
        functions.append(
            {
                "name": derive_registry_function_name(function_id, metadata),
                "description": details.get("description"),
                "request_schema": details.get("request_format"),
                "response_schema": details.get("response_format"),
                "metadata": metadata if isinstance(metadata, dict) else {},
            }
        )

    worker_ids = set(worker_function_ids)
    triggers = []
    if triggers_json:
        for trigger in _extract_array(triggers_json, "triggers"):
            if trigger.get("function_id") not in worker_ids:
                continue
            metadata = trigger.get("metadata") or {}
            triggers.append(
                {
                    "id": trigger.get("id"),
                    "trigger_type": trigger.get("trigger_type"),
                    "function_id": trigger.get("function_id"),
                    "config": trigger.get("config") or {},
                    "metadata": metadata if isinstance(metadata, dict) else {},
                }
            )

    return {"functions": functions, "triggers": triggers}


def build_payload(
    *,
    repo_root: pathlib.Path,
    worker: str,
    version: str,
    registry_tag: str,
    deploy: str,
    repo_url: str,
    interface: dict[str, Any],
    binaries: dict[str, Any],
    image_tag: str,
) -> dict[str, Any]:
    root = repo_root / worker
    meta = yaml.safe_load((root / "iii.worker.yaml").read_text(encoding="utf-8")) or {}

    readme_path = root / "README.md"
    readme = readme_path.read_text(encoding="utf-8") if readme_path.exists() else ""

    config_path = root / "config.yaml"
    config = yaml.safe_load(config_path.read_text(encoding="utf-8")) if config_path.exists() else {}
    if config is None:
        config = {}

    payload: dict[str, Any] = {
        "worker_name": worker,
        "version": version,
        "tag": registry_tag or "latest",
        "type": deploy,
        "readme": readme,
        "repo": repo_url,
        "description": meta.get("description", ""),
        "dependencies": normalize_dependencies(meta.get("dependencies")),
        "config": config,
        "functions": interface.get("functions") or [],
        "triggers": interface.get("triggers") or [],
    }

    if deploy == "binary":
        if not binaries:
            raise ValueError("deploy=binary requires non-empty binaries")
        payload["binaries"] = binaries
    elif deploy == "image":
        if not image_tag:
            raise ValueError("deploy=image requires image_tag")
        payload["image_tag"] = image_tag
    else:
        raise ValueError(f"unsupported deploy={deploy}")

    return payload


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--registry-tag", default="latest")
    parser.add_argument("--deploy", required=True, choices=["binary", "image"])
    parser.add_argument("--repo-url", required=True)
    parser.add_argument("--interface-json", required=True)
    parser.add_argument("--binaries-json", default="")
    parser.add_argument("--image-tag", default="")
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--out", default="payload.json")
    args = parser.parse_args()

    interface = json.loads(pathlib.Path(args.interface_json).read_text(encoding="utf-8"))
    binaries = {}
    if args.binaries_json:
        binaries = json.loads(pathlib.Path(args.binaries_json).read_text(encoding="utf-8"))

    payload = build_payload(
        repo_root=pathlib.Path(args.repo_root),
        worker=args.worker,
        version=args.version,
        registry_tag=args.registry_tag,
        deploy=args.deploy,
        repo_url=args.repo_url,
        interface=interface,
        binaries=binaries,
        image_tag=args.image_tag,
    )
    pathlib.Path(args.out).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in payload.items() if k != "readme"}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
