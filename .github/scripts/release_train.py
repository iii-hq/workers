#!/usr/bin/env python3
"""Prepare and build immutable releases from the Workers-owned descriptor."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import shutil
import stat
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path

import _lib
import release_targets
import validate_oci_dockerfile

def run(*command: str, cwd: Path | None = None) -> None:
    print("+", " ".join(command))
    subprocess.run(command, cwd=cwd, check=True)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_descriptor(descriptor: object) -> dict[str, object]:
    if not isinstance(descriptor, dict):
        raise SystemExit("release descriptor must be an object")
    required = {
        "contract", "worker", "version", "source_sha", "release_spec_sha256",
        "public_manifest_sha256", "registry_projection_sha256", "compiler_digest",
        "descriptor_sha256", "source", "artifact", "runtime", "validation",
        "publish", "build_units", "registry_projection",
    }
    if set(descriptor) != required:
        raise SystemExit(
            "release descriptor fields differ from compiler contract: "
            f"missing={sorted(required - set(descriptor))} unknown={sorted(set(descriptor) - required)}"
        )
    if descriptor["contract"] != "release-descriptor":
        raise SystemExit("release descriptor contract mismatch")
    digest_subject = {key: value for key, value in descriptor.items() if key != "descriptor_sha256"}
    digest = hashlib.sha256(
        json.dumps(digest_subject, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    ).hexdigest()
    if descriptor["descriptor_sha256"] != digest:
        raise SystemExit("release descriptor digest is invalid")
    for field in ("source", "artifact", "runtime", "validation", "registry_projection"):
        if not isinstance(descriptor[field], dict):
            raise SystemExit(f"release descriptor {field} must be an object")
    if not isinstance(descriptor["build_units"], list) or not descriptor["build_units"]:
        raise SystemExit("release descriptor build_units must be non-empty")
    return descriptor


def select_descriptor(args: argparse.Namespace) -> int:
    index_path = args.index_dir / "release-descriptor-index.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    required_index = {"contract", "source_sha", "compiler", "workers"}
    if not isinstance(index, dict) or set(index) != required_index:
        raise SystemExit("descriptor index fields differ from compiler contract")
    if index["contract"] != "release-descriptor-index":
        raise SystemExit("descriptor index contract mismatch")
    if index["source_sha"] != args.source_sha:
        raise SystemExit("descriptor index source SHA mismatch")
    compiler = index["compiler"]
    if not isinstance(compiler, dict) or compiler.get("digest") != args.compiler_digest:
        raise SystemExit("descriptor index compiler digest mismatch")
    workers = index["workers"]
    if not isinstance(workers, dict) or args.worker not in workers:
        raise SystemExit("worker is absent from descriptor index")
    entry = workers[args.worker]
    required_entry = {
        "path", "digest", "version", "publish", "release_spec_sha256",
        "public_manifest_sha256", "registry_projection_sha256",
    }
    if not isinstance(entry, dict) or set(entry) != required_entry:
        raise SystemExit("descriptor index worker entry differs from compiler contract")
    expected_path = f"descriptors/{args.worker}.json"
    if entry["path"] != expected_path:
        raise SystemExit("descriptor index worker path mismatch")
    if entry["digest"] != args.descriptor_sha256:
        raise SystemExit("descriptor index digest mismatch")
    if entry["version"] != args.candidate_version:
        raise SystemExit("descriptor index version mismatch")
    if entry["publish"] is not True:
        raise SystemExit("descriptor index worker is not publishable")

    descriptor_path = args.index_dir / expected_path
    descriptor_bytes = descriptor_path.read_bytes()
    descriptor = verify_descriptor(json.loads(descriptor_bytes))
    expected_identity = {
        "worker": args.worker,
        "source_sha": args.source_sha,
        "version": args.candidate_version,
        "descriptor_sha256": args.descriptor_sha256,
    }
    for field, expected in expected_identity.items():
        if descriptor[field] != expected:
            raise SystemExit(f"selected descriptor {field} mismatch")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(descriptor_bytes)
    print(json.dumps(expected_identity, sort_keys=True))
    return 0


def _safe_relative(value: object, field: str, *, allow_dot: bool = False) -> Path:
    if not isinstance(value, str) or not value or (value == "." and not allow_dot):
        raise SystemExit(f"{field} must be a non-empty relative path")
    path = Path(value)
    if path.is_absolute() or any(part == ".." for part in path.parts):
        raise SystemExit(f"{field} must remain within the repository")
    return path


def _argv(value: object, field: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not value or any(not isinstance(part, str) or not part for part in value):
        raise SystemExit(f"{field} must be a non-empty argv")
    return tuple(value)


def frontend_specs(descriptor: dict[str, object]) -> list[dict[str, object]]:
    artifact = descriptor["artifact"]
    if not isinstance(artifact, dict):
        raise SystemExit("descriptor artifact must be an object")
    raw = artifact.get("frontends", [])
    if not isinstance(raw, list):
        raise SystemExit("artifact.frontends must be an array")
    required = {
        "workspace_root", "source_path", "runtime", "package_manager", "lockfile",
        "install_command", "build_command", "outputs",
    }
    normalized: list[dict[str, object]] = []
    identities: set[str] = set()
    for index, value in enumerate(raw):
        if not isinstance(value, dict) or set(value) != required:
            raise SystemExit(f"artifact.frontends[{index}] differs from compiler contract")
        runtime = value["runtime"]
        package_manager = value["package_manager"]
        if not isinstance(runtime, dict) or set(runtime) != {"name", "version"}:
            raise SystemExit(f"artifact.frontends[{index}].runtime differs from compiler contract")
        if not isinstance(package_manager, dict) or set(package_manager) != {"name", "version"}:
            raise SystemExit(f"artifact.frontends[{index}].package_manager differs from compiler contract")
        if runtime["name"] != "node" or not isinstance(runtime["version"], str) or not runtime["version"]:
            raise SystemExit("release frontends require an explicit Node runtime")
        if package_manager["name"] != "pnpm" or not isinstance(package_manager["version"], str) or not package_manager["version"]:
            raise SystemExit("release frontends require an explicit pnpm runtime")
        workspace_root = _safe_relative(value["workspace_root"], f"artifact.frontends[{index}].workspace_root", allow_dot=True)
        source_path = _safe_relative(value["source_path"], f"artifact.frontends[{index}].source_path")
        lockfile = _safe_relative(value["lockfile"], f"artifact.frontends[{index}].lockfile")
        install_command = _argv(value["install_command"], f"artifact.frontends[{index}].install_command")
        build_command = _argv(value["build_command"], f"artifact.frontends[{index}].build_command")
        outputs = value["outputs"]
        if not isinstance(outputs, list) or not outputs:
            raise SystemExit(f"artifact.frontends[{index}].outputs must be non-empty")
        output_paths = [_safe_relative(output, f"artifact.frontends[{index}].outputs") for output in outputs]
        normalized_value = {
            **value,
            "workspace_root": workspace_root,
            "source_path": source_path,
            "lockfile": lockfile,
            "install_command": install_command,
            "build_command": build_command,
            "outputs": output_paths,
        }
        identity = json.dumps(value, sort_keys=True, separators=(",", ":"))
        if identity in identities:
            raise SystemExit("duplicate frontend build declaration")
        identities.add(identity)
        normalized.append(normalized_value)
    return normalized


def frontend_metadata(args: argparse.Namespace) -> int:
    descriptor = verify_descriptor(json.loads(args.descriptor.read_bytes()))
    specs = frontend_specs(descriptor)
    metadata = {"count": len(specs), "runtime_version": "", "package_manager_version": "", "lock_sha256": ""}
    if specs:
        runtimes = {str(spec["runtime"]["version"]) for spec in specs}  # type: ignore[index]
        managers = {str(spec["package_manager"]["version"]) for spec in specs}  # type: ignore[index]
        if len(runtimes) != 1 or len(managers) != 1:
            raise SystemExit("a release must use one explicit frontend runtime and package-manager version")
        lock_digest = hashlib.sha256()
        lockfiles: set[Path] = set()
        for spec in specs:
            lock = Path(spec["workspace_root"]) / Path(spec["lockfile"])
            if lock in lockfiles:
                continue
            lockfiles.add(lock)
            try:
                mode = lock.lstat().st_mode
            except FileNotFoundError:
                raise SystemExit(f"frontend lockfile is missing: {lock}") from None
            if not stat.S_ISREG(mode):
                raise SystemExit(f"frontend lockfile is not a regular file: {lock}")
            lock_digest.update(lock.as_posix().encode())
            lock_digest.update(b"\0")
            lock_digest.update(lock.read_bytes())
            lock_digest.update(b"\0")
        metadata.update({
            "runtime_version": next(iter(runtimes)),
            "package_manager_version": next(iter(managers)),
            "lock_sha256": lock_digest.hexdigest(),
        })
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as handle:
            for key, value in metadata.items():
                handle.write(f"{key}={value}\n")
    print(json.dumps(metadata, sort_keys=True))
    return 0


def build_frontends(args: argparse.Namespace) -> int:
    descriptor = verify_descriptor(json.loads(args.descriptor.read_bytes()))
    specs = frontend_specs(descriptor)
    args.out.mkdir(parents=True, exist_ok=False)
    if not specs:
        (args.out / ".empty").touch()
        return 0

    installed: set[tuple[str, tuple[str, ...], str, str]] = set()
    destinations: set[Path] = set()
    repository = Path.cwd().resolve()
    for spec in specs:
        workspace = (repository / Path(spec["workspace_root"])).resolve()
        source = (repository / Path(spec["source_path"])).resolve()
        if not workspace.is_dir() or not workspace.is_relative_to(repository):
            raise SystemExit(f"frontend workspace escapes repository: {workspace}")
        if not source.is_dir() or not source.is_relative_to(repository):
            raise SystemExit(f"frontend source escapes repository: {source}")
        install_identity = (
            workspace.as_posix(),
            spec["install_command"],  # type: ignore[arg-type]
            str(spec["runtime"]["version"]),  # type: ignore[index]
            str(spec["package_manager"]["version"]),  # type: ignore[index]
        )
        if install_identity not in installed:
            run(*spec["install_command"], cwd=workspace)  # type: ignore[arg-type]
            installed.add(install_identity)
        run(*spec["build_command"], cwd=source)  # type: ignore[arg-type]

        for relative_output in spec["outputs"]:  # type: ignore[union-attr]
            output = source / relative_output
            try:
                resolved = output.resolve(strict=True)
            except FileNotFoundError:
                raise SystemExit(f"declared frontend output is missing after build: {output}") from None
            if not resolved.is_relative_to(source):
                raise SystemExit(f"declared frontend output escapes source: {output}")
            for candidate in [output, *output.rglob("*")] if output.is_dir() else [output]:
                if candidate.is_symlink():
                    raise SystemExit(f"declared frontend output contains a symlink: {candidate}")
            destination = args.out / Path(spec["source_path"]) / relative_output
            if destination in destinations:
                raise SystemExit(f"duplicate frontend output destination: {destination}")
            destinations.add(destination)
            destination.parent.mkdir(parents=True, exist_ok=True)
            if output.is_dir():
                shutil.copytree(output, destination)
            elif output.is_file():
                shutil.copy2(output, destination)
            else:
                raise SystemExit(f"declared frontend output is not a regular file or directory: {output}")
    return 0


def build_metadata(args: argparse.Namespace) -> int:
    descriptor = verify_descriptor(json.loads(args.descriptor.read_bytes()))
    units = [unit for unit in descriptor["build_units"] if isinstance(unit, dict) and unit.get("id") == args.unit]
    if len(units) != 1:
        raise SystemExit(f"build unit {args.unit!r} is not declared exactly once")
    artifact = descriptor["artifact"]
    source = descriptor["source"]
    if not isinstance(artifact, dict) or not isinstance(source, dict):
        raise SystemExit("descriptor source/artifact must be objects")
    kind = str(artifact.get("kind"))
    if units[0].get("kind") != kind:
        raise SystemExit("build unit kind differs from descriptor artifact")
    metadata = {
        "kind": kind,
        "source_path": str(source.get("path", "")),
        "workspace_root": "",
        "runtime_name": "",
        "runtime_version": "",
        "package_manager_name": "",
        "package_manager_version": "",
        "lock_sha256": "",
        "toolchain_name": "",
        "toolchain_version": "",
    }
    if kind == "rust-binary":
        toolchain = artifact.get("toolchain")
        if not isinstance(toolchain, dict) or set(toolchain) != {"name", "version"}:
            raise SystemExit("rust artifact toolchain differs from compiler contract")
        if toolchain["name"] != "rust" or not isinstance(toolchain["version"], str) or not toolchain["version"]:
            raise SystemExit("rust build requires an explicit Rust toolchain")
        metadata.update(toolchain_name="rust", toolchain_version=toolchain["version"])
    elif kind in {"javascript-bundle", "python-bundle"}:
        required = {
            "kind", "workspace_root", "runtime", "package_manager", "lockfile",
            "install_command", "build_command", "include",
        }
        if set(artifact) != required:
            raise SystemExit("bundle artifact differs from compiler contract")
        workspace = _safe_relative(artifact["workspace_root"], "artifact.workspace_root", allow_dot=True)
        lockfile = _safe_relative(artifact["lockfile"], "artifact.lockfile")
        runtime = artifact["runtime"]
        manager = artifact["package_manager"]
        if not isinstance(runtime, dict) or set(runtime) != {"name", "version"}:
            raise SystemExit("bundle runtime differs from compiler contract")
        if not isinstance(manager, dict) or set(manager) != {"name", "version"}:
            raise SystemExit("bundle package_manager differs from compiler contract")
        expected_runtime = "node" if kind == "javascript-bundle" else "python"
        allowed_managers = {"pnpm", "npm"} if kind == "javascript-bundle" else {"uv"}
        if runtime["name"] != expected_runtime or not isinstance(runtime["version"], str) or not runtime["version"]:
            raise SystemExit(f"{kind} requires an explicit {expected_runtime} runtime")
        if manager["name"] not in allowed_managers or not isinstance(manager["version"], str) or not manager["version"]:
            raise SystemExit(f"unsupported package manager for {kind}")
        _argv(artifact["install_command"], "artifact.install_command")
        lock = workspace / lockfile
        try:
            mode = lock.lstat().st_mode
        except FileNotFoundError:
            raise SystemExit(f"bundle lockfile is missing: {lock}") from None
        if not stat.S_ISREG(mode):
            raise SystemExit(f"bundle lockfile is not a regular file: {lock}")
        metadata.update(
            workspace_root=workspace.as_posix(),
            runtime_name=runtime["name"],
            runtime_version=runtime["version"],
            package_manager_name=manager["name"],
            package_manager_version=manager["version"],
            lock_sha256=sha256(lock),
        )
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as handle:
            for key, value in metadata.items():
                handle.write(f"{key}={value}\n")
    print(json.dumps(metadata, sort_keys=True))
    return 0


def normalized_tar(entries: list[tuple[Path, str]], destination: Path) -> None:
    seen: set[str] = set()
    for source, arcname in entries:
        try:
            mode = source.lstat().st_mode
        except FileNotFoundError:
            raise SystemExit(f"declared artifact include is not a file: {source}") from None
        if not stat.S_ISREG(mode):
            raise SystemExit(f"declared artifact include is not a regular file: {source}")
        if arcname in seen:
            raise SystemExit(f"duplicate artifact include path: {arcname}")
        seen.add(arcname)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for source, arcname in sorted(entries, key=lambda entry: entry[1]):
                    info = archive.gettarinfo(str(source), arcname=arcname)
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    info.mode = 0o755 if source.stat().st_mode & 0o111 else 0o644
                    with source.open("rb") as handle:
                        archive.addfile(info, handle)


def normalized_zip(entries: list[tuple[Path, str]], destination: Path) -> None:
    seen: set[str] = set()
    for source, arcname in entries:
        try:
            mode = source.lstat().st_mode
        except FileNotFoundError:
            raise SystemExit(f"declared artifact include is not a file: {source}") from None
        if not stat.S_ISREG(mode):
            raise SystemExit(f"declared artifact include is not a regular file: {source}")
        if arcname in seen:
            raise SystemExit(f"duplicate artifact include path: {arcname}")
        seen.add(arcname)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for source, arcname in sorted(entries, key=lambda entry: entry[1]):
            info = zipfile.ZipInfo(arcname, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | 0o755) << 16
            archive.writestr(info, source.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def checksum_sidecar(artifact: Path) -> Path:
    suffix = ".sha256"
    if artifact.name.endswith(".tar.gz"):
        name = artifact.name[:-len(".tar.gz")] + suffix
    elif artifact.name.endswith(".zip"):
        name = artifact.name[:-len(".zip")] + suffix
    else:
        name = artifact.name + suffix
    sidecar = artifact.parent / name
    sidecar.write_text(f"{sha256(artifact)}  {artifact.name}\n", encoding="utf-8")
    return sidecar


def build_oci_layout(*, context: Path, dockerfile: Path, platforms: list[object], destination: Path) -> str:
    run(
        "docker", "buildx", "build",
        "--provenance=false",
        "--build-arg", "SOURCE_DATE_EPOCH=0",
        "--platform", ",".join(str(value) for value in platforms),
        "--file", str(dockerfile),
        "--output", f"type=oci,tar=false,dest={destination},rewrite-timestamp=true",
        str(context),
    )
    index = destination / "index.json"
    if not index.is_file():
        raise SystemExit("BuildKit did not produce an OCI index.json")
    return f"sha256:{sha256(index)}"


def prepare(args: argparse.Namespace) -> int:
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != args.source_sha:
        raise SystemExit(f"source SHA mismatch: checkout={actual}, requested={args.source_sha}")
    _lib.parse_release_version(args.candidate_version)
    stable = _lib.parse_release_version(args.stable_version)
    if stable.maturity != "stable":
        raise SystemExit("stable version must not contain a prerelease suffix")
    if args.candidate_version.split("-", 1)[0] != args.stable_version:
        raise SystemExit("candidate and stable versions must share the same core")

    descriptor = verify_descriptor(json.loads(args.descriptor.read_bytes()))
    expected = {
        "worker": args.worker,
        "source_sha": args.source_sha,
        "version": args.candidate_version,
        "descriptor_sha256": args.descriptor_sha256,
    }
    if any(descriptor[field] != value for field, value in expected.items()):
        raise SystemExit("selected descriptor identity differs from release intent")
    print(json.dumps(descriptor, sort_keys=True))
    return 0


def matrix(args: argparse.Namespace) -> int:
    descriptor = verify_descriptor(json.loads(args.descriptor.read_text(encoding="utf-8")))
    include: list[dict[str, str]] = []
    for unit in descriptor["build_units"]:
        if not isinstance(unit, dict):
            raise SystemExit("build unit must be an object")
        kind = str(unit.get("kind"))
        target = str(unit.get("target") or unit.get("platform") or "none")
        if kind == "rust-binary":
            runner = release_targets.TARGET_LARGER_RUNNERS.get(target)
            if runner is None:
                raise SystemExit(f"no runner configured for target {target}")
        else:
            runner = "workers-release-linux-8core"
        include.append({"unit": str(unit["id"]), "kind": kind, "target": target, "runner": runner})
    print(json.dumps({"include": include}, separators=(",", ":")))
    return 0


def build(args: argparse.Namespace) -> int:
    descriptor = verify_descriptor(json.loads(args.descriptor.read_text(encoding="utf-8")))
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != descriptor["source_sha"]:
        raise SystemExit(f"prepared SHA mismatch: checkout={actual}, descriptor={descriptor['source_sha']}")
    units = [unit for unit in descriptor["build_units"] if isinstance(unit, dict) and unit.get("id") == args.unit]
    if len(units) != 1:
        raise SystemExit(f"build unit {args.unit!r} is not declared exactly once")
    unit = units[0]
    source = descriptor["source"]
    artifact = descriptor["artifact"]
    assert isinstance(source, dict) and isinstance(artifact, dict)
    source_dir = Path(str(source["path"]))
    args.out.mkdir(parents=True, exist_ok=True)
    kind = str(artifact["kind"])

    if kind == "rust-binary":
        target = str(unit.get("target") or "")
        manifest = source_dir / str(source["package_manifest"])
        command = ["cargo", "build", "--release", "--target", target, "--manifest-path", str(manifest)]
        if (source_dir / "Cargo.lock").is_file():
            command.append("--locked")
        run(*command)
        binary = str(artifact["binary"])
        is_windows = target.endswith("-pc-windows-msvc")
        binary_filename = f"{binary}.exe" if is_windows else binary
        binary_path = source_dir / "target" / target / "release" / binary_filename
        if not binary_path.is_file():
            raise SystemExit(f"built binary not found at {binary_path}")
        if is_windows:
            archive = args.out / f"{binary}-{target}.zip"
            normalized_zip([(binary_path, binary_filename)], archive)
        else:
            archive = args.out / f"{binary}-{target}.tar.gz"
            normalized_tar([(binary_path, binary)], archive)
        role = "binary"
    elif kind in {"javascript-bundle", "python-bundle"}:
        workspace_root = Path(str(artifact["workspace_root"]))
        install_command = artifact.get("install_command")
        if not isinstance(install_command, list) or not install_command:
            raise SystemExit("bundle install_command must be non-empty")
        run(*(str(part) for part in install_command), cwd=workspace_root)
        command = artifact.get("build_command")
        if not isinstance(command, list) or not command:
            raise SystemExit("bundle build_command must be non-empty")
        run(*(str(part) for part in command), cwd=source_dir)
        includes = artifact.get("include")
        if not isinstance(includes, list) or not includes:
            raise SystemExit("bundle include must be non-empty")
        entries = [(source_dir / str(relative), str(relative)) for relative in includes]
        archive = args.out / f"{descriptor['worker']}.tar.gz"
        normalized_tar(entries, archive)
        role = "bundle"
    elif kind == "oci-image":
        platform = unit.get("platform")
        if not isinstance(platform, str) or not platform:
            raise SystemExit("OCI build unit must identify exactly one platform")
        context = source_dir / str(artifact["context"])
        dockerfile = source_dir / str(artifact["dockerfile"])
        validate_oci_dockerfile.validate(dockerfile)
        with tempfile.TemporaryDirectory(prefix=f"{descriptor['worker']}-oci-") as temporary:
            layout = Path(temporary) / "layout-first"
            repeated_layout = Path(temporary) / "layout-second"
            first_digest = build_oci_layout(
                context=context, dockerfile=dockerfile, platforms=[platform], destination=layout,
            )
            second_digest = build_oci_layout(
                context=context, dockerfile=dockerfile, platforms=[platform], destination=repeated_layout,
            )
            if first_digest != second_digest:
                raise SystemExit(
                    "OCI build is not reproducible: "
                    f"first index={first_digest} second index={second_digest}"
                )
            platform_slug = platform.replace("/", "-")
            archive = args.out / f"{descriptor['worker']}-image-{platform_slug}.oci.tar.gz"
            files = [(path, path.relative_to(layout).as_posix()) for path in layout.rglob("*") if path.is_file()]
            normalized_tar(files, archive)
        role = "oci-platform"
    else:
        raise SystemExit(f"unsupported artifact kind {kind!r}")

    checksum = checksum_sidecar(archive)
    artifacts = [
        {"name": archive.name, "role": role, "sha256": sha256(archive), "size": archive.stat().st_size},
        {"name": checksum.name, "role": "checksum", "sha256": sha256(checksum), "size": checksum.stat().st_size},
    ]
    result = {"contract": "release-build-result", "worker": descriptor["worker"],
              "source_sha": descriptor["source_sha"], "descriptor_sha256": descriptor["descriptor_sha256"],
              "unit": args.unit, "artifacts": artifacts}
    (args.out / "build-result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


def assemble(args: argparse.Namespace) -> int:
    descriptor_bytes = args.descriptor.read_bytes()
    descriptor = verify_descriptor(json.loads(descriptor_bytes))
    args.out.mkdir(parents=True, exist_ok=True)
    entries: list[dict[str, object]] = []
    for result_path in sorted(args.artifacts_dir.rglob("build-result.json")):
        result = json.loads(result_path.read_text(encoding="utf-8"))
        if result.get("descriptor_sha256") != descriptor["descriptor_sha256"]:
            raise SystemExit(f"{result_path}: descriptor digest mismatch")
        unit = result.get("unit")
        for artifact in result.get("artifacts", []):
            source = result_path.parent / artifact["name"]
            if not source.is_file() or sha256(source) != artifact["sha256"] or source.stat().st_size != artifact["size"]:
                raise SystemExit(f"{source}: artifact bytes differ from build result")
            destination = args.out / source.name
            if destination.exists():
                raise SystemExit(f"duplicate artifact name {destination.name}")
            shutil.copy2(source, destination)
            entries.append({"unit": unit, **artifact})
    expected_units = {unit["id"] for unit in descriptor["build_units"]}
    actual_units = {entry["unit"] for entry in entries if entry["role"] != "checksum"}
    if expected_units != actual_units:
        raise SystemExit(f"assembled units differ: expected={sorted(expected_units)} actual={sorted(actual_units)}")
    (args.out / "release-descriptor.json").write_bytes(descriptor_bytes)
    prepared = {"contract": "prepared-artifacts", "worker": descriptor["worker"],
                "source_sha": descriptor["source_sha"], "descriptor_sha256": descriptor["descriptor_sha256"],
                "artifacts": sorted(entries, key=lambda entry: (str(entry["unit"]), str(entry["name"])))}
    (args.out / "prepared-artifacts.json").write_text(json.dumps(prepared, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(prepared, sort_keys=True))
    return 0


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    select_parser = sub.add_parser("select-descriptor")
    select_parser.set_defaults(handler=select_descriptor)
    select_parser.add_argument("--index-dir", type=Path, required=True)
    select_parser.add_argument("--worker", required=True)
    select_parser.add_argument("--source-sha", required=True)
    select_parser.add_argument("--compiler-digest", required=True)
    select_parser.add_argument("--candidate-version", required=True)
    select_parser.add_argument("--descriptor-sha256", required=True)
    select_parser.add_argument("--out", type=Path, required=True)
    frontend_metadata_parser = sub.add_parser("frontend-metadata")
    frontend_metadata_parser.set_defaults(handler=frontend_metadata)
    frontend_metadata_parser.add_argument("--descriptor", type=Path, required=True)
    frontend_metadata_parser.add_argument("--github-output", type=Path)
    build_frontends_parser = sub.add_parser("build-frontends")
    build_frontends_parser.set_defaults(handler=build_frontends)
    build_frontends_parser.add_argument("--descriptor", type=Path, required=True)
    build_frontends_parser.add_argument("--out", type=Path, required=True)
    build_metadata_parser = sub.add_parser("build-metadata")
    build_metadata_parser.set_defaults(handler=build_metadata)
    build_metadata_parser.add_argument("--descriptor", type=Path, required=True)
    build_metadata_parser.add_argument("--unit", required=True)
    build_metadata_parser.add_argument("--github-output", type=Path)
    prepare_parser = sub.add_parser("prepare")
    prepare_parser.set_defaults(handler=prepare)
    prepare_parser.add_argument("--worker", required=True)
    prepare_parser.add_argument("--source-sha", required=True)
    prepare_parser.add_argument("--candidate-version", required=True)
    prepare_parser.add_argument("--stable-version", required=True)
    prepare_parser.add_argument("--descriptor-sha256", required=True)
    prepare_parser.add_argument("--descriptor", type=Path, required=True)
    matrix_parser = sub.add_parser("matrix")
    matrix_parser.set_defaults(handler=matrix)
    matrix_parser.add_argument("--descriptor", type=Path, required=True)
    build_parser = sub.add_parser("build")
    build_parser.set_defaults(handler=build)
    build_parser.add_argument("--descriptor", type=Path, required=True)
    build_parser.add_argument("--unit", required=True)
    build_parser.add_argument("--out", type=Path, required=True)
    assemble_parser = sub.add_parser("assemble")
    assemble_parser.set_defaults(handler=assemble)
    assemble_parser.add_argument("--descriptor", type=Path, required=True)
    assemble_parser.add_argument("--artifacts-dir", type=Path, required=True)
    assemble_parser.add_argument("--out", type=Path, required=True)
    return parser


if __name__ == "__main__":
    arguments = make_parser().parse_args()
    raise SystemExit(arguments.handler(arguments))
