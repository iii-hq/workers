#!/usr/bin/env python3
"""Parse workflow_dispatch workers input into a GitHub Actions matrix JSON array.

Accepts a comma-separated list of worker folder names or the special value
``all`` (every worker allowed by create-tag).  Writes the deduplicated list as
JSON to ``$GITHUB_OUTPUT`` under ``--out-key`` (default ``matrix``).
"""
from __future__ import annotations

import argparse
import json
import os
import sys

ALLOWED_WORKERS: tuple[str, ...] = (
    "acp",
    "console",
    "database",
    "harness",
    "iii-directory",
    "image-resize",
    "mcp",
    "shell",
    "storage",
    "web",
    "worktree",
)

_ALLOWED_SET = frozenset(ALLOWED_WORKERS)


def parse_workers(raw: str) -> list[str]:
    """Return a deduplicated worker list preserving first-seen order."""
    text = raw.strip()
    if not text:
        raise ValueError("workers input is empty")

    if text.lower() == "all":
        return list(ALLOWED_WORKERS)

    names: list[str] = []
    for part in text.split(","):
        name = part.strip()
        if name:
            names.append(name)

    if not names:
        raise ValueError("workers input is empty")

    unknown = sorted({n for n in names if n not in _ALLOWED_SET})
    if unknown:
        allowed = ", ".join(ALLOWED_WORKERS)
        raise ValueError(
            f"unknown worker(s): {', '.join(unknown)}. "
            f"Allowed: {allowed} (or use all)"
        )

    seen: set[str] = set()
    deduped: list[str] = []
    for name in names:
        if name not in seen:
            seen.add(name)
            deduped.append(name)
    return deduped


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--workers",
        required=True,
        help='Comma-separated worker names or "all"',
    )
    parser.add_argument(
        "--out-key",
        default="matrix",
        help="GITHUB_OUTPUT key for the JSON array (default: matrix)",
    )
    args = parser.parse_args()

    try:
        workers = parse_workers(args.workers)
    except ValueError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1

    payload = json.dumps(workers)
    gha_out = os.environ.get("GITHUB_OUTPUT")
    if gha_out:
        with open(gha_out, "a", encoding="utf-8") as f:
            f.write(f"{args.out_key}={payload}\n")

    print(f"::notice::publish skills for {len(workers)} worker(s): {', '.join(workers)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
