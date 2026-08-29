#!/usr/bin/env python3
"""Create the deterministic, offline Scrapling runtime dependency layer."""

from __future__ import annotations

import gzip
import io
import os
import sys
import tarfile
from pathlib import Path, PurePosixPath


SKIPPED_PARTS = {"__pycache__", ".pytest_cache"}


def site_packages(venv: Path) -> Path:
    matches = sorted(venv.glob("lib/python*/site-packages"))
    if len(matches) != 1 or not matches[0].is_dir():
        raise SystemExit("expected exactly one .venv/lib/python*/site-packages directory")
    return matches[0]


def entries(root: Path) -> list[tuple[Path, PurePosixPath]]:
    selected: list[tuple[Path, PurePosixPath]] = []
    for path in sorted(root.rglob("*")):
        relative = PurePosixPath(path.relative_to(root).as_posix())
        if any(part in SKIPPED_PARTS for part in relative.parts) or path.suffix == ".pyc":
            continue
        resolved = path.resolve(strict=True) if path.is_symlink() else path
        if resolved.is_file():
            selected.append((resolved, relative))
        elif not path.is_dir():
            raise SystemExit(f"unsupported site-packages entry: {path}")
    if not selected:
        raise SystemExit("site-packages dependency layer is empty")
    return selected


def build(venv: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(mode="w", fileobj=compressed, format=tarfile.PAX_FORMAT) as archive:
                for source, relative in entries(site_packages(venv)):
                    data = source.read_bytes()
                    info = tarfile.TarInfo(relative.as_posix())
                    info.size = len(data)
                    info.mode = 0o755 if os.access(source, os.X_OK) else 0o644
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    archive.addfile(info, io.BytesIO(data))


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    build(root / ".venv", root / "dist" / "site-packages.tar.gz")
    return 0


if __name__ == "__main__":
    sys.exit(main())
