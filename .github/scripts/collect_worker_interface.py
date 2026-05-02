#!/usr/bin/env python3
import argparse
import json
import pathlib
import subprocess
import sys

from build_publish_payload import normalize_worker_interface


def run_iii(function_id: str, payload: dict[str, object]) -> dict[str, object]:
    completed = subprocess.run(
        [
            "iii",
            "trigger",
            "--function-id",
            function_id,
            "--payload",
            json.dumps(payload),
        ],
        check=True,
        text=True,
        capture_output=True,
    )
    return json.loads(completed.stdout)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True)
    parser.add_argument("--out", default="worker-interface.json")
    parser.add_argument("--allow-missing-triggers", action="store_true")
    args = parser.parse_args()

    workers_json = run_iii("engine::workers::list", {})
    functions_json = run_iii("engine::functions::list", {"include_internal": True})

    triggers_json = None
    try:
        triggers_json = run_iii("engine::triggers::list", {"include_internal": True})
    except (subprocess.CalledProcessError, json.JSONDecodeError) as exc:
        if not args.allow_missing_triggers:
            raise RuntimeError(
                "could not collect triggers; confirm the engine exposes "
                "`engine::triggers::list` or pass --allow-missing-triggers"
            ) from exc

    interface = normalize_worker_interface(
        worker_name=args.worker,
        workers_json=workers_json,
        functions_json=functions_json,
        triggers_json=triggers_json,
    )
    pathlib.Path(args.out).write_text(json.dumps(interface, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(interface, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
