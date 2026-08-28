#!/usr/bin/env python3
"""Prepare, build, and assemble one immutable Release Train attempt."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
from pathlib import Path

import _lib
import release_targets


def run(*command: str, cwd: Path | None = None) -> None:
    print("+", " ".join(command))
    subprocess.run(command, cwd=cwd, check=True)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def manifest_for(worker: str) -> tuple[Path, _lib.WorkerManifest]:
    root = Path(worker)
    return root, _lib.read_iii_worker_yaml(root)


def target_policy(manifest: _lib.WorkerManifest, requested: str | None) -> list[str]:
    raw = requested if requested is not None else manifest.raw.get("targets")
    targets = release_targets.normalize_targets(raw, deploy=manifest.deploy)
    declared = release_targets.normalize_targets(manifest.raw.get("targets"), deploy=manifest.deploy)
    if requested is not None and targets != declared:
        raise SystemExit(
            f"{manifest.name}: workflow targets {targets} do not match manifest targets {declared}"
        )
    return targets


def package_binary(binary_path: Path, binary: str, target: str, out: Path) -> list[Path]:
    archive = out / f"{binary}-{target}.tar.gz"
    with tarfile.open(archive, "w:gz") as handle:
        handle.add(binary_path, arcname=binary)
    checksum = out / f"{binary}-{target}.sha256"
    checksum.write_text(f"{sha256(archive)}  {archive.name}\n", encoding="utf-8")
    return [archive, checksum]


def package_source(worker: str, out: Path) -> Path:
    archive = out / f"{worker}.tar.gz"
    with tarfile.open(archive, "w:gz") as handle:
        handle.add(worker, arcname=".", filter=lambda item: None if ".git" in item.name.split("/") else item)
    return archive


def prepare(args: argparse.Namespace) -> int:
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != args.source_sha:
        raise SystemExit(f"source SHA mismatch: checkout={actual}, requested={args.source_sha}")
    if not _lib.RELEASE_VERSION_RE.fullmatch(args.candidate_version):
        raise SystemExit(f"invalid candidate version: {args.candidate_version}")

    root, manifest = manifest_for(args.worker)
    targets = target_policy(manifest, args.targets)
    run(
        "python3",
        ".github/scripts/manifest_version.py",
        "bump",
        str(root / manifest.manifest),
        "--kind",
        "none",
        "--target",
        args.candidate_version,
    )
    run("python3", ".github/scripts/manifest_version.py", "verify", str(root / manifest.manifest), "--expected", args.candidate_version)
    run("python3", ".github/scripts/manifest_version.py", "sync-lock", str(root / manifest.manifest))
    if manifest.deploy == "binary":
        cargo = root / manifest.manifest
        # Bumping a worker can also change the versions of path dependencies
        # recorded in its lockfile (for example, an RC may depend on another
        # worker that was prepared earlier in the train).  Allow Cargo to
        # refresh those local package entries before the prepared commit is
        # created; build() will use that committed lockfile with --locked.
        run("cargo", "test", "--manifest-path", str(cargo))
    run("git", "config", "user.name", "workers-ci[bot]")
    run("git", "config", "user.email", "workers-ci[bot]@users.noreply.github.com")
    run("git", "add", str(root))
    if subprocess.run(["git", "diff", "--cached", "--quiet"], check=False).returncode != 0:
        run("git", "commit", "-m", f"chore({args.worker}): prepare v{args.candidate_version}")
    prepared_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    run("git", "push", "origin", f"HEAD:{args.branch}")

    metadata = {
        "schema_version": 2,
        "worker": args.worker,
        "source_sha": args.source_sha,
        "prepared_sha": prepared_sha,
        "candidate_version": args.candidate_version,
        "release_intent_id": args.intent_id,
        "release_attempt_id": args.attempt_id,
        "deploy": manifest.deploy,
        "bin": manifest.bin or manifest.name,
        "targets": targets,
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "identity.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(metadata, sort_keys=True))
    return 0


def build(args: argparse.Namespace) -> int:
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != args.prepared_sha:
        raise SystemExit(f"prepared SHA mismatch: checkout={actual}, requested={args.prepared_sha}")
    root, manifest = manifest_for(args.worker)
    targets = target_policy(manifest, args.targets)
    target = args.target
    args.out.mkdir(parents=True, exist_ok=True)
    if manifest.deploy == "binary":
        if target not in targets:
            raise SystemExit(f"{args.worker}: target {target} is not in the release target policy")
        cargo = root / manifest.manifest
        command = ["cargo", "build", "--release", "--target", target]
        if (root / "Cargo.lock").exists():
            command.append("--locked")
        command += ["--manifest-path", str(cargo)]
        run(*command)
        binary = manifest.bin or manifest.name
        binary_path = root / "target" / target / "release" / binary
        if not binary_path.exists():
            raise SystemExit(f"{args.worker}: built binary not found at {binary_path}")
        artifacts = package_binary(binary_path, binary, target, args.out)
    elif target != "none":
        raise SystemExit(f"{args.worker}: non-binary builds use target=none")
    elif manifest.deploy == "image":
        image = f"release-attempt/{args.worker}:{os.environ.get('RELEASE_ATTEMPT_ID', 'local')}"
        run("docker", "build", "--tag", image, str(root))
        image_tar = args.out / f"{args.worker}-image.tar"
        run("docker", "save", "--output", str(image_tar), image)
        artifacts = [image_tar]
    elif manifest.deploy == "bundle":
        package = root / "package.json"
        if package.exists():
            lock = Path("pnpm-lock.yaml")
            if lock.exists():
                run("pnpm", "install", "--frozen-lockfile", "--ignore-workspace", cwd=root)
            else:
                run("pnpm", "install", "--ignore-workspace", cwd=root)
            package_json = json.loads(package.read_text(encoding="utf-8"))
            if "build:bundle" in (package_json.get("scripts") or {}):
                run("pnpm", "run", "build:bundle", cwd=root)
        artifacts = [package_source(args.worker, args.out)]
    else:
        raise SystemExit(f"{args.worker}: unsupported deploy mode {manifest.deploy}")

    metadata = {
        "worker": args.worker,
        "prepared_sha": args.prepared_sha,
        "target": target,
        "artifacts": [{"name": artifact.name, "sha256": sha256(artifact), "path": artifact.name} for artifact in artifacts],
    }
    (args.out / "target.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


def assemble(args: argparse.Namespace) -> int:
    identity = json.loads(args.identity.read_text(encoding="utf-8"))
    args.out.mkdir(parents=True, exist_ok=True)
    artifacts: list[dict[str, str]] = []
    for source in sorted(args.artifacts_dir.rglob("*")):
        if not source.is_file() or source.name in {"identity.json", "target.json", "prepared.json"}:
            continue
        destination = args.out / source.name
        if source.resolve() != destination.resolve():
            shutil.copy2(source, destination)
        artifacts.append({"name": destination.name, "sha256": sha256(destination), "path": destination.name})
    if identity["deploy"] == "binary":
        prefix = identity["bin"]
        expected = {f"{prefix}-{target}.tar.gz" for target in identity["targets"]}
        actual = {entry["name"] for entry in artifacts if entry["name"].endswith(".tar.gz")}
        if expected != actual:
            raise SystemExit(
                f"assembled binary artifacts do not match targets: expected={sorted(expected)} actual={sorted(actual)}"
            )
    metadata = {**identity, "artifacts": artifacts}
    (args.out / "prepared.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(metadata, sort_keys=True))
    return 0


def print_targets(args: argparse.Namespace) -> int:
    _, manifest = manifest_for(args.worker)
    raw = args.targets if args.targets is not None else manifest.raw.get("targets")
    print(json.dumps(release_targets.matrix_targets(raw, deploy=manifest.deploy), separators=(",", ":")))
    return 0


def make_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="command", required=True)

    prepare_parser = sub.add_parser("prepare")
    prepare_parser.set_defaults(handler=prepare)
    prepare_parser.add_argument("--worker", required=True)
    prepare_parser.add_argument("--source-sha", required=True)
    prepare_parser.add_argument("--candidate-version", required=True)
    prepare_parser.add_argument("--intent-id", required=True)
    prepare_parser.add_argument("--attempt-id", required=True)
    prepare_parser.add_argument("--branch", required=True)
    prepare_parser.add_argument("--targets")
    prepare_parser.add_argument("--out", type=Path, default=Path("release-identity"))

    build_parser = sub.add_parser("build")
    build_parser.set_defaults(handler=build)
    build_parser.add_argument("--worker", required=True)
    build_parser.add_argument("--prepared-sha", required=True)
    build_parser.add_argument("--target", required=True)
    build_parser.add_argument("--targets")
    build_parser.add_argument("--out", type=Path, required=True)

    assemble_parser = sub.add_parser("assemble")
    assemble_parser.set_defaults(handler=assemble)
    assemble_parser.add_argument("--identity", type=Path, required=True)
    assemble_parser.add_argument("--artifacts-dir", type=Path, required=True)
    assemble_parser.add_argument("--out", type=Path, required=True)

    targets_parser = sub.add_parser("print-targets")
    targets_parser.set_defaults(handler=print_targets)
    targets_parser.add_argument("--worker", required=True)
    targets_parser.add_argument("--targets")
    return p


if __name__ == "__main__":
    arguments = make_parser().parse_args()
    raise SystemExit(arguments.handler(arguments))
