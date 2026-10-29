#!/usr/bin/env python3
import argparse
import json
import os
import pathlib
import subprocess
import sys
import time

from build_publish_payload import (
    _function_ids_for_workers,
    _resolve_target_worker_names,
    normalize_worker_interface,
)


# A real JSON Schema carries at least one of these schema-defining keywords. The
# permissive schema `schemars` emits for an untyped `serde_json::Value` handler
# is `{"$schema": ..., "title": "AnyValue"}` (no keyword below) and a missing
# schema normalizes to `{}` — both surface as "unknown" request/response schemas
# in the registry. This mirrors the per-worker Rust invariant in
# `tests/schemas.rs` (`assert_typed_schema`).
SCHEMA_DEFINING_KEYS = frozenset(
    {"type", "properties", "$ref", "allOf", "anyOf", "oneOf", "enum", "items", "const"}
)

# Gap between registration snapshots. Three equal samples must fit inside the
# life of a worker booted without its dependencies: harness registers, then
# exits ~5 s later when no queue worker answers.
SAMPLE_INTERVAL_SECONDS = 0.5


def _schema_is_typed(schema: object) -> bool:
    return isinstance(schema, dict) and any(k in schema for k in SCHEMA_DEFINING_KEYS)


def _typed_schema_violations(functions: list[dict[str, object]]) -> list[str]:
    """Return `"<name>.<field>"` for every function whose request/response schema
    is the permissive AnyValue/empty ("unknown") schema."""
    violations: list[str] = []
    for fn in functions:
        if not isinstance(fn, dict):
            continue
        name = fn.get("name") or fn.get("function_id") or "<unknown>"
        for field in ("request_schema", "response_schema"):
            if not _schema_is_typed(fn.get(field)):
                violations.append(f"{name}.{field}")
    return violations


def run_iii(function_path: str, payload: dict[str, object]) -> dict[str, object]:
    command = ["iii", "call", function_path, "--json", json.dumps(payload)]
    if port := os.environ.get("III_ENGINE_PORT"):
        if not port.isdigit() or not (1 <= int(port) <= 65535):
            raise ValueError("III_ENGINE_PORT must be an integer from 1 to 65535")
        command.extend(["--port", port])
    completed = subprocess.run(
        command,
        check=True,
        text=True,
        capture_output=True,
        timeout=30,
    )
    return json.loads(completed.stdout)


