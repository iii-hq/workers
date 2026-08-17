#!/usr/bin/env python3
import argparse
import json
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


def _untyped_trigger_names(triggers: list[dict[str, object]]) -> list[str]:
    """Trigger names whose `invocation_schema` is the empty/AnyValue ('unknown')
    schema. Unlike functions, a schema-less trigger is NOT a hard publish blocker:
    some trigger types legitimately take no binding config (e.g. iii-directory's
    `directory::*::on-change`), so callers warn rather than fail."""
    names: list[str] = []
    for trigger in triggers:
        if not isinstance(trigger, dict):
            continue
        if not _schema_is_typed(trigger.get("invocation_schema")):
            names.append(str(trigger.get("name") or "<unknown>"))
    return names


def _warn_untyped_triggers(triggers: list[dict[str, object]], source: str) -> None:
    untyped = _untyped_trigger_names(triggers)
    if untyped:
        print(
            f"::warning::triggers in {source} publishing without a typed "
            f"invocation_schema (renders 'unknown' in the registry): "
            f"{', '.join(untyped)}. If the trigger takes a binding config, "
            "register it with .trigger_request_format::<T>() "
            "(see session-manager/src/events.rs).",
            file=sys.stderr,
        )


def run_iii(function_path: str, payload: dict[str, object]) -> dict[str, object]:
    completed = subprocess.run(
        [
            "iii",
            "trigger",
            function_path,
            "--json",
            json.dumps(payload),
        ],
        check=True,
        text=True,
        capture_output=True,
        timeout=30,
    )
    return json.loads(completed.stdout)


