#!/usr/bin/env python3
"""Validate one publishable entry from the root worker-compose catalog."""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _lib  # noqa: E402


BINARY_NAME_EXCEPTIONS = frozenset({"acp"})
def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True)
    parser.add_argument("--base-ref", required=True)
    parser.add_argument("--source-changed", required=True)
    args = parser.parse_args(argv)

    worker = args.worker
    strict = worker in set(json.loads(args.source_changed))
    errors: list[str] = []

    def hard(message: str) -> None:
        errors.append(message)

    def soft(message: str) -> None:
        if strict:
            errors.append(message)
        else:
            print(f"::notice::{message} (skipped: {worker} only changed metadata)")

    try:
        spec = _lib.read_worker(worker)
    except (FileNotFoundError, ValueError) as error:
        hard(str(error))
        spec = None

    if spec is not None:
        if not spec.publish:
            hard(f"{worker}: first-party CI workers must set registry.publish=true")
        expected_path = pathlib.Path(worker).resolve()
        if spec.path != expected_path:
            hard(f"{worker}: source.path must be {worker!r}, got {spec.source.get('path')!r}")
        readme = spec.path / "README.md"
        if not readme.exists():
            soft(f"{worker}/README.md is missing")
        elif readme.stat().st_size == 0:
            soft(f"{worker}/README.md is empty")

        tags = spec.registry.get("tags")
        if tags is not None and (
            not isinstance(tags, list) or any(not isinstance(tag, str) for tag in tags)
        ):
            hard(f"{worker}: registry.tags must be an array of strings")
        elif spec.validation.get("interface") == "required" and not tags:
            hard(f"{worker}: registry.tags must be non-empty when interface validation is enabled")

        if spec.validation.get("interface") not in {"required", "skipped"}:
            hard(f"{worker}: validation.interface must be required or skipped")

        if spec.artifact_kind == "rust-binary":
            executable = spec.binary or worker
            if executable != worker and worker not in BINARY_NAME_EXCEPTIONS:
                hard(
                    f"{worker}: artifact.binary={executable!r} must match the worker ID "
                    "because Registry extraction addresses the worker by ID"
                )
        if spec.artifact_kind != "oci-image":
            command = spec.runtime.get("exec")
            if not isinstance(command, list) or not command or not all(isinstance(part, str) and part for part in command):
                hard(f"{worker}: runtime.exec must be a non-empty string array")

        if spec.manifest:
            manifest_path = spec.path / spec.manifest
            if not manifest_path.is_file():
                hard(f"{manifest_path} not found")
            else:
                try:
                    current_version = _lib.read_version(manifest_path)
                except (ValueError, FileNotFoundError) as error:
                    hard(f"could not read version from {manifest_path}: {error}")
                    current_version = None
                try:
                    base_blob = subprocess.check_output(
                        ["git", "show", f"{args.base_ref}:{worker}/{spec.manifest}"],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    )
                    base_sha = subprocess.check_output(
                        ["git", "rev-parse", args.base_ref],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                    head_sha = subprocess.check_output(
                        ["git", "rev-parse", "HEAD"],
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                except subprocess.CalledProcessError:
                    base_blob = None
                    base_sha = head_sha = ""
                if current_version and base_blob is not None and base_sha != head_sha:
                    with tempfile.TemporaryDirectory() as directory:
                        temporary = pathlib.Path(directory) / spec.manifest
                        temporary.write_text(base_blob, encoding="utf-8")
                        try:
                            base_version = _lib.read_version(temporary)
                        except (ValueError, FileNotFoundError):
                            base_version = None
                    if base_version and _lib.parse_semver(current_version) < _lib.parse_semver(base_version):
                        soft(
                            f"{worker}/{spec.manifest} version {current_version} "
                            f"is less than base {base_version}"
                        )

        tests = spec.path / "tests"
        if not tests.exists():
            soft(f"{worker}/tests/ is missing")
        elif not any(tests.iterdir()):
            soft(f"{worker}/tests/ is empty")

    for error in errors:
        print(f"::error::{error}")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
