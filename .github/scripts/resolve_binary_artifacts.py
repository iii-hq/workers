#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys
import urllib.request
from collections.abc import Callable

import deployment_targets


def read_checksum_url(url: str) -> str:
    with urllib.request.urlopen(url, timeout=20) as response:
        text = response.read().decode("utf-8").strip()
    return text.split()[0]


def build_binary_artifact_map(
    *,
    repo_url: str,
    tag: str,
    bin_name: str,
    targets: list[str],
    read_checksum: Callable[[str], str],
) -> dict[str, dict[str, str]]:
    base = f"{repo_url}/releases/download/{tag}"
    binaries = {}
    missing = []
    for target in targets:
        if target not in deployment_targets.TARGET_RUNNERS:
            raise ValueError(f"unsupported release target: {target}")
        ext = "tar.gz"
        asset_url = f"{base}/{bin_name}-{target}.{ext}"
        sha_url = f"{base}/{bin_name}-{target}.sha256"
        try:
            binaries[target] = {
                "url": asset_url,
                "sha256": read_checksum(sha_url),
            }
        except Exception as exc:
            missing.append(target)
            print(f"::error::missing checksum for {target}: {exc}", file=sys.stderr)
    if missing:
        raise RuntimeError(f"missing binary artefacts for targets: {', '.join(missing)}")
    if not binaries:
        raise RuntimeError("no binary artefacts could be resolved")
    return binaries


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-url", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--bin", required=True)
    parser.add_argument(
        "--targets",
        default=",".join(deployment_targets.DEFAULT_TARGETS),
        help="Exact comma-separated Unix targets; Windows targets are rejected",
    )
    parser.add_argument("--out", default="binaries.json")
    args = parser.parse_args()

    binaries = build_binary_artifact_map(
        repo_url=args.repo_url,
        tag=args.tag,
        bin_name=args.bin,
        targets=deployment_targets.normalize_targets(args.targets),
        read_checksum=read_checksum_url,
    )
    pathlib.Path(args.out).write_text(json.dumps(binaries, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(binaries, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
