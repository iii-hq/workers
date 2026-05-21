#!/usr/bin/env python3
"""CLI for reading/bumping/verifying language manifests.

Subcommands:
    read <path>            print the manifest's version to stdout
    bump <path> --kind ... bump the version in-place
    verify <path> --expected V
                           assert the file's version equals V
    deploy-mode <worker>   print the interface-collection mode

Exit codes: 0 on success, 1 on parse / IO / mismatch failure.
"""
from __future__ import annotations

import argparse
import sys
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
        new = _lib.bump(current, args.kind)
        _lib.write_version(path, new)
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(new)
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


def cmd_deploy_mode(args: argparse.Namespace) -> int:
    """Mirrors the historic _publish-registry.yml heredoc.

    Decides how _publish-registry should start a local copy of the worker
    for interface collection. Output is one of:
      release-binary  -> download binary tarball from GitHub Release
      release-bundle  -> download bundle tar.gz from GitHub Release
      iii-add         -> `iii worker add ./<dir>` (worker is self-bootstrapping)
      cargo-run       -> `cargo run` from source
      unsupported     -> can't collect locally
    """
    worker_dir = Path(args.worker_dir)
    try:
        m = _lib.read_iii_worker_yaml(worker_dir)
    except (FileNotFoundError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    raw = m.raw
    scripts = raw.get("scripts") or {}
    runtime = raw.get("runtime") or {}
    has_scripts_start = bool(str(scripts.get("start") or "").strip())
    has_runtime = bool(runtime.get("kind") or runtime.get("language"))

    if m.deploy == "binary":
        print("release-binary")
    elif m.deploy == "bundle":
        print("release-bundle")
    elif has_scripts_start or has_runtime:
        print("iii-add")
    elif (m.language or "").lower() == "rust":
        print("cargo-run")
    else:
        print("unsupported")
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="manifest_version.py")
    sub = p.add_subparsers(dest="cmd", required=True)

    p_read = sub.add_parser("read", help="print the manifest version")
    p_read.add_argument("manifest", help="path to Cargo.toml/package.json/pyproject.toml")
    p_read.set_defaults(func=cmd_read)

    p_bump = sub.add_parser("bump", help="bump the manifest version in place")
    p_bump.add_argument("manifest")
    p_bump.add_argument("--kind", choices=["patch", "minor", "major"], required=True)
    p_bump.set_defaults(func=cmd_bump)

    p_verify = sub.add_parser("verify", help="assert the manifest version equals --expected")
    p_verify.add_argument("manifest")
    p_verify.add_argument("--expected", required=True)
    p_verify.set_defaults(func=cmd_verify)

    p_dm = sub.add_parser("deploy-mode", help="print the interface-collection mode")
    p_dm.add_argument("worker_dir")
    p_dm.set_defaults(func=cmd_deploy_mode)

    args = p.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
