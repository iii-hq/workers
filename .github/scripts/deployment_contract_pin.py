#!/usr/bin/env python3
"""Validate the immutable cross-repository deployment-result schema pin."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from pathlib import Path


PIN = Path(__file__).resolve().parents[1] / "deployment-control-contract.json"
LOCAL = Path(__file__).resolve().parents[1] / "contracts/deployment-execution.schema.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--github-output")
    args = parser.parse_args()
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    if set(pin) != {"repository", "commit", "path", "sha256"}:
        raise SystemExit("release-control contract pin fields differ")
    if pin["repository"] != "iii-hq/release-control":
        raise SystemExit("release-control contract repository is not canonical")
    if not re.fullmatch(r"[0-9a-f]{40}", pin["commit"]) or set(pin["commit"]) == {"0"}:
        raise SystemExit("release-control contract commit must be an immutable non-zero SHA")
    if pin["path"] != "api/contracts/deployment-execution.schema.json":
        raise SystemExit("release-control contract path is not canonical")
    actual = hashlib.sha256(LOCAL.read_bytes()).hexdigest()
    if actual != pin["sha256"]:
        raise SystemExit(f"local release contract digest mismatch: {actual}")
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as output:
            for key in ("repository", "commit", "path", "sha256"):
                output.write(f"{key}={pin[key]}\n")
    else:
        print(json.dumps(pin, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
