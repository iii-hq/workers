#!/usr/bin/env python3
"""Resolve the bundle artefact URL + sha256 uploaded by `_bundle.yml`.

`_bundle.yml` uploads two files to the GitHub Release for a tag:
    <worker>.tar.gz     # the archive containing index.mjs + iii.worker.yaml
    <worker>.sha256     # `<digest>  <worker>.tar.gz` (one line)

This CLI mirrors `resolve_binary_artifacts.py` but for the single-artefact
shape used by `PublishRequestBundle`. It writes a JSON file like:

    { "archive_url": "https://.../harness.tar.gz",
      "sha256":      "9f86d08..." }

…which `build_publish_payload.py` reads to populate the `bundle`
`/publish` payload.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys
import urllib.request


def read_checksum_url(url: str) -> str:
    with urllib.request.urlopen(url, timeout=20) as response:
        text = response.read().decode("utf-8").strip()
    # `sha256sum` emits `<digest>  <filename>`; take the first token.
    return text.split()[0]


def resolve_bundle_artifact(*, repo_url: str, tag: str, worker: str) -> dict[str, str]:
    base = f"{repo_url}/releases/download/{tag}"
    archive_url = f"{base}/{worker}.tar.gz"
    sha_url = f"{base}/{worker}.sha256"
    try:
        sha256 = read_checksum_url(sha_url)
    except Exception as exc:  # noqa: BLE001 — surfaced to the workflow log
        raise RuntimeError(f"could not read checksum from {sha_url}: {exc}") from exc
    return {"archive_url": archive_url, "sha256": sha256}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-url", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--out", default="bundle.json")
    args = parser.parse_args()

    bundle = resolve_bundle_artifact(
        repo_url=args.repo_url,
        tag=args.tag,
        worker=args.worker,
    )
    pathlib.Path(args.out).write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(bundle, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
