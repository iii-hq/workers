#!/usr/bin/env python3
"""CLI for reading/bumping/verifying language manifests.

Subcommands:
    read <path>            print the manifest's version to stdout
    bump <path> --kind ... resolve and write the release version
                           [--suffix ...] [--target VERSION]
    resolve <path> --kind ...
                           resolve without writing the manifest
    maturity <version>     print experimental|alpha|beta|stable
    check-history <worker> <version>
                           validate the target against existing git tags
    verify <path> --expected V
                           assert the file's version equals V
    sync-tags [--remote R] write every worker's highest published tag into
                           its manifest and the repository's lockfiles
    deploy-mode <worker>   print the interface-collection mode

Exit codes: 0 on success, 1 on parse / IO / mismatch failure.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path

# Make _lib importable when run as a script.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _lib  # noqa: E402


def cmd_read(args: argparse.Namespace) -> int:
    path = Path(args.manifest)
    try:
        print(_lib.read_version(path))
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


def cmd_bump(args: argparse.Namespace) -> int:
    path = Path(args.manifest)
    try:
        current = _lib.read_version(path)
        new = _lib.resolve_release_version(current, args.kind, args.suffix, args.target)
        _lib.write_version(path, new)
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(new)
    return 0


def cmd_resolve(args: argparse.Namespace) -> int:
    path = Path(args.manifest)
    try:
        current = _lib.read_version(path)
        print(_lib.resolve_release_version(current, args.kind, args.suffix, args.target))
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


def cmd_maturity(args: argparse.Namespace) -> int:
    try:
        print(_lib.release_maturity(args.version))
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


def cmd_check_history(args: argparse.Namespace) -> int:
    try:
        _lib.validate_release_history(args.version, _lib.list_tagged_versions(args.worker))
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


def cmd_verify(args: argparse.Namespace) -> int:
    path = Path(args.manifest)
    try:
        actual = _lib.read_version(path)
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    if actual != args.expected:
        print(
            f"version mismatch in {path}: expected {args.expected}, got {actual}",
            file=sys.stderr,
        )
        return 1
    return 0


def cmd_sync_lock(args: argparse.Namespace) -> int:
    """Sync a Rust worker's own version in Cargo.lock to its bumped Cargo.toml.

    No-op (exit 0) when the manifest is not a Cargo.toml or the worker has no
    committed Cargo.lock — non-Rust workers and lockless crates need nothing.
    """
    manifest = Path(args.manifest)
    if manifest.name != "Cargo.toml":
        return 0
    lock = manifest.with_name("Cargo.lock")
    if not lock.exists():
        return 0
    try:
        name = _lib.read_package_name(manifest)
        version = _lib.read_version(manifest)
        changed = _lib.sync_lock_self_version(lock, name, version)
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(f"{name} {version}" + ("" if changed else " (already in sync)"))
    return 0


def sync_manifests(
    catalog: dict[str, _lib.WorkerSpec], tags: Iterable[str], locks: Iterable[Path]
) -> tuple[list[str], list[str]]:
    """Write each catalog worker's highest `<worker>/v<version>` tag into its manifest.

    Release Control mints one tag per published Registry version, so the
    highest in-grammar tag is the newest version any channel ever received.
    Manifests only move forward: one already at or past its tag (a hand bump
    seeding a first publication) stays. Every lock in `locks` gets the bumped
    packages' local entries, since workers lock each other through path deps.
    Returns (changes, errors); one worker's error never blocks the others.
    """
    published: dict[str, list[str]] = {}
    for tag in tags:
        worker, _, version = tag.partition("/v")
        if worker not in catalog:
            continue
        try:
            _lib.parse_release_version(version)
        except ValueError:
            continue
        published.setdefault(worker, []).append(version)

    # ponytail: lock entries are matched by package name, which assumes no two
    # crates in the repository share one; a duplicate would need path matching.
    lock_versions: dict[str, dict[str, str]] = {"Cargo.lock": {}, "uv.lock": {}}
    changes: list[str] = []
    errors: list[str] = []
    for worker, versions in sorted(published.items()):
        manifest = catalog[worker].path / str(catalog[worker].manifest)
        target = max(versions, key=_lib.parse_semver)
        try:
            current = _lib.read_version(manifest)
            version = max(current, target, key=_lib.parse_semver)
            # Locks follow the manifest even when it is already current, so a
            # hand bump that skipped its lock converges on the next run.
            if manifest.name == "Cargo.toml":
                lock_versions["Cargo.lock"][_lib.read_package_name(manifest)] = version
            elif manifest.name == "pyproject.toml":
                # Also the guard: `-experimental` has no Python spelling.
                name = re.sub(r"[-_.]+", "-", _lib.read_package_name(manifest)).lower()
                lock_versions["uv.lock"][name] = _lib.pep440_version(version)
        except (FileNotFoundError, ValueError) as e:
            errors.append(f"{worker}: {e}")
            continue
        if version != current:
            _lib.write_version(manifest, version)
            changes.append(f"{worker} {current} -> {version}")
    for lock in locks:
        _lib.sync_lock_versions(lock, lock_versions.get(lock.name, {}))
    return changes, errors


def cmd_sync_tags(args: argparse.Namespace) -> int:
    root = _lib.worker_catalog_path().parents[1]
    try:
        refs = subprocess.check_output(["git", "-C", str(root), "ls-remote", "--tags", "--refs", args.remote], text=True)
        tracked = subprocess.check_output(
            ["git", "-C", str(root), "ls-files", "-z", "--", "*Cargo.lock", "*uv.lock"], text=True
        )
        catalog = _lib.read_worker_catalog()
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    tags = [line.split("\trefs/tags/", 1)[1] for line in refs.splitlines() if "\trefs/tags/" in line]
    locks = [root / path for path in tracked.split("\0") if path]
    changes, errors = sync_manifests(catalog, tags, locks)
    for change in changes:
        print(change)
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="manifest_version.py")
    sub = p.add_subparsers(dest="cmd", required=True)

    p_read = sub.add_parser("read", help="print the manifest version")
    p_read.add_argument("manifest", help="path to Cargo.toml/package.json/pyproject.toml")
    p_read.set_defaults(func=cmd_read)

    p_bump = sub.add_parser("bump", help="bump the manifest version in place")
    p_bump.add_argument("manifest")
    p_bump.add_argument("--kind", choices=["patch", "minor", "major", "none"], required=True)
    p_bump.add_argument("--suffix", choices=_lib.RELEASE_SUFFIXES, default="none")
    p_bump.add_argument(
        "--target",
        default="",
        help="exact release version; authoritative when provided",
    )
    p_bump.set_defaults(func=cmd_bump)

    p_resolve = sub.add_parser("resolve", help="resolve without writing the manifest")
    p_resolve.add_argument("manifest")
    p_resolve.add_argument("--kind", choices=["patch", "minor", "major", "none"], required=True)
    p_resolve.add_argument("--suffix", choices=_lib.RELEASE_SUFFIXES, default="none")
    p_resolve.add_argument("--target", default="")
    p_resolve.set_defaults(func=cmd_resolve)

    p_maturity = sub.add_parser("maturity", help="print a release version's maturity")
    p_maturity.add_argument("version")
    p_maturity.set_defaults(func=cmd_maturity)

    p_history = sub.add_parser("check-history", help="validate a release against existing worker tags")
    p_history.add_argument("worker")
    p_history.add_argument("version")
    p_history.set_defaults(func=cmd_check_history)

    p_verify = sub.add_parser("verify", help="assert the manifest version equals --expected")
    p_verify.add_argument("manifest")
    p_verify.add_argument("--expected", required=True)
    p_verify.set_defaults(func=cmd_verify)

    p_sl = sub.add_parser("sync-lock", help="sync Cargo.lock self-version to Cargo.toml")
    p_sl.add_argument("manifest", help="path to the bumped Cargo.toml")
    p_sl.set_defaults(func=cmd_sync_lock)

    p_st = sub.add_parser("sync-tags", help="write published tag versions into worker manifests")
    p_st.add_argument("--remote", default="origin", help="git remote whose tags are authoritative")
    p_st.set_defaults(func=cmd_sync_tags)

    args = p.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
