#!/usr/bin/env python3
"""Capture the live function catalog for local search evaluation."""

import argparse
import json
import shutil
import subprocess
from datetime import datetime, timezone
from functools import partial
from pathlib import Path


MAX_INFO_BATCH = 32
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_FIXTURE = ROOT / "iii-directory/tests/fixtures/discover_catalog.json"
DEFAULT_NAMESPACE = "search-eval"
LIST_FIELDS = ("function_id", "id", "name", "worker_name", "description")
INFO_FIELDS = LIST_FIELDS + ("parameters", "request_format", "request_schema", "response_schema", "error")


def run_iii(function, payload, *, port=None):
    command = ["iii", "trigger"]
    if port is not None:
        command.extend(["--port", str(port)])
    command.extend([function, "--json", json.dumps(payload)])
    completed = subprocess.run(
        command,
        check=True,
        text=True,
        capture_output=True,
        timeout=30,
    )
    return json.loads(completed.stdout)


def function_ids(response, namespace=None):
    if isinstance(response, list):
        rows = response
    elif isinstance(response, dict):
        rows = response.get("functions", response.get("items"))
    else:
        rows = None
    if not isinstance(rows, list):
        raise ValueError("malformed response")
    ids = []
    for entry in rows:
        if (
            namespace is not None
            and isinstance(entry, dict)
            and isinstance(entry.get("namespace"), str)
            and entry["namespace"] != namespace
        ):
            continue
        function_id = entry if isinstance(entry, str) else next(
            (entry.get(key) for key in ("function_id", "id", "name") if isinstance(entry, dict) and isinstance(entry.get(key), str)),
            None,
        )
        if not isinstance(function_id, str):
            raise ValueError("malformed response")
        ids.append(function_id)
    return ids


def collect(trigger=run_iii, batch_size=MAX_INFO_BATCH, namespace=DEFAULT_NAMESPACE):
    if not 1 <= batch_size <= MAX_INFO_BATCH:
        raise ValueError(f"batch size must be between 1 and {MAX_INFO_BATCH}")

    errors = []
    raw = {"normal": None, "internal": None, "info": []}
    ids = set()
    for name, payload in (("normal", {}), ("internal", {"include_internal": True})):
        try:
            raw[name] = trigger("engine::functions::list", payload)
            ids.update(function_ids(raw[name], namespace))
        except Exception as exc:  # collection remains useful when one list is unavailable
            errors.append({"stage": f"{name} list", "error": str(exc)})

    ids = sorted(
        function_id
        for function_id in ids
        if not function_id.startswith("engine::")
        and function_id != "directory::search_functions"
    )
    entries = {}
    for start in range(0, len(ids), batch_size):
        batch = ids[start : start + batch_size]
        try:
            response = trigger("engine::functions::info", {"function_ids": batch, "namespace": namespace})
            raw["info"].append({"function_ids": batch, "response": response})
            if not isinstance(response, dict) or not isinstance(response.get("functions"), list):
                raise ValueError("malformed response")
            for entry in response["functions"]:
                if isinstance(entry, dict) and isinstance(entry.get("function_id"), str):
                    entries[entry["function_id"]] = entry
        except Exception as exc:  # one failed batch must not lose the rest of the catalog
            errors.append({"batch": batch, "error": str(exc)})

    catalog = []
    for function_id in ids:
        entry = entries.get(function_id)
        if entry is None:
            errors.append({"function_id": function_id, "error": "missing response"})
            continue
        if entry.get("error") is not None:
            errors.append({"function_id": function_id, "error": entry["error"]})
            continue
        if isinstance(entry.get("metadata"), dict) and entry["metadata"].get("internal") is True:
            continue
        parameters = entry.get("parameters") or entry.get("request_format") or entry.get("request_schema")
        catalog.append(
            {
                "name": function_id,
                "description": entry.get("description") if isinstance(entry.get("description"), str) else "",
                "parameters": parameters if parameters is not None else {"type": "object"},
            }
        )
    return catalog, errors, raw


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def allowed_entry(entry, fields):
    if isinstance(entry, str):
        return {"function_id": entry}
    if not isinstance(entry, dict):
        return None
    kept = {field: entry[field] for field in fields if field in entry}
    metadata = entry.get("metadata")
    if isinstance(metadata, dict) and isinstance(metadata.get("internal"), bool):
        kept["metadata"] = {"internal": metadata["internal"]}
    return kept


def sanitize_response(response, fields):
    def entries(rows):
        return [kept for row in rows if (kept := allowed_entry(row, fields)) is not None]

    if isinstance(response, list):
        return entries(response)
    if not isinstance(response, dict):
        return None
    return {key: entries(response[key]) for key in ("functions", "items") if isinstance(response.get(key), list)}


def sanitize_info_batches(batches):
    sanitized = []
    for batch in batches:
        if not isinstance(batch, dict):
            continue
        function_ids = batch.get("function_ids")
        if not isinstance(function_ids, list) or not all(isinstance(function_id, str) for function_id in function_ids):
            continue
        sanitized.append(
            {"function_ids": function_ids, "response": sanitize_response(batch.get("response"), INFO_FIELDS)}
        )
    return sanitized


def write_capture(output_root, catalog, errors, raw, *, accept, fixture):
    artifact = output_root / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    artifact.mkdir(parents=True, exist_ok=True)
    write_json(artifact / "normal-functions.json", sanitize_response(raw["normal"], LIST_FIELDS))
    write_json(artifact / "internal-functions.json", sanitize_response(raw["internal"], LIST_FIELDS))
    write_json(artifact / "info-batches.json", sanitize_info_batches(raw["info"]))
    write_json(artifact / "errors.json", errors)
    write_json(artifact / "catalog.json", catalog)
    if accept:
        fixture.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(artifact / "catalog.json", fixture)
    return artifact


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept", action="store_true", help="replace the committed normalized fixture")
    parser.add_argument("--batch-size", type=int, default=MAX_INFO_BATCH)
    parser.add_argument("--namespace", default=DEFAULT_NAMESPACE, help="compose namespace to query")
    parser.add_argument("--port", type=int, required=True, help="engine WebSocket port")
    parser.add_argument("--output-root", type=Path, default=ROOT / "target/search-eval")
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    args = parser.parse_args(argv)

    catalog, errors, raw = collect(
        trigger=partial(run_iii, port=args.port),
        batch_size=args.batch_size,
        namespace=args.namespace,
    )
    valid = bool(catalog) and not errors
    artifact = write_capture(
        args.output_root,
        catalog,
        errors,
        raw,
        accept=args.accept and valid,
        fixture=args.fixture,
    )
    print(
        json.dumps(
            {
                "artifact": str(artifact),
                "functions": len(catalog),
                "errors": len(errors),
                "valid": valid,
            }
        )
    )
    if not valid:
        raise SystemExit("catalog capture is empty or incomplete; diagnostic artifacts were retained")


if __name__ == "__main__":
    main()
