#!/usr/bin/env python3
"""Build and package one immutable Release Train attempt.

The script deliberately has no GitHub Release, Registry, or final tag logic.
Those effects belong to the publish workflows, which consume this attempt's
artifact and prepared commit identity.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tarfile
from pathlib import Path

import _lib


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


def package_source(worker: str, out: Path) -> Path:
    archive = out / f"{worker}.tar.gz"
    with tarfile.open(archive, "w:gz") as handle:
        handle.add(worker, arcname=".", filter=lambda item: None if ".git" in item.name.split("/") else item)
    return archive


def build_attempt(worker: str, out: Path) -> list[Path]:
    root, manifest = manifest_for(worker)
    out.mkdir(parents=True, exist_ok=True)
    artifacts: list[Path] = []
    if manifest.deploy == "binary":
        cargo = root / "Cargo.toml"
        if not cargo.exists():
            raise SystemExit(f"{worker}: deploy=binary requires Cargo.toml")
        locked = (root / "Cargo.lock").exists()
        test_command = ["cargo", "test"]
        build_command = ["cargo", "build", "--release"]
        if locked:
            test_command.append("--locked")
            build_command.append("--locked")
        test_command += ["--manifest-path", str(cargo)]
        build_command += ["--manifest-path", str(cargo)]
        run(*test_command)
        run(*build_command)
        binary = manifest.bin or manifest.name
        # Workers are standalone crates (no root workspace): cargo places the
        # artifacts under the worker's own target directory.
        binary_path = root / "target" / "release" / binary
        if not binary_path.exists():
            raise SystemExit(f"{worker}: built binary not found at {binary_path}")
        archive = out / f"{binary}-x86_64-unknown-linux-gnu.tar.gz"
        with tarfile.open(archive, "w:gz") as handle:
            handle.add(binary_path, arcname=binary)
        artifacts.append(archive)
    elif manifest.deploy == "image":
        image = f"release-attempt/{worker}:{os.environ.get('RELEASE_ATTEMPT_ID', 'local')}"
        run("docker", "build", "--tag", image, str(root))
        image_tar = out / f"{worker}-image.tar"
        run("docker", "save", "--output", str(image_tar), image)
        artifacts.append(image_tar)
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
        archive = package_source(worker, out)
        artifacts.append(archive)
    else:
        raise SystemExit(f"{worker}: unsupported deploy mode {manifest.deploy}")
    return artifacts


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--candidate-version", required=True)
    parser.add_argument("--intent-id", required=True)
    parser.add_argument("--attempt-id", required=True)
    parser.add_argument("--branch", required=True)
    parser.add_argument("--out", type=Path, default=Path("release-prepared"))
    args = parser.parse_args()

    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != args.source_sha:
        raise SystemExit(f"source SHA mismatch: checkout={actual}, requested={args.source_sha}")
    if not _lib.RELEASE_VERSION_RE.fullmatch(args.candidate_version):
        raise SystemExit(f"invalid candidate version: {args.candidate_version}")

    root, manifest = manifest_for(args.worker)
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
    run("git", "config", "user.name", "workers-ci[bot]")
    run("git", "config", "user.email", "workers-ci[bot]@users.noreply.github.com")
    run("git", "add", str(root))
    if subprocess.run(["git", "diff", "--cached", "--quiet"], check=False).returncode != 0:
        run("git", "commit", "-m", f"chore({args.worker}): prepare v{args.candidate_version}")
    prepared_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    run("git", "push", "origin", f"HEAD:{args.branch}")

    artifacts = build_attempt(args.worker, args.out)
    metadata = {
        "schema_version": 1,
        "worker": args.worker,
        "source_sha": args.source_sha,
        "prepared_sha": prepared_sha,
        "candidate_version": args.candidate_version,
        "release_intent_id": args.intent_id,
        "release_attempt_id": args.attempt_id,
        "deploy": manifest.deploy,
        "artifacts": [
            {"name": artifact.name, "sha256": sha256(artifact), "path": artifact.name}
            for artifact in artifacts
        ],
    }
    (args.out / "prepared.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(metadata, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