def _fetch_function_detail(function_id: str) -> dict[str, object] | None:
    """`engine::functions::info` for one function, or None on any failure.

    Unlike `engine::functions::list` (a `FunctionSummary`: id + worker_name +
    description), `::info` returns the `FunctionDetail` that carries the typed
    `request_schema`/`response_schema`. Best-effort: a failed lookup leaves the
    row schema-less, which the `--assert-typed-schemas` check then flags."""
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
    to the empty `{}` that blocks publish."""
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
        for key in ("request_schema", "response_schema", "metadata", "description"):
            value = detail.get(key)
            if value is not None:
                row[key] = value
    return functions


def _fetch_trigger_detail(trigger_id: str) -> dict[str, object] | None:
    """`engine::triggers::info` for one trigger type, or None on any failure.

    `engine::triggers::list` returns a `TriggerTypeSummary` (id + worker_name +
    description only); only `::info` returns the `TriggerTypeDetail` carrying the
    typed `configuration_schema`/`request_schema`/`response_schema`. Best-effort:
    a failed lookup leaves the row schema-less, which publishes the empty
    ("unknown") schema."""
    try:
        return run_iii("engine::triggers::info", {"id": trigger_id})
    except (
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as exc:
        print(
            f"::warning::could not fetch engine::triggers::info for {trigger_id}: {exc}",
            file=sys.stderr,
        )
        return None


def enrich_trigger_types_with_schemas(
    trigger_types: list[dict[str, object]],
    target_trigger_ids: list[str],
    *,
    fetch_detail=_fetch_trigger_detail,
) -> list[dict[str, object]]:
    """Merge each target trigger type's typed schemas into its `::list` row in place.

    `engine::triggers::list` omits the schemas; only `engine::triggers::info`
    exposes them (`configuration_schema`/`request_schema`/`response_schema`).
    Without this enrichment every collected trigger schema normalizes to the
    empty `{}` that surfaces as 'unknown' in the registry. Mirrors
    `enrich_functions_with_schemas` for the trigger-type surface."""
    rows_by_id = {t.get("id"): t for t in trigger_types if isinstance(t, dict)}
    for trigger_id in target_trigger_ids:
        row = rows_by_id.get(trigger_id)
        if row is None:
            continue
        detail = fetch_detail(trigger_id)
        if not isinstance(detail, dict):
            continue
        for key in (
            "configuration_schema",
            "request_schema",
            "response_schema",
            "description",
        ):
            value = detail.get(key)
            if value is not None:
                row[key] = value
    return trigger_types


def wait_for_worker(
    worker_name: str,
    wait_seconds: int,
    *,
    workers_baseline_json: dict[str, object] | None = None,
) -> tuple[dict[str, object], dict[str, object]]:
    deadline = time.monotonic() + wait_seconds
    workers_json = run_iii("engine::workers::list", {})
    functions_json = run_iii("engine::functions::list", {"include_internal": True})

    def ready() -> bool:
        workers = workers_json.get("workers", [])
        functions = functions_json.get("functions", [])
        if not isinstance(workers, list) or not isinstance(functions, list):
            return False
        try:
            target_names = _resolve_target_worker_names(
                workers=workers,
                worker_name=worker_name,
                baseline_workers_json=workers_baseline_json,
            )
        except ValueError:
            return False
        return len(_function_ids_for_workers(functions, target_names)) > 0

    while not ready() and time.monotonic() < deadline:
        time.sleep(2)
        workers_json = run_iii("engine::workers::list", {})
        functions_json = run_iii("engine::functions::list", {"include_internal": True})

    return workers_json, functions_json


def collect_trigger_types() -> dict[str, object] | None:
    try:
        return run_iii("engine::triggers::list", {"include_internal": False})
    except (
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as exc:
        print(
            f"::warning::could not collect trigger types; publishing triggers=[]: {exc}",
            file=sys.stderr,
        )
        return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=False)
    parser.add_argument("--out", default="worker-interface.json")
    parser.add_argument("--wait-seconds", type=int, default=0)
    parser.add_argument("--trigger-types-baseline", default="")
    parser.add_argument("--workers-baseline", default="")
    parser.add_argument(
        "--assert-non-empty",
        action="store_true",
        help="after writing --out, assert payload['functions'] is non-empty",
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
        functions = data.get("functions") or []
        if args.assert_non_empty and not functions:
            print(
                f"::error::no worker functions in {args.assert_file} (empty)",
                file=sys.stderr,
            )
            return 1
        if args.assert_typed_schemas:
            violations = _typed_schema_violations(functions)
            if violations:
                print(
                    "::error::untyped (AnyValue/empty) schemas in "
                    f"{args.assert_file}: {', '.join(violations)}. Register the "
                    "handler with a typed struct deriving schemars::JsonSchema "
                    "(see docs/sops/binary-worker.md).",
                    file=sys.stderr,
                )
                return 1
            _warn_untyped_triggers(data.get("triggers") or [], args.assert_file)
        return 0

    if not args.worker:
        print("--worker is required unless --assert-file is given", file=sys.stderr)
        return 1

    baseline_json = None
    if args.trigger_types_baseline:
        baseline_path = pathlib.Path(args.trigger_types_baseline)
        if baseline_path.exists():
            baseline_json = json.loads(baseline_path.read_text(encoding="utf-8"))

    workers_baseline_json = None
    if args.workers_baseline:
        workers_baseline_path = pathlib.Path(args.workers_baseline)
        if workers_baseline_path.exists():
            workers_baseline_json = json.loads(workers_baseline_path.read_text(encoding="utf-8"))

    workers_json, functions_json = wait_for_worker(
        args.worker,
        args.wait_seconds,
        workers_baseline_json=workers_baseline_json,
    )

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

    trigger_types_json = collect_trigger_types()

    # `engine::triggers::list` carries no schemas — pull each publishable trigger
    # type's typed schemas from `engine::triggers::info` and merge them into the
    # rows before normalizing. Skip engine built-ins and trigger types already in
    # the pre-worker baseline (they're dropped from the payload anyway), so we
    # only spend `::info` calls on the worker's own trigger types.
    if isinstance(trigger_types_json, dict):
        collected_triggers = trigger_types_json.get("triggers")
        if isinstance(collected_triggers, list):
            baseline_trigger_ids = {
                tt.get("id")
                for tt in (
                    baseline_json.get("triggers", [])
                    if isinstance(baseline_json, dict)
                    else []
                )
                if isinstance(tt, dict) and isinstance(tt.get("id"), str)
            }
            target_trigger_ids = [
                tid
                for t in collected_triggers
                if isinstance(t, dict)
                and isinstance((tid := t.get("id")), str)
                and not tid.startswith("engine::")
                and tid not in baseline_trigger_ids
            ]
            if target_trigger_ids:
                enrich_trigger_types_with_schemas(collected_triggers, target_trigger_ids)

    interface = normalize_worker_interface(
        worker_name=args.worker,
        workers_json=workers_json,
        functions_json=functions_json,
        trigger_types_json=trigger_types_json,
        baseline_trigger_types_json=baseline_json,
        baseline_workers_json=workers_baseline_json,
    )
    pathlib.Path(args.out).write_text(json.dumps(interface, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(interface, indent=2))

    if (args.assert_non_empty or args.assert_typed_schemas) and not args.assert_file:
        with open(args.out) as f:
            data = json.load(f)
        functions = data.get("functions") or []
        if args.assert_non_empty and not functions:
            print(f"::error::no worker functions in {args.out} (empty)", file=sys.stderr)
            return 1
        if args.assert_typed_schemas:
            violations = _typed_schema_violations(functions)
            if violations:
                print(
                    f"::error::untyped (AnyValue/empty) schemas in {args.out}: "
                    f"{', '.join(violations)}. Register the handler with a typed "
                    "struct deriving schemars::JsonSchema (see "
                    "docs/sops/binary-worker.md).",
                    file=sys.stderr,
                )
                return 1
            _warn_untyped_triggers(data.get("triggers") or [], args.out)

    return 0


if __name__ == "__main__":
    sys.exit(main())
