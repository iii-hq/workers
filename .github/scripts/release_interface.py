#!/usr/bin/env python3
"""Stage, record, and compare release interfaces from prepared artifacts only."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
from pathlib import Path, PurePosixPath
from typing import Any


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _read_object(path: Path, label: str) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit(f"{label} must be an object")
    return value


def _safe_member(name: str) -> PurePosixPath:
    member = PurePosixPath(name)
    if member.is_absolute() or not member.parts or any(part in {"", ".", ".."} for part in member.parts):
        raise SystemExit(f"unsafe prepared archive path: {name!r}")
    return member


def extract_regular_tar(archive_path: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=False)
    with tarfile.open(archive_path, mode="r:gz") as archive:
        members = archive.getmembers()
        if not members:
            raise SystemExit("prepared archive is empty")
        for member in members:
            relative = _safe_member(member.name)
            if not member.isfile():
                raise SystemExit(f"prepared archive member is not a regular file: {member.name}")
            target = destination.joinpath(*relative.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            source = archive.extractfile(member)
            if source is None:
                raise SystemExit(f"cannot read prepared archive member: {member.name}")
            with target.open("wb") as handle:
                shutil.copyfileobj(source, handle)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)


def _verify_prepared(descriptor: dict[str, Any], prepared: dict[str, Any], root: Path) -> list[dict[str, Any]]:
    expected = {
        "contract": "prepared-artifacts",
        "worker": descriptor["worker"],
        "source_sha": descriptor["source_sha"],
        "descriptor_sha256": descriptor["descriptor_sha256"],
    }
    if any(prepared.get(key) != value for key, value in expected.items()):
        raise SystemExit("prepared artifact identity differs from release descriptor")
    artifacts = prepared.get("artifacts")
    if not isinstance(artifacts, list):
        raise SystemExit("prepared artifacts must be an array")
    for artifact in artifacts:
        if not isinstance(artifact, dict):
            raise SystemExit("prepared artifact entry must be an object")
        path = root / str(artifact.get("name", ""))
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"prepared artifact is not a regular file: {path}")
        if path.stat().st_size != artifact.get("size") or sha256(path) != artifact.get("sha256"):
            raise SystemExit(f"prepared artifact bytes differ from inventory: {path}")
    return artifacts


def _single_artifact(artifacts: list[dict[str, Any]], role: str, *, unit: str | None = None) -> dict[str, Any]:
    matches = [
        artifact for artifact in artifacts
        if artifact.get("role") == role and (unit is None or artifact.get("unit") == unit)
    ]
    if len(matches) != 1:
        suffix = f" for unit {unit}" if unit else ""
        raise SystemExit(f"prepared release requires exactly one {role} artifact{suffix}")
    return matches[0]


def stage(args: argparse.Namespace) -> int:
    descriptor = _read_object(args.descriptor, "release descriptor")
    prepared = _read_object(args.prepared, "prepared artifact inventory")
    package = descriptor.get("package")
    if not isinstance(package, dict):
        raise SystemExit("release descriptor package must be an object")
    validation = (package.get("validation") or {}).get("interface")
    if validation not in {"required", "skipped"}:
        raise SystemExit("descriptor validation.interface must be required or skipped")
    artifact = package.get("artifact")
    runtime = package.get("runtime")
    if not isinstance(artifact, dict) or not isinstance(runtime, dict):
        raise SystemExit("descriptor artifact and runtime must be objects")
    artifacts = _verify_prepared(descriptor, prepared, args.prepared.parent)
    kind = artifact.get("kind")
    result: dict[str, Any] = {
        "contract": "release-interface-stage",
        "worker": descriptor["worker"],
        "descriptor_sha256": descriptor["descriptor_sha256"],
        "validation": validation,
        "kind": kind,
        "cwd": None,
        "prepare": [],
        "command": [],
        "oci_archive": None,
        "runtime_name": None,
        "runtime_version": None,
        "package_manager_name": None,
        "package_manager_version": None,
        "environment": {},
    }
    environment = runtime.get("environment", {})
    if not isinstance(environment, dict) or any(
        not isinstance(key, str) or not key or not isinstance(value, str)
        for key, value in environment.items()
    ):
        raise SystemExit("runtime.environment must map non-empty strings to strings")
    result["environment"] = dict(sorted(environment.items()))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    if validation == "required":
        if kind == "rust-binary":
            target = "x86_64-unknown-linux-gnu"
            units = [
                unit for unit in descriptor.get("build_units", [])
                if isinstance(unit, dict) and unit.get("kind") == kind and unit.get("target") == target
            ]
            if len(units) != 1:
                raise SystemExit(f"required interface needs exactly one {target} build unit")
            selected = _single_artifact(artifacts, "binary", unit=str(units[0]["id"]))
            stage_dir = args.out.parent / "runtime"
            extract_regular_tar(args.prepared.parent / str(selected["name"]), stage_dir)
            command = runtime.get("exec")
            if not isinstance(command, list) or not command or not all(isinstance(part, str) for part in command):
                raise SystemExit("non-OCI runtime.exec must be a non-empty argv array")
            executable = stage_dir / command[0]
            if not executable.is_file() or executable.is_symlink():
                raise SystemExit(f"prepared runtime executable is missing: {executable}")
            executable.chmod(0o755)
            result.update(cwd=str(stage_dir), command=[str(executable), *command[1:]])
        elif kind in {"javascript-bundle", "python-bundle"}:
            selected = _single_artifact(artifacts, "bundle")
            stage_dir = args.out.parent / "runtime"
            extract_regular_tar(args.prepared.parent / str(selected["name"]), stage_dir)
            command = runtime.get("exec")
            prepare = runtime.get("prepare", [])
            if not isinstance(command, list) or not command or not all(isinstance(part, str) for part in command):
                raise SystemExit("bundle runtime.exec must be a non-empty argv array")
            if not isinstance(prepare, list) or any(
                not isinstance(value, list) or not value or not all(isinstance(part, str) for part in value)
                for value in prepare
            ):
                raise SystemExit("bundle runtime.prepare must contain argv arrays")
            entry = command[1] if len(command) > 1 and command[1].startswith("./") else None
            if entry is not None and not (stage_dir / entry.removeprefix("./")).is_file():
                raise SystemExit(f"prepared bundle entrypoint is missing: {entry}")
            tool_runtime = artifact.get("runtime")
            manager = artifact.get("package_manager")
            if not isinstance(tool_runtime, dict) or not isinstance(manager, dict):
                raise SystemExit("bundle build runtime and package manager must be explicit")
            result.update(
                cwd=str(stage_dir), prepare=prepare, command=command,
                runtime_name=tool_runtime.get("name"), runtime_version=tool_runtime.get("version"),
                package_manager_name=manager.get("name"), package_manager_version=manager.get("version"),
            )
        elif kind == "oci-image":
            selected = _single_artifact(artifacts, "oci-image")
            archive = args.out.parent / "image.oci.tar"
            with gzip.open(args.prepared.parent / str(selected["name"]), "rb") as source:
                with archive.open("wb") as destination:
                    shutil.copyfileobj(source, destination)
            if not tarfile.is_tarfile(archive):
                raise SystemExit("prepared OCI artifact is not an OCI tar archive")
            result["oci_archive"] = str(archive)
        else:
            raise SystemExit(f"unsupported release interface artifact kind {kind!r}")
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as handle:
            for key in (
                "validation", "kind", "runtime_name", "runtime_version",
                "package_manager_name", "package_manager_version",
            ):
                handle.write(f"{key}={result[key] or ''}\n")
    return 0


def _canonical_interface(value: object) -> dict[str, list[dict[str, Any]]]:
    if not isinstance(value, dict) or set(value) != {"functions", "triggers"}:
        raise SystemExit("worker interface must contain only functions and triggers")
    result: dict[str, list[dict[str, Any]]] = {}
    for key in ("functions", "triggers"):
        rows = value[key]
        if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
            raise SystemExit(f"worker interface {key} must be an array of objects")
        result[key] = sorted(rows, key=lambda row: (str(row.get("name", "")), json.dumps(row, sort_keys=True)))
    return result


def snapshot(args: argparse.Namespace) -> int:
    descriptor = _read_object(args.descriptor, "release descriptor")
    validation = ((descriptor.get("package") or {}).get("validation") or {}).get("interface")
    if validation == "required":
        if args.interface is None:
            raise SystemExit("required interface snapshot needs collected interface bytes")
        interface: dict[str, Any] | None = _canonical_interface(
            json.loads(args.interface.read_text(encoding="utf-8"))
        )
        if not interface["functions"] and not interface["triggers"]:
            raise SystemExit("required interface snapshot must contain a function or trigger")
    elif validation == "skipped":
        if args.interface is not None:
            raise SystemExit("skipped interface snapshot must not accept collected interface")
        interface = None
    else:
        raise SystemExit("descriptor validation.interface must be required or skipped")
    result = {
        "contract": "release-interface",
        "worker": descriptor["worker"],
        "source_sha": descriptor["source_sha"],
        "descriptor_sha256": descriptor["descriptor_sha256"],
        "validation": validation,
        "interface": interface,
    }
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


def build_evidence(args: argparse.Namespace) -> int:
    descriptor = _read_object(args.descriptor, "release descriptor")
    prepared = _read_object(args.prepared, "prepared artifact inventory")
    interface = _read_object(args.interface, "release interface")
    artifacts = _verify_prepared(descriptor, prepared, args.prepared.parent)
    identity = {
        "worker": descriptor["worker"],
        "source_sha": descriptor["source_sha"],
        "descriptor_sha256": descriptor["descriptor_sha256"],
    }
    if any(interface.get(key) != value for key, value in identity.items()):
        raise SystemExit("release interface identity differs from release descriptor")

    evidence = [dict(artifact) for artifact in artifacts]
    for path, role in (
        (args.descriptor, "descriptor"),
        (args.prepared, "prepared-inventory"),
        (args.interface, "interface"),
    ):
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"release evidence is not a regular file: {path}")
        evidence.append({
            "name": path.name,
            "role": role,
            "sha256": sha256(path),
            "size": path.stat().st_size,
        })
    evidence.sort(key=lambda artifact: (str(artifact["name"]), str(artifact["role"])))
    document = {"contract": "release-evidence", **identity, "artifacts": evidence}
    args.out.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


def verify_evidence(args: argparse.Namespace) -> int:
    descriptor = _read_object(args.descriptor, "release descriptor")
    evidence = _read_object(args.evidence, "release evidence inventory")
    identity = {
        "contract": "release-evidence",
        "worker": descriptor["worker"],
        "source_sha": descriptor["source_sha"],
        "descriptor_sha256": descriptor["descriptor_sha256"],
    }
    if any(evidence.get(key) != value for key, value in identity.items()):
        raise SystemExit("release evidence identity differs from release descriptor")
    artifacts = evidence.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        raise SystemExit("release evidence artifacts must be a non-empty array")
    names: set[str] = set()
    for artifact in artifacts:
        if not isinstance(artifact, dict):
            raise SystemExit("release evidence artifact must be an object")
        name = str(artifact.get("name", ""))
        safe_name = _safe_member(name)
        if len(safe_name.parts) != 1:
            raise SystemExit(f"release evidence artifact name must be a basename: {name}")
        if name in names:
            raise SystemExit(f"duplicate release evidence artifact: {name}")
        names.add(name)
        path = args.evidence.parent / name
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"release evidence file is missing: {path}")
        if path.stat().st_size != artifact.get("size") or sha256(path) != artifact.get("sha256"):
            raise SystemExit(f"release evidence bytes differ from inventory: {path}")
    required = {args.descriptor.name, "prepared-artifacts.json", "release-interface.json"}
    if not required.issubset(names):
        raise SystemExit(f"release evidence is incomplete: missing {sorted(required - names)}")
    prepared = _read_object(args.evidence.parent / "prepared-artifacts.json", "prepared artifact inventory")
    interface = _read_object(args.evidence.parent / "release-interface.json", "release interface")
    for document, label in ((prepared, "prepared artifact"), (interface, "release interface")):
        if any(document.get(key) != value for key, value in identity.items() if key != "contract"):
            raise SystemExit(f"{label} identity differs from release descriptor")
    expected_validation = ((descriptor.get("package") or {}).get("validation") or {}).get("interface")
    if interface.get("contract") != "release-interface" or interface.get("validation") != expected_validation:
        raise SystemExit("release interface contract differs from release descriptor")
    if prepared.get("contract") != "prepared-artifacts":
        raise SystemExit("prepared artifact inventory contract differs")
    return 0


def compare(args: argparse.Namespace) -> int:
    descriptor = _read_object(args.descriptor, "release descriptor")
    expected = _read_object(args.expected, "prepared release interface")
    identity = {
        "contract": "release-interface",
        "worker": descriptor["worker"],
        "source_sha": descriptor["source_sha"],
        "descriptor_sha256": descriptor["descriptor_sha256"],
        "validation": ((descriptor.get("package") or {}).get("validation") or {}).get("interface"),
    }
    if any(expected.get(key) != value for key, value in identity.items()):
        raise SystemExit("prepared interface identity differs from release descriptor")
    if identity["validation"] == "skipped":
        if expected.get("interface") is not None or args.actual is not None:
            raise SystemExit("skipped interface evidence must remain explicit and empty")
        return 0
    if args.actual is None:
        raise SystemExit("required candidate smoke must collect the published interface")
    actual = _canonical_interface(json.loads(args.actual.read_text(encoding="utf-8")))
    if expected.get("interface") != actual:
        raise SystemExit("published candidate interface differs from prepared artifact snapshot")
    return 0


def run_adapter(args: argparse.Namespace) -> int:
    metadata = _read_object(args.metadata, "release interface stage")
    if metadata.get("contract") != "release-interface-stage" or metadata.get("validation") != "required":
        raise SystemExit("only a required prepared interface stage can be executed")
    if metadata.get("kind") == "oci-image":
        raise SystemExit("OCI interface stages must run through the container adapter")
    cwd = Path(str(metadata["cwd"]))
    environment = os.environ.copy()
    environment.update(metadata.get("environment") or {})
    environment["III_URL"] = args.engine_url
    for command in metadata["prepare"]:
        subprocess.run(command, cwd=cwd, env=environment, check=True)
    with args.log.open("wb") as log:
        process = subprocess.Popen(
            metadata["command"], cwd=cwd, env=environment,
            stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT,
        )
        return process.wait()


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    stage_parser = sub.add_parser("stage")
    stage_parser.set_defaults(handler=stage)
    stage_parser.add_argument("--descriptor", type=Path, required=True)
    stage_parser.add_argument("--prepared", type=Path, required=True)
    stage_parser.add_argument("--out", type=Path, required=True)
    stage_parser.add_argument("--github-output", type=Path)
    snapshot_parser = sub.add_parser("snapshot")
    snapshot_parser.set_defaults(handler=snapshot)
    snapshot_parser.add_argument("--descriptor", type=Path, required=True)
    snapshot_parser.add_argument("--interface", type=Path)
    snapshot_parser.add_argument("--out", type=Path, required=True)
    compare_parser = sub.add_parser("compare")
    compare_parser.set_defaults(handler=compare)
    compare_parser.add_argument("--descriptor", type=Path, required=True)
    compare_parser.add_argument("--expected", type=Path, required=True)
    compare_parser.add_argument("--actual", type=Path)
    evidence_parser = sub.add_parser("build-evidence")
    evidence_parser.set_defaults(handler=build_evidence)
    evidence_parser.add_argument("--descriptor", type=Path, required=True)
    evidence_parser.add_argument("--prepared", type=Path, required=True)
    evidence_parser.add_argument("--interface", type=Path, required=True)
    evidence_parser.add_argument("--out", type=Path, required=True)
    verify_parser = sub.add_parser("verify-evidence")
    verify_parser.set_defaults(handler=verify_evidence)
    verify_parser.add_argument("--descriptor", type=Path, required=True)
    verify_parser.add_argument("--evidence", type=Path, required=True)
    run_parser = sub.add_parser("run-adapter")
    run_parser.set_defaults(handler=run_adapter)
    run_parser.add_argument("--metadata", type=Path, required=True)
    run_parser.add_argument("--engine-url", required=True)
    run_parser.add_argument("--log", type=Path, required=True)
    return parser


if __name__ == "__main__":
    arguments = make_parser().parse_args()
    raise SystemExit(arguments.handler(arguments))
