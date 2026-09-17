"""Download every tracked entry in a template folder without executing its code."""

import argparse
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import tempfile

RESERVED_NAMES = {"agents", "config", "data", "scripts", "skills", "tests", "upstream"}


def template_name(value):
    """Confine downloads to a single non-tooling directory beside sync.sh."""
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", value) or value in RESERVED_NAMES:
        raise argparse.ArgumentTypeError("Invalid or reserved template name")
    return value


def git(args, *command):
    """Read Git objects directly, without checkout filters or upstream scripts."""
    return subprocess.check_output(["git", "-C", args.checkout, *command])


def stage_download(args, stage):
    """Copy all blobs, hidden files, executable bits and symlinks verbatim."""
    source = f"iii/{args.template}"
    tree = subprocess.run(
        ["git", "-C", args.checkout, "rev-parse", "--verify", f"{args.commit}:{source}"],
        capture_output=True,
    )
    if tree.returncode or git(args, "cat-file", "-t", tree.stdout.decode().strip()).strip() != b"tree":
        raise ValueError(f"Template folder not found: {source}")
    listing = git(args, "ls-tree", "-rz", tree.stdout.decode().strip())
    paths = []
    for record in listing.split(b"\0"):
        if not record:
            continue
        header, raw_path = record.split(b"\t", 1)
        mode, kind, oid = header.decode().split()
        relative = PurePosixPath(os.fsdecode(raw_path))
        if relative.is_absolute() or any(part in ("..", ".git") for part in relative.parts):
            raise ValueError(f"Unsafe download path: {relative}")
        # A gitlink points outside this repository's contents; do not silently omit it.
        if kind != "blob" or mode not in ("100644", "100755", "120000"):
            raise ValueError(f"Unsupported Git entry ({mode} {kind}): {relative}")
        content = git(args, "cat-file", "blob", oid)
        target = stage / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if mode == "120000":
            target.symlink_to(os.fsdecode(content))
        else:
            target.write_bytes(content)
            target.chmod(0o755 if mode == "100755" else 0o644)
        paths.append(relative)
    return paths


def check_destination(destination, paths):
    """Prevent local symlinks or type conflicts from redirecting downloaded writes."""
    if destination.is_symlink() or (destination.exists() and not destination.is_dir()):
        raise ValueError(f"Refusing non-directory or symlink destination: {destination}")
    for relative in paths:
        for parent in relative.parents:
            directory = destination / parent
            if directory.is_symlink() or (directory.exists() and not directory.is_dir()):
                raise ValueError(f"Refusing non-directory or symlink parent: {directory}")
        target = destination / relative
        if not target.is_symlink() and target.is_dir():
            raise ValueError(f"Cannot overwrite a local directory with a file: {target}")


def confirm_overwrite(destination):
    """Require explicit consent; empty input, EOF and interruptions never approve."""
    print(f"\nThe template folder already exists: {destination}")
    print("Back up your files before continuing.")
    print("All files with matching template paths will be overwritten, including local changes.")
    print("Local-only files will be kept.")
    try:
        answer = input("Do you really want to overwrite these files? [yes/no] (default: no): ")
    except (EOFError, KeyboardInterrupt):
        print()
        return False
    return answer.strip().lower() in ("yes", "y")


def apply(args):
    """Stage the download, then overwrite matching paths and keep local-only files."""
    destination = args.root / args.template
    with tempfile.TemporaryDirectory(prefix=".sync-stage-", dir=args.root) as temporary:
        stage = Path(temporary) / "content"
        stage.mkdir()
        paths = stage_download(args, stage)
        check_destination(destination, paths)
        if destination.exists() and not args.dry_run:
            if not confirm_overwrite(destination):
                raise SystemExit("Download cancelled. No files were changed.")
            # Recheck filesystem boundaries after waiting for the user's answer.
            check_destination(destination, paths)
        print(f"Template: {args.template}; destination: {destination}")
        print(f"Upstream commit: {args.commit}")
        print(f"{'Would download' if args.dry_run else 'Downloading'} {len(paths)} files")
        for relative in paths:
            print(f"  {relative}")
        if args.dry_run:
            return
        if not destination.exists():
            stage.rename(destination)
        else:
            for relative in paths:
                target = destination / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                # Replace symlink leaves themselves, never write through their targets.
                (stage / relative).replace(target)
        print("Template downloaded! Enter the folder with the command:")
        print(f"\ncd {shlex.quote(os.path.relpath(destination))}")


def main():
    """Parse download inputs and report transport or filesystem failures."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--template", default="harness", type=template_name)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        apply(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Sync failed: {error}\n")


if __name__ == "__main__":
    main()
