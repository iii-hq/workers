"""Release target policy shared by the release train and Registry publisher."""

from __future__ import annotations

from collections.abc import Iterable


DEFAULT_TARGETS = [
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
    "armv7-unknown-linux-gnueabihf",
]

# Windows targets are stable-profile only: they are never built for release
# candidates and only ever enter a worker's `stable_targets` by explicit
# opt-in. x86_64 is the only supported triple today; i686/aarch64 msvc extend
# this map without redesign when product support lands.
WINDOWS_TARGETS = [
    "x86_64-pc-windows-msvc",
]

# Canonical ordering for every declarable target across both profiles.
ALL_TARGETS = [*DEFAULT_TARGETS, *WINDOWS_TARGETS]

TARGET_RUNNERS = {
    "x86_64-apple-darwin": "macos-latest",
    "aarch64-apple-darwin": "macos-latest",
    "x86_64-unknown-linux-gnu": "ubuntu-22.04",
    "x86_64-unknown-linux-musl": "ubuntu-latest",
    "aarch64-unknown-linux-gnu": "ubuntu-22.04",
    "armv7-unknown-linux-gnueabihf": "ubuntu-22.04",
    "x86_64-pc-windows-msvc": "windows-latest",
}

# The release runner group is restricted to the release workflows on main.
# Keep the execution label alongside the target-to-OS policy so every
# Release Control path uses the same isolated GitHub-hosted capacity.
TARGET_LARGER_RUNNERS = {
    "x86_64-apple-darwin": "workers-release-macos-12core",
    # Build Apple Silicon artifacts on a dedicated native M2 pool. This keeps
    # the Intel and ARM macOS targets independently schedulable.
    "aarch64-apple-darwin": "workers-release-macos-arm-5core",
    "x86_64-unknown-linux-gnu": "workers-release-linux-8core",
    "x86_64-unknown-linux-musl": "workers-release-linux-8core",
    "aarch64-unknown-linux-gnu": "workers-release-linux-8core",
    "armv7-unknown-linux-gnueabihf": "workers-release-linux-8core",
    # Windows builds only run at finalization (stable-delta) and use
    # GitHub-hosted capacity until a dedicated pool proves necessary.
    "x86_64-pc-windows-msvc": "windows-latest",
}


def normalize_targets(raw: object, *, deploy: str | None = "binary", allow_windows: bool = False) -> list[str]:
    """Return canonical targets, rejecting unknown triples.

    An omitted target list means every supported Unix target for binaries.
    Windows triples are accepted only when `allow_windows` is set — the
    stable profile of an explicitly opted-in worker — so candidate manifests
    can never silently advertise unsupported artifacts.
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
            if "windows" in target:
                raise ValueError(f"Windows release target is not supported: {target}")
            raise ValueError(f"unknown release target: {target}")
        if target in WINDOWS_TARGETS and not allow_windows:
            raise ValueError(f"Windows release target is stable-profile only: {target}")
        if target in requested:
            raise ValueError(f"duplicate release target: {target}")
        requested.append(target)

    if deploy == "binary" and not requested:
        raise ValueError("binary workers must declare at least one Unix release target")
    return [target for target in ALL_TARGETS if target in requested]
