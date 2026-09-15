"""Apply the data-only snapshot fetched by sync.sh. Never execute upstream code."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOTS = ("agents", "skills", "upstream")
SOURCE = "iii/harness/"
MANIFEST = "upstream/sync.json"



def snapshot(root):
    result = {}
    for name in ROOTS:
        directory = root / name
        if directory.is_symlink() or (directory.exists() and not directory.is_dir()):
            raise ValueError(f"Refusing non-directory or symlink destination: {directory}")
        for path in sorted(directory.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"Refusing symlink in generated files: {path}")
            if path.is_file() and path.relative_to(root).as_posix() != MANIFEST:
                result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def stage_snapshot(args, stage):
    for name in ROOTS:
        (stage / name).mkdir()
    listing = subprocess.check_output([
        "git", "-C", args.checkout, "ls-tree", "-rz", args.commit, "--", SOURCE,
    ])
    for record in listing.split(b"\0"):
        if not record:
            continue
        header, raw_path = record.split(b"\t", 1)
        mode, kind, oid = header.decode().split()
        source_path = raw_path.decode("utf-8")
        relative = Path(source_path.removeprefix(SOURCE))
        # Deliberately do not import .env, credentials, executable scripts or
        # the package-based Compose into the runnable project root.
        if any(part.startswith(".") for part in relative.parts):
            continue
        if relative.parts[0] in ("agents", "skills") and relative.suffix == ".md":
            target = relative
        elif relative.parts[0] == "config" and relative.suffix in (".yaml", ".yml", ".json"):
            if any(part in ("secrets", "credentials") for part in relative.parts):
                raise ValueError(f"Refusing sensitive configuration path: {relative}")
            # Reference only: never apply upstream settings to local config/.
            target = Path("upstream") / relative
        else:
            continue
        if mode not in ("100644", "100755") or kind != "blob":
            raise ValueError(f"Only regular upstream files are supported: {relative}")
        content = subprocess.check_output(["git", "-C", args.checkout, "cat-file", "blob", oid])
        destination = stage / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)
    for name in ("agents", "skills"):
        if not any((stage / name).rglob("*.md")):
            raise ValueError(f"Upstream {SOURCE}{name}/ has no Markdown files; nothing was changed")


def apply(args):
    current = snapshot(args.destination)
    manifest_path = args.destination / MANIFEST
    old_manifest = json.loads(manifest_path.read_text()) if manifest_path.is_file() else {}
    if current != old_manifest.get("files", {}) and not args.force:
        raise ValueError("Generated files have local edits or are unmanaged; preserve them before using --force")
    with tempfile.TemporaryDirectory(prefix=".sync-stage-", dir=args.destination) as temporary:
        stage = Path(temporary)
        stage_snapshot(args, stage)
        expected = snapshot(stage)
        manifest = {
            "repository": args.repo, "ref": args.ref, "commit": args.commit,
            "source": SOURCE, "files": expected,
        }
        (stage / MANIFEST).write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        changed = [name for name in sorted(set(current) | set(expected)) if current.get(name) != expected.get(name)]
        print(f"Upstream commit: {args.commit}")
        print(f"{'Would synchronize' if args.dry_run else 'Synchronizing'} {len(changed)} changed files")
        for name in changed:
            print(f"  {name}")
        if args.dry_run:
            return
        # Build and validate everything first, then swap directories. Roll back
        # an unsuccessful replacement rather than leaving a half-updated tree.
        backup = stage / "backup"
        backup.mkdir()
        installed = []
        try:
            for name in ROOTS:
                destination = args.destination / name
                if destination.exists():
                    destination.rename(backup / name)
                (stage / name).rename(destination)
                installed.append(name)
        except OSError:
            for name in ROOTS:
                destination = args.destination / name
                if name in installed:
                    shutil.rmtree(destination)
                if (backup / name).exists():
                    (backup / name).rename(destination)
            raise
        print("Sync complete. Local Compose, config, environment files and runtime data were not changed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("checkout", "commit", "repo", "ref"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    try:
        apply(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Sync failed: {error}\n")


if __name__ == "__main__":
    main()