def _fetch_function_detail(function_id: str) -> dict[str, object] | None:
    """`engine::functions::info` for one function, or None on any failure.

    `engine::functions::list` returns a `FunctionSummary` (id, worker_name,
    description and the observe fields); `::info` returns the `FunctionDetail`,
    which also carries the typed `request_schema`/`response_schema`.
    Best-effort: a failed lookup leaves the row schema-less, which the
    `--assert-typed-schemas` check then flags."""
    try:
        return run_iii("engine::functions::info", {"function_id": function_id})
    except (
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as exc:
        print(
            f"::warning::could not fetch engine::functions::info for {function_id}: {exc}",
            file=sys.stderr,
        )
        return None


def enrich_functions_with_schemas(
    functions: list[dict[str, object]],
    target_function_ids: list[str],
    *,
    fetch_detail=_fetch_function_detail,
) -> list[dict[str, object]]:
    """Merge each target function's typed schemas into its `::list` row in place.

    `engine::functions::list` omits request/response schemas; only
    `engine::functions::info` exposes them (as `request_schema`/`response_schema`,
    plus `metadata`). Without this enrichment every collected schema normalizes
    to the empty `{}` that blocks publish. The detail's observe fields
    (`args_format`, `private`, `observable`) replace the list row's, so the
    row describes the function as `::info` does."""
    rows_by_id = {
        f.get("function_id"): f for f in functions if isinstance(f, dict)
    }
    for function_id in target_function_ids:
        row = rows_by_id.get(function_id)
        if row is None:
            continue
        detail = fetch_detail(function_id)
        if not isinstance(detail, dict):
            continue
        for key in (
            "request_schema",
            "response_schema",
            "metadata",
            "description",
            "args_format",
            "private",
            "observable",
        ):
            value = detail.get(key)
            if value is not None:
                row[key] = value
    return functions


def wait_for_worker(
    worker_name: str,
    wait_seconds: int,
    *,
    workers_baseline_json: dict[str, object] | None = None,
) -> tuple[dict[str, object], dict[str, object]]:
    deadline = time.monotonic() + wait_seconds
    workers_json = run_iii("engine::workers::list", {})
    functions_json = run_iii("engine::functions::list", {"include_internal": True})
    if wait_seconds <= 0:
        return workers_json, functions_json

    def target_function_ids() -> frozenset[str]:
        workers = workers_json.get("workers", [])
        functions = functions_json.get("functions", [])
        if not isinstance(workers, list) or not isinstance(functions, list):
            return frozenset()
        try:
            target_names = _resolve_target_worker_names(
                workers=workers,
                worker_name=worker_name,
                baseline_workers_json=workers_baseline_json,
            )
        except ValueError:
            return frozenset()
        return frozenset(_function_ids_for_workers(functions, target_names))

    previous_ids: frozenset[str] = frozenset()
    stable_samples = 0
    # SDKs can register their functions asynchronously, and a v2 SDK holds the
    # functions that need observe.v2 until the engine has advertised it. A
    # connected worker with one registration is not enough: require three
    # unchanged, non-empty snapshots so release capture does not publish the
    # first partial batch.
    while time.monotonic() < deadline:
        current_ids = target_function_ids()
        if current_ids and current_ids == previous_ids:
            stable_samples += 1
        else:
            stable_samples = 1 if current_ids else 0
        if stable_samples >= 3:
            return workers_json, functions_json
        previous_ids = current_ids
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        time.sleep(min(SAMPLE_INTERVAL_SECONDS, remaining))
        if time.monotonic() >= deadline:
            break
        workers_json = run_iii("engine::workers::list", {})
        functions_json = run_iii("engine::functions::list", {"include_internal": True})

    raise TimeoutError(
        f"{worker_name}: interface registrations did not stabilize within {wait_seconds} seconds"
    )


def _assert_interface(data: dict[str, object], source: str, args: argparse.Namespace) -> int:
    functions = data.get("functions") or []
    if args.assert_non_empty and not functions:
        print(f"::error::no worker functions in {source} (empty)", file=sys.stderr)
        return 1
    if args.assert_typed_schemas:
        violations = _typed_schema_violations(functions)
        if violations:
            print(
                f"::error::untyped (AnyValue/empty) schemas in {source}: "
                f"{', '.join(violations)}. Register the handler with a typed "
                "struct deriving schemars::JsonSchema (see "
                "docs/sops/binary-worker.md).",
                file=sys.stderr,
            )
            return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=False)
    parser.add_argument("--out", default="worker-interface.json")
    parser.add_argument("--wait-seconds", type=int, default=0)
    parser.add_argument("--workers-baseline", default="")
    parser.add_argument(
        "--assert-non-empty",
        action="store_true",
        help="after writing --out, assert functions is non-empty",
    )
    parser.add_argument(
        "--assert-typed-schemas",
        action="store_true",
        help="assert every function has a typed (non-AnyValue/non-empty) "
        "request_schema and response_schema — blocks 'unknown' schemas at publish",
    )
    parser.add_argument(
        "--assert-file",
        help="path to an already-written interface JSON to assert; bypasses collection",
    )
    args = parser.parse_args()

    # Standalone assertion mode — bypass collection entirely.
    if args.assert_file:
        if not (args.assert_non_empty or args.assert_typed_schemas):
            print(
                "--assert-file requires --assert-non-empty and/or --assert-typed-schemas",
                file=sys.stderr,
            )
            return 1
        try:
            data = json.loads(pathlib.Path(args.assert_file).read_text())
        except Exception as e:
            print(f"could not read {args.assert_file}: {e}", file=sys.stderr)
            return 1
        return _assert_interface(data, args.assert_file, args)

    if not args.worker:
        print("--worker is required unless --assert-file is given", file=sys.stderr)
        return 1

    workers_baseline_json = None
    if args.workers_baseline:
        workers_baseline_path = pathlib.Path(args.workers_baseline)
        if workers_baseline_path.exists():
            workers_baseline_json = json.loads(workers_baseline_path.read_text(encoding="utf-8"))

    try:
        workers_json, functions_json = wait_for_worker(
            args.worker,
            args.wait_seconds,
            workers_baseline_json=workers_baseline_json,
        )
    except TimeoutError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1

    # `engine::functions::list` carries no schemas — pull each target function's
    # typed request/response schema from `engine::functions::info` and merge it
    # into the rows before normalizing. Best-effort worker resolution here;
    # normalize_worker_interface remains the authority (and raises) if the
    # worker can't be matched.
    all_functions = functions_json.get("functions") or []
    if isinstance(all_functions, list):
        try:
            target_worker_names = _resolve_target_worker_names(
                workers=workers_json.get("workers") or [],
                worker_name=args.worker,
                baseline_workers_json=workers_baseline_json,
            )
            target_function_ids = _function_ids_for_workers(
                all_functions, target_worker_names
            )
        except ValueError:
            target_function_ids = []
        if target_function_ids:
            enrich_functions_with_schemas(all_functions, target_function_ids)

    interface = normalize_worker_interface(
        worker_name=args.worker,
        workers_json=workers_json,
        functions_json=functions_json,
        baseline_workers_json=workers_baseline_json,
    )
    pathlib.Path(args.out).write_text(json.dumps(interface, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(interface, indent=2))

    if args.assert_non_empty or args.assert_typed_schemas:
        with open(args.out) as f:
            data = json.load(f)
        return _assert_interface(data, args.out, args)

    return 0


if __name__ == "__main__":
    sys.exit(main())
