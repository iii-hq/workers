#!/usr/bin/env python3
"""Validate the immutable iii compiler pin used by release workflows."""

from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path


FIELDS = {"repository", "commit", "cargo_manifest", "binary"}


def read_pin(path: Path) -> dict[str, str]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or set(value) != FIELDS:
        raise SystemExit(f"{path}: expected exactly {sorted(FIELDS)}")
    if not all(isinstance(value[field], str) and value[field] for field in FIELDS):
        raise SystemExit(f"{path}: every pin field must be a non-empty string")
    if not re.fullmatch(r"[0-9a-f]{40}", value["commit"]):
        raise SystemExit(f"{path}: commit must be a full lowercase 40-character SHA")
    if value["commit"] == "0" * 40:
        raise SystemExit(f"{path}: compiler pin is unresolved (all-zero fail-closed placeholder)")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", value["repository"]):
        raise SystemExit(f"{path}: repository must be an owner/name pair")
    for field in ("cargo_manifest", "binary"):
        candidate = Path(value[field])
        if candidate.is_absolute() or ".." in candidate.parts:
            raise SystemExit(f"{path}: {field} must be a safe relative path")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pin", type=Path, default=Path(".github/release-compiler.json"))
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--allow-env", action="store_true")
    args = parser.parse_args()
    if args.allow_env and os.environ.get("III_BIN"):
        print(os.environ["III_BIN"])
        return 0
    pin = read_pin(args.pin)
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            for key, value in pin.items():
                output.write(f"{key}={value}\n")
    print(json.dumps(pin, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
