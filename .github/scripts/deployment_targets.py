"""Release target policy shared by the release train and Registry publisher."""

from __future__ import annotations

from collections.abc import Iterable


UNIX_TARGETS = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
    "armv7-unknown-linux-gnueabihf",
]

WINDOWS_TARGETS = [
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
]

# Every worker that publishes Windows binaries ships both supported msvc
# triples, so the default matrix carries them: a worker without Windows is
# the exception and must say so in its catalog entry.
#
# Dropped on 2026-09-03: `i686-pc-windows-msvc`, because 64-bit Windows runs
# 32-bit binaries through WOW64 and Windows 11 has no 32-bit edition; and
# `x86_64-apple-darwin`, which was the only consumer of the paid
# `workers-release-macos-12core` pool, since retired with it. Removing a
# triple here makes it an
# unknown target, so a catalog entry that still names it fails the compile
# instead of silently publishing a narrower set of binaries.
DEFAULT_TARGETS = [*UNIX_TARGETS, *WINDOWS_TARGETS]

TARGET_RUNNERS = {
    "aarch64-apple-darwin": "macos-latest",
    "x86_64-unknown-linux-gnu": "ubuntu-22.04",
    "x86_64-unknown-linux-musl": "ubuntu-latest",
    "aarch64-unknown-linux-gnu": "ubuntu-22.04",
    "armv7-unknown-linux-gnueabihf": "ubuntu-22.04",
    "x86_64-pc-windows-msvc": "windows-latest",
    "aarch64-pc-windows-msvc": "windows-latest",
}

# The release runner group is restricted to the release workflows on main.
# Keep the execution label alongside the target-to-OS policy so every
# Release Control path uses the same isolated GitHub-hosted capacity.
TARGET_LARGER_RUNNERS = {
    # macOS ships Apple Silicon only, built on a dedicated native M2 pool.
    "aarch64-apple-darwin": "workers-release-macos-arm-5core",
    "x86_64-unknown-linux-gnu": "workers-release-linux-8core",
    "x86_64-unknown-linux-musl": "workers-release-linux-8core",
    "aarch64-unknown-linux-gnu": "workers-release-linux-8core",
    "armv7-unknown-linux-gnueabihf": "workers-release-linux-8core",
    # Windows has no self-hosted pool; GitHub-hosted capacity keeps these
    # targets schedulable without competing for the release runners.
    "x86_64-pc-windows-msvc": "windows-latest",
    "aarch64-pc-windows-msvc": "windows-latest",
}


def normalize_targets(raw: object, *, deploy: str | None = "binary") -> list[str]:
    """Return canonical targets, rejecting unknown triples.

    An omitted target list means every supported target for binaries, Windows
    included. A worker that cannot ship Windows declares that explicitly in the
    catalog rather than omitting the triples silently.
    """
    if raw is None or raw == "":
        return list(DEFAULT_TARGETS) if deploy == "binary" else []
    if isinstance(raw, str):
        values: Iterable[object] = raw.split(",")
    elif isinstance(raw, list):
        values = raw
    else:
        raise ValueError("`targets` must be a comma-separated string or list")

    requested: list[str] = []
    for value in values:
        if not isinstance(value, str) or not value.strip():
            raise ValueError("`targets` entries must be non-empty strings")
        target = value.strip()
        if target not in TARGET_RUNNERS:
            raise ValueError(f"unknown release target: {target}")
        if target in requested:
            raise ValueError(f"duplicate release target: {target}")
        requested.append(target)

    if deploy == "binary" and not requested:
        raise ValueError("binary workers must declare at least one release target")
    return [target for target in DEFAULT_TARGETS if target in requested]
