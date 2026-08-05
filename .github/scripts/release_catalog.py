#!/usr/bin/env python3
"""Validate and query the single worker release catalog."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import _lib  # noqa: E402

CATALOG_PATH = Path(".github/release-workers.yaml")


def load_catalog(path: Path = CATALOG_PATH) -> dict[str, dict[str, Any]]:
    import yaml

    raw = yaml.safe_load(path.read_text()) or {}
    if raw.get("schema_version") != 1:
        raise ValueError("release catalog schema_version must be 1")
    defaults = raw.get("defaults") or {}
    standard = raw.get("standard_workers") or []
    special = raw.get("special_workers") or {}
    policies = raw.get("policies") or {}
    if not isinstance(standard, list) or not isinstance(special, dict) or not isinstance(policies, dict):
        raise ValueError("invalid release catalog shape")

    catalog: dict[str, dict[str, Any]] = {}
    for slug in standard:
        if not isinstance(slug, str) or not slug:
            raise ValueError("standard worker slugs must be non-empty strings")
        catalog[slug] = {**defaults, "slug": slug, "manifest": None}
    for slug, config in special.items():
        if slug in catalog:
            raise ValueError(f"duplicate release worker: {slug}")
        if not isinstance(config, dict):
            raise ValueError(f"special worker {slug} config must be an object")
        catalog[slug] = {**defaults, **config, "slug": slug}
    for slug, policy in policies.items():
        if slug not in catalog:
            raise ValueError(f"policy references unknown release worker: {slug}")
        if not isinstance(policy, dict):
            raise ValueError(f"policy for {slug} must be an object")
        catalog[slug].update(policy)

    for slug, config in catalog.items():
        if config.get("release_workflow") not in {"release.yml", "release-lsp-vscode.yml"}:
            raise ValueError(f"{slug}: unsupported release_workflow")
        if config.get("required_validation") not in {"smoke", "full"}:
            raise ValueError(f"{slug}: required_validation must be smoke|full")
        if not isinstance(config.get("allow_direct_latest"), bool):
            raise ValueError(f"{slug}: allow_direct_latest must be boolean")
    return catalog


def validate_checkout(catalog: dict[str, dict[str, Any]], root: Path = Path(".")) -> None:
    for slug, config in catalog.items():
        worker_dir = root / slug
        if not worker_dir.is_dir():
            raise ValueError(f"release worker directory is missing: {slug}")
        manifest = config.get("manifest")
        if manifest:
            if not (worker_dir / str(manifest)).is_file():
                raise ValueError(f"{slug}: manifest is missing: {manifest}")
            continue
        parsed = _lib.read_iii_worker_yaml(worker_dir)
        if parsed.name != slug:
            raise ValueError(f"{slug}: iii.worker.yaml name is {parsed.name!r}")
        if not parsed.manifest or not (worker_dir / parsed.manifest).is_file():
            raise ValueError(f"{slug}: declared manifest is missing: {parsed.manifest}")


def resolved_entries(
    catalog: dict[str, dict[str, Any]], root: Path = Path(".")
) -> list[dict[str, Any]]:
    """Return catalog entries with inherited manifests made explicit."""
    entries: list[dict[str, Any]] = []
    for slug, config in catalog.items():
        entry = dict(config)
        if not entry.get("manifest"):
            entry["manifest"] = _lib.read_iii_worker_yaml(root / slug).manifest
        entries.append(entry)
    return entries


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalog", type=Path, default=CATALOG_PATH)
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("validate")
    get = subparsers.add_parser("get")
    get.add_argument("worker")
    get.add_argument(
        "field",
        choices=("release_workflow", "manifest", "allow_direct_latest", "required_validation"),
    )
    subparsers.add_parser("json")
    args = parser.parse_args(argv)

    try:
        catalog = load_catalog(args.catalog)
        if args.command == "validate":
            validate_checkout(catalog)
            print(f"validated {len(catalog)} release workers")
        elif args.command == "json":
            validate_checkout(catalog)
            print(json.dumps(resolved_entries(catalog), sort_keys=True))
        else:
            if args.worker not in catalog:
                raise ValueError(f"worker is not releasable: {args.worker}")
            value = catalog[args.worker][args.field]
            if args.field == "manifest" and not value:
                value = _lib.read_iii_worker_yaml(Path(args.worker)).manifest
            if isinstance(value, bool):
                print(str(value).lower())
            else:
                print(value)
    except (FileNotFoundError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
