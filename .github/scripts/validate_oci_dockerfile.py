#!/usr/bin/env python3
"""Fail closed on mutable inputs in release OCI Dockerfiles."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


SHA_IMAGE = re.compile(r"@sha256:[0-9a-f]{64}$")
CHECKSUM_OPTION = re.compile(r"(?:^|\s)--checksum=sha256:[0-9a-f]{64}(?:\s|$)")


def instructions(path: Path) -> list[tuple[int, str]]:
    parsed: list[tuple[int, str]] = []
    pending = ""
    start = 0
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = raw.strip()
        if not pending and (not stripped or stripped.startswith("#")):
            continue
        if not pending:
            start = number
        pending += (" " if pending else "") + stripped.rstrip("\\").rstrip()
        if stripped.endswith("\\"):
            continue
        parsed.append((start, pending))
        pending = ""
    if pending:
        raise SystemExit(f"{path}:{start}: unterminated Dockerfile continuation")
    return parsed


def validate(path: Path) -> None:
    errors: list[str] = []
    for number, instruction in instructions(path):
        keyword, _, arguments = instruction.partition(" ")
        keyword = keyword.upper()
        lower = arguments.lower()
        if keyword == "FROM":
            fields = arguments.split()
            while fields and fields[0].startswith("--"):
                fields.pop(0)
            image = fields[0] if fields else ""
            if image != "scratch" and not SHA_IMAGE.search(image):
                errors.append(f"{path}:{number}: FROM image must be pinned by sha256 digest")
        if keyword == "ADD" and re.search(r"https?://", arguments):
            if not CHECKSUM_OPTION.search(arguments):
                errors.append(f"{path}:{number}: remote ADD must declare a sha256 checksum")
        if keyword == "RUN":
            if re.search(r"\b(?:curl|wget)\b[^|]*\|\s*(?:ba)?sh\b", lower):
                errors.append(f"{path}:{number}: network installer pipe is forbidden")
            if "install.sh" in lower or "/releases/latest/" in lower:
                errors.append(f"{path}:{number}: mutable installer or latest release URL is forbidden")
            if re.search(r"\bpip(?:3)?\s+install\b", lower):
                errors.append(f"{path}:{number}: pip install is not lockfile-enforced; use uv sync --frozen")
            if "uv sync" in lower and "--frozen" not in lower:
                errors.append(f"{path}:{number}: uv sync must use the committed lock without re-resolution")
            if "urllib.request" in lower and "sha256sum -c" not in lower:
                errors.append(f"{path}:{number}: downloaded bytes must be checked before extraction")
    if errors:
        raise SystemExit("\n".join(errors))


def descriptors(index_dir: Path) -> list[Path]:
    index = json.loads((index_dir / "deployment-descriptor-index.json").read_text(encoding="utf-8"))
    workers = index.get("workers")
    if not isinstance(workers, dict):
        raise SystemExit("release descriptor index workers must be an object")
    result: list[Path] = []
    for slug, entry in sorted(workers.items()):
        if not isinstance(entry, dict) or entry.get("path") != f"descriptors/{slug}.json":
            raise SystemExit(f"descriptor index path differs for {slug}")
        descriptor = json.loads((index_dir / str(entry["path"])).read_text(encoding="utf-8"))
        package = descriptor.get("package")
        if not isinstance(package, dict):
            raise SystemExit(f"descriptor package differs for {slug}")
        artifact = package.get("artifact")
        source = package.get("source")
        if isinstance(artifact, dict) and artifact.get("kind") == "oci-image":
            if not isinstance(source, dict):
                raise SystemExit(f"descriptor source differs for {slug}")
            result.append(Path(str(source["path"])) / str(artifact["dockerfile"]))
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", type=Path, nargs="*")
    parser.add_argument("--index-dir", type=Path)
    args = parser.parse_args()
    paths = list(args.paths)
    if args.index_dir is not None:
        paths.extend(descriptors(args.index_dir))
    if not paths:
        raise SystemExit("at least one Dockerfile or descriptor index is required")
    for path in paths:
        if not path.is_file():
            raise SystemExit(f"OCI Dockerfile is not a regular file: {path}")
        validate(path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
