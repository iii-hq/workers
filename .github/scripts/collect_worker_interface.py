#!/usr/bin/env python3
import argparse
import json
import pathlib
import subprocess
import sys
import time

from build_publish_payload import normalize_worker_interface


def count_worker_matches(workers_json: dict[str, object], worker_name: str) -> int:
    workers = workers_json.get("workers", [])
    if not isinstance(workers, list):
        return 0
    return sum(
        1
        for worker in workers
        if isinstance(worker, dict)
        and (worker.get("name") == worker_name or worker.get("id") == worker_name)
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


def wait_for_worker(worker_name: str, wait_seconds: int) -> dict[str, object]:
    deadline = time.monotonic() + wait_seconds
    workers_json = run_iii("engine::workers::list", {})

    while count_worker_matches(workers_json, worker_name) != 1 and time.monotonic() < deadline:
        time.sleep(2)
        workers_json = run_iii("engine::workers::list", {})

    return workers_json


def collect_trigger_types() -> dict[str, object] | None:
    try:
        return run_iii("engine::trigger-types::list", {"include_internal": False})
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
    parser.add_argument(
        "--assert-non-empty",
        action="store_true",
        help="after writing --out, assert payload['functions'] is non-empty",
    )
    parser.add_argument(
        "--assert-file",
        help="path to an already-written interface JSON to assert; bypasses collection",
    )
    args = parser.parse_args()

    # Standalone assertion mode — bypass collection entirely.
    if args.assert_file:
        if not args.assert_non_empty:
            print("--assert-file requires --assert-non-empty", file=sys.stderr)
            return 1
        try:
            data = json.loads(pathlib.Path(args.assert_file).read_text())
        except Exception as e:
            print(f"could not read {args.assert_file}: {e}", file=sys.stderr)
            return 1
        if not data.get("functions"):
            print(
                f"::error::no worker functions in {args.assert_file} (empty)",
                file=sys.stderr,
            )
            return 1
        return 0

    if not args.worker:
        print("--worker is required unless --assert-file is given", file=sys.stderr)
        return 1

    baseline_json = None
    if args.trigger_types_baseline:
        baseline_path = pathlib.Path(args.trigger_types_baseline)
        if baseline_path.exists():
            baseline_json = json.loads(baseline_path.read_text(encoding="utf-8"))

    workers_json = wait_for_worker(args.worker, args.wait_seconds)
    functions_json = run_iii("engine::functions::list", {"include_internal": True})
    trigger_types_json = collect_trigger_types()

    interface = normalize_worker_interface(
        worker_name=args.worker,
        workers_json=workers_json,
        functions_json=functions_json,
        trigger_types_json=trigger_types_json,
        baseline_trigger_types_json=baseline_json,
    )
    pathlib.Path(args.out).write_text(json.dumps(interface, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(interface, indent=2))

    if args.assert_non_empty and not args.assert_file:
        with open(args.out) as f:
            data = json.load(f)
        if not data.get("functions"):
            print(f"::error::no worker functions in {args.out} (empty)", file=sys.stderr)
            return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
