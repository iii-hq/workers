#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402
import worker_screenshots  # noqa: E402

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


def _args_format_or_none(value: Any) -> dict[str, Any] | None:
    if value is None:
        return None
    if isinstance(value, dict):
        return value
    raise ValueError("function args_format must be an object or null")


def _observe_fields(function: dict[str, Any]) -> dict[str, Any]:
    """What a function row says about observing the function.

    `args_format` is the JSON Schema of observers' `args` (`None` when the
    function takes no args), `private` is true when only the hosting worker
    calls it, and `observable` is false when nobody may observe it. The engine
    leaves the defaults out of its rows (`private: false`, `observable: true`),
    and the Registry row states all three."""
    return {
        "args_format": _args_format_or_none(function.get("args_format")),
        "private": function.get("private") is True,
        "observable": function.get("observable") is not False,
    }


def _normalize_registry_function(function: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": function.get("name"),
        "description": _string_or_empty(function.get("description")),
        "request_schema": _schema_or_empty(function.get("request_schema")),
        "response_schema": _schema_or_empty(function.get("response_schema")),
        **_observe_fields(function),
        "metadata": _metadata_or_empty(function.get("metadata")),
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
# installed from the registry). A target install can flip one on mid-boot
# (e.g. a worker that still lists the deprecated `iii-stream` builtin as a
# dependency), which lands it in the workers-baseline diff even though its interface is not part of the
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


def normalize_worker_interface(
    *,
    worker_name: str,
    workers_json: dict[str, Any],
    functions_json: dict[str, Any],
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
                **_observe_fields(details),
                "metadata": _metadata_or_empty(metadata),
            }
        )

    return {"functions": functions}


def publish_screenshots(screenshots: Any, *, deploy: str, worker_name: str) -> list[dict[str, Any]]:
    """`POST /publish`'s `screenshots` from the build manifest's (or the descriptor's).

    Each image by the asset name it is registered under on the version's
    release, in display order, with its platform and caption. A worker the
    registry gives no release (an image) or no page (a hidden one) can only
    clear them with `[]`.
    """
    if not isinstance(screenshots, list):
        raise ValueError("screenshots must be an array")
    if screenshots and deploy == "image":
        raise ValueError("an image version has no release to host screenshots on; only [] clears them")
    if screenshots and worker_name in worker_screenshots.HIDDEN_WORKERS:
        raise ValueError(f"the registry hides {worker_name}: its screenshots could never be read back; only []")
    listed: list[dict[str, Any]] = []
    for entry in screenshots:
        name = entry.get("name") if isinstance(entry, dict) else None
        platform = entry.get("platform") if isinstance(entry, dict) else None
        if not isinstance(name, str) or not isinstance(platform, str):
            raise ValueError("every screenshot needs a name and a platform")
        shot: dict[str, Any] = {"asset": name, "platform": platform}
        if "caption" in entry:
            shot["caption"] = entry["caption"]
        listed.append(shot)
    return listed


def build_payload(
    *,
    registry_projection: dict[str, Any],
    published_version: str,
    repo_url: str,
    interface: dict[str, Any],
    interface_capture: str,
    artifacts: dict[str, Any],
    readme: str | None = None,
    screenshots: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Merge the immutable compiler projection with current Registry fields.

    `screenshots` is the build manifest's: given (`[]` included), the payload
    lists them; None leaves the field out, and the version inherits the
    previous version's.
    """
    required = {
        "worker_name", "type", "description", "license", "tags",
        "dependencies", "config", "experimental", "readme",
    }
    if not isinstance(registry_projection, dict) or set(registry_projection) != required:
        raise ValueError("registry_projection differs from the current Registry metadata contract")
    config = registry_projection["config"]
    if config is not None and not isinstance(config, dict):
        raise ValueError("registry_projection.config must be an object or null")
    if interface_capture not in {"required", "skipped"}:
        raise ValueError("interface_capture must be required or skipped")
    if not isinstance(interface, dict) or set(interface) != {"functions"}:
        raise ValueError("interface must contain only functions")
    if not isinstance(interface["functions"], list) or any(
        not isinstance(row, dict) for row in interface["functions"]
    ):
        raise ValueError("interface functions must be an array of objects")
    interface_empty = not interface["functions"]
    if interface_capture == "required" and interface_empty:
        raise ValueError("required interface capture must contain a function")
    if interface_capture == "skipped" and not interface_empty:
        raise ValueError("skipped interface capture must publish an empty interface")
    _lib.validate_deployment_target_version(published_version)
    deploy = registry_projection["type"]
    kind = artifacts.get("kind")
    expected_kind = {
        "binary": {"rust-binary", "go-binary"},
        "bundle": {"javascript-bundle", "python-bundle"},
        "image": "oci-image",
    }.get(deploy)
    if (kind not in expected_kind) if isinstance(expected_kind, set) else (kind != expected_kind):
        raise ValueError("prepared artifacts differ from the public deploy type")
    payload: dict[str, Any] = {
        **registry_projection,
        # The Registry nullable-config rollout requires an explicit key while
        # treating an empty public default as absent.
        "config": config or None,
        "version": published_version,
        "repo": repo_url,
        "functions": [
            _normalize_registry_function(function)
            for function in interface.get("functions") or []
        ],
    }
    if deploy == "binary":
        binaries = artifacts.get("binaries")
        if not isinstance(binaries, dict) or not binaries:
            raise ValueError("binary publication requires prepared binaries")
        payload["binaries"] = binaries
    elif deploy == "bundle":
        archive_url = artifacts.get("archive_url")
        digest = artifacts.get("sha256")
        if not isinstance(archive_url, str) or not archive_url or not isinstance(digest, str) or not digest:
            raise ValueError("bundle publication requires archive_url and sha256")
        payload.update(archive_url=archive_url, sha256=digest)
    elif deploy == "image":
        image_tag = artifacts.get("image_tag")
        if not isinstance(image_tag, str) or "@sha256:" not in image_tag:
            raise ValueError("image publication requires a digest-pinned image_tag")
        payload["image_tag"] = image_tag
    if readme is not None and readme != payload["readme"]:
        raise ValueError("external readme differs from the compiled Registry projection")
    if screenshots is not None:
        payload["screenshots"] = publish_screenshots(
            screenshots, deploy=deploy, worker_name=str(registry_projection["worker_name"])
        )
    return payload


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--descriptor", required=True)
    parser.add_argument("--published-version", required=True)
    parser.add_argument("--repo-url", required=True)
    parser.add_argument("--interface-json", required=True)
    parser.add_argument("--artifacts-json", required=True)
    parser.add_argument("--readme")
    parser.add_argument("--out", default="payload.json")
    args = parser.parse_args()

    descriptor = json.loads(pathlib.Path(args.descriptor).read_text(encoding="utf-8"))
    interface = json.loads(pathlib.Path(args.interface_json).read_text(encoding="utf-8"))
    artifacts = json.loads(pathlib.Path(args.artifacts_json).read_text(encoding="utf-8"))
    readme = pathlib.Path(args.readme).read_text(encoding="utf-8") if args.readme else None

    payload = build_payload(
        registry_projection=descriptor["registry_projection"],
        published_version=args.published_version,
        repo_url=args.repo_url,
        interface=interface,
        interface_capture=descriptor["interface_capture"],
        artifacts=artifacts,
        readme=readme,
        # Present exactly when the worker has a list; the manifest's is the same.
        screenshots=descriptor.get("screenshots"),
    )
    pathlib.Path(args.out).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in payload.items() if k != "readme"}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
