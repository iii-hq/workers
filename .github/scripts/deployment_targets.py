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

TARGET_RUNNERS = {
    "x86_64-apple-darwin": "macos-latest",
    "aarch64-apple-darwin": "macos-latest",
    "x86_64-unknown-linux-gnu": "ubuntu-22.04",
    "x86_64-unknown-linux-musl": "ubuntu-latest",
    "aarch64-unknown-linux-gnu": "ubuntu-22.04",
    "armv7-unknown-linux-gnueabihf": "ubuntu-22.04",
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
}


def normalize_targets(raw: object, *, deploy: str | None = "binary") -> list[str]:
    """Return canonical targets, rejecting Windows and unknown triples.

    An omitted target list means every supported Unix target for binaries. A
    non-binary worker has no executable target matrix, but an explicit Windows
    entry is still rejected so manifests cannot silently advertise unsupported
    artifacts.
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
        if target in requested:
            raise ValueError(f"duplicate release target: {target}")
        requested.append(target)

    if deploy == "binary" and not requested:
        raise ValueError("binary workers must declare at least one Unix release target")
    return [target for target in DEFAULT_TARGETS if target in requested]
