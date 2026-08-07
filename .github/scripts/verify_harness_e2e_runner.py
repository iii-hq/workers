#!/usr/bin/env python3
"""Verify an immutable Harness E2E runner bundle before execution."""

from __future__ import annotations

import argparse
import hashlib
import json
import tarfile
from pathlib import Path
from typing import Any


class VerificationError(ValueError):
    """Raised when runner evidence is missing or contradictory."""


def verify_runner(
    *, metadata_path: Path, archive_path: Path, source_sha: str
) -> dict[str, Any]:
    try:
        metadata = json.loads(metadata_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise VerificationError(f"cannot read runner metadata: {error}") from error
    if not isinstance(metadata, dict):
        raise VerificationError("runner metadata must be an object")
    if metadata.get("schema_version") != 1:
        raise VerificationError("unsupported runner metadata schema")
    if metadata.get("source_sha") != source_sha:
        raise VerificationError("runner source SHA does not match the requested source")

    try:
        digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    except OSError as error:
        raise VerificationError(f"cannot read runner archive: {error}") from error
    if metadata.get("sha256") != digest:
        raise VerificationError("runner archive checksum does not match metadata")

    scenarios = metadata.get("scenario_ids")
    if (
        not isinstance(scenarios, list)
        or not scenarios
        or not all(isinstance(item, str) and item for item in scenarios)
        or len(set(scenarios)) != len(scenarios)
    ):
        raise VerificationError("runner scenario catalog must be unique and non-empty")
    if not isinstance(metadata.get("runner_version"), str) or not metadata[
        "runner_version"
    ]:
        raise VerificationError("runner version is required")

    try:
        with tarfile.open(archive_path, "r:gz") as archive:
            members = archive.getnames()
    except (OSError, tarfile.TarError) as error:
        raise VerificationError(f"cannot inspect runner archive: {error}") from error
    if members != ["harness-e2e"]:
        raise VerificationError("runner archive must contain only harness-e2e")
    return metadata


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    try:
        metadata = verify_runner(
            metadata_path=args.metadata,
            archive_path=args.archive,
            source_sha=args.source_sha,
        )
    except VerificationError as error:
        raise SystemExit(f"invalid_runner: {error}") from error
    print(json.dumps(metadata, sort_keys=True))


if __name__ == "__main__":
    main()
