# /// script
# requires-python = ">=3.11"
# dependencies = ["ruamel.yaml==0.18.16"]
# ///
"""Download a template and use matching workers from this source checkout."""

import argparse
from contextlib import contextmanager, ExitStack
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import tempfile
import unicodedata

from ruamel.yaml import YAML, YAMLError

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
    entries = []
    namespace = {}
    for record in listing.split(b"\0"):
        if not record:
            continue
        header, raw_path = record.split(b"\t", 1)
        mode, kind, oid = header.decode().split()
        relative = PurePosixPath(os.fsdecode(raw_path))
        if relative.is_absolute() or any(
            part == ".." or part.casefold() == ".git" for part in relative.parts
        ):
            raise ValueError(f"Unsafe download path: {relative}")
        # A gitlink points outside this repository's contents; do not silently omit it.
        if kind != "blob" or mode not in ("100644", "100755", "120000"):
            raise ValueError(f"Unsupported Git entry ({mode} {kind}): {relative}")
        # Validate the complete tree before materializing even the first blob.
        # Track ancestors too: Config (symlink) must conflict with config/file.
        # Unicode normalization also accounts for decomposing macOS filesystems.
        for length in range(1, len(relative.parts) + 1):
            prefix = relative.parts[:length]
            key = tuple(unicodedata.normalize("NFC", part.casefold()) for part in prefix)
            directory = length < len(relative.parts)
            previous = namespace.get(key)
            if previous is not None and (previous != (prefix, directory) or not directory):
                raise ValueError(f"Conflicting download paths: {PurePosixPath(*previous[0])} and {relative}")
            namespace[key] = (prefix, directory)
        entries.append((relative, mode, oid))

    for relative, mode, oid in entries:
        content = git(args, "cat-file", "blob", oid)
        target = stage / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if mode == "120000":
            target.symlink_to(os.fsdecode(content))
        else:
            target.write_bytes(content)
            target.chmod(0o755 if mode == "100755" else 0o644)
    return [relative for relative, _, _ in entries]


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


def configure_local_workers(compose, destination, workers_root):
    """Adapt a staged Compose file without changing other template files."""
    # A template may have no Compose file, or may carry a link to another file.
    # Never follow a downloaded link while preparing local configuration.
    if compose.is_symlink() or not compose.is_file():
        return []
    yaml = YAML()
    yaml.preserve_quotes = True
    yaml.indent(mapping=2, sequence=4, offset=2)
    document = yaml.load(compose)
    containers = document.get("containers", {}) if isinstance(document, dict) else {}
    if not isinstance(containers, dict):
        return []
    changed = []
    for key, container in containers.items():
        if not isinstance(container, dict):
            continue
        reference = container.get("worker")
        if not isinstance(reference, str):
            continue
        match = re.fullmatch(r"package://([a-z0-9][a-z0-9_-]*)", reference)
        if not match:
            continue
        name = match[1]
        source = workers_root / name
        manifest_path = source / "iii.worker.yaml"
        if not manifest_path.is_file():
            continue
        manifest = yaml.load(manifest_path)
        scripts = container.get("scripts") or {}
        if not isinstance(manifest, dict) or not isinstance(scripts, dict):
            raise ValueError(f"Invalid local worker manifest or scripts: {name}")
        if not scripts.get("run") and manifest.get("language") == "rust":
            if not (source / "Cargo.toml").is_file():
                raise ValueError(f"Missing Cargo.toml for local worker: {name}")
            binary = manifest.get("bin", name)
            scripts["run"] = f"cargo run --bin {shlex.quote(binary)}"
            container["scripts"] = scripts
        elif not scripts.get("run") and not (manifest.get("scripts") or {}).get("start"):
            # Keep published workers when the local checkout has no start command.
            continue
        container["worker"] = f"path://{Path(os.path.relpath(source, destination)).as_posix()}"
        # Keep the version field and its attached comments. Compose ignores it
        # for path:// sources; retaining it also preserves commented providers.
        changed.append((key, container["worker"]))
    if changed:
        yaml.dump(document, compose)
    return changed


@contextmanager
def open_directory(path, *, dir_fd=None):
    """Pin a real directory inode; never follow a symlink for this component."""
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=dir_fd)
    try:
        yield fd
    finally:
        os.close(fd)


def install_download(destination, stage, paths, *, overwrite):
    """Create and replace entries relative to pinned, no-follow directory handles."""
    with open_directory(destination.parent) as base_fd:
        if not overwrite:
            # Fail if another process creates the destination after our check:
            # a newly appeared folder has not received overwrite confirmation.
            os.mkdir(destination.name, dir_fd=base_fd)
        with open_directory(destination.name, dir_fd=base_fd) as root_fd:
            for relative in paths:
                with ExitStack() as directories:
                    parent_fd = root_fd
                    for part in relative.parts[:-1]:
                        try:
                            os.mkdir(part, dir_fd=parent_fd)
                        except FileExistsError:
                            pass
                        parent_fd = directories.enter_context(open_directory(part, dir_fd=parent_fd))
                    # Replace a leaf symlink itself, never traverse its target.
                    os.replace(stage / relative, relative.name, dst_dir_fd=parent_fd)


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
        changed = configure_local_workers(
            stage / "worker-compose.yaml", destination, args.root.parent,
        )
        check_destination(destination, paths)
        overwrite = destination.exists()
        if overwrite and not args.dry_run:
            if not confirm_overwrite(destination):
                raise SystemExit("Download cancelled. No files were changed.")
            # Recheck filesystem boundaries after waiting for the user's answer.
            check_destination(destination, paths)
        print(f"Template: {args.template}; destination: {destination}")
        print(f"Upstream commit: {args.commit}")
        print(f"{'Would download' if args.dry_run else 'Downloading'} {len(paths)} files")
        for relative in paths:
            print(f"  {relative}")
        for key, source in changed:
            print(f"  {'Would use' if args.dry_run else 'Using'} local worker {key}: {source}")
        if args.dry_run:
            return
        install_download(destination, stage, paths, overwrite=overwrite)
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
    except YAMLError:
        # Parser errors can contain configuration values. Do not print them.
        parser.exit(1, "Sync failed: invalid YAML in Compose or a local worker manifest.\n")
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Sync failed: {error}\n")


if __name__ == "__main__":
    main()
