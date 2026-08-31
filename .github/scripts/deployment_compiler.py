#!/usr/bin/env python3
"""Compile the private Workers release catalog into immutable descriptors.

The compiler is deliberately repository-owned. It joins release-only build
metadata with the public ``iii.worker.yaml`` contract and the selected package
manifest exactly once. Release phases consume only its JSON output.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


SHA_RE = re.compile(r"^[0-9a-f]{40}$")
WORKER_RE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
ARTIFACT_KINDS = {"rust-binary", "javascript-bundle", "python-bundle", "oci-image"}
DEPLOY_KIND = {
    "binary": "rust-binary",
    "bundle": {"javascript-bundle", "python-bundle"},
    "image": "oci-image",
}
DEPLOYMENT_SECTIONS = {"source", "artifact", "publish"}
SECRET_KEYS = {
    "api_key",
    "access_key",
    "secret",
    "secret_key",
    "password",
    "token",
    "private_key",
    "credential",
    "credentials",
}


def fail(message: str) -> None:
    raise ValueError(message)


def canonical_value(value: Any) -> Any:
    """Normalize JSON numbers to the representation used by JSON.stringify.

    YAML and TOML preserve ``2.0`` as a float, while JavaScript has one Number
    type and serializes that value as ``2``. Descriptors are verified by the
    TypeScript coordinator, so integral floats (including negative zero) must
    not create a digest that cannot be reproduced after JSON parsing.
    """
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, list):
        return [canonical_value(item) for item in value]
    if isinstance(value, dict):
        return {key: canonical_value(item) for key, item in value.items()}
    return value


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(
        canonical_value(value),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


def json_sha256(value: Any) -> str:
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_yaml(path: Path) -> Any:
    try:
        import yaml
    except ImportError as error:  # pragma: no cover - CI installs PyYAML explicitly
        raise ValueError("PyYAML is required by the deployment compiler") from error
    if not path.is_file():
        fail(f"{path}: file does not exist")
    value = yaml.safe_load(path.read_text(encoding="utf-8"))
    return {} if value is None else value


def safe_relative(root: Path, value: Any, field: str, *, directory: bool = False) -> Path:
    if not isinstance(value, str) or not value or Path(value).is_absolute() or ".." in Path(value).parts:
        fail(f"{field} must be a safe relative path")
    resolved = (root / value).resolve()
    try:
        resolved.relative_to(root.resolve())
    except ValueError as error:
        raise ValueError(f"{field} escapes the repository") from error
    if directory and not resolved.is_dir():
        fail(f"{field} does not identify a directory: {value}")
    return resolved


def read_version(path: Path) -> str:
    if path.name == "package.json":
        value = json.loads(path.read_text(encoding="utf-8"))
        version = value.get("version") if isinstance(value, dict) else None
    elif path.name == "Cargo.toml":
        value = tomllib.loads(path.read_text(encoding="utf-8"))
        version = (value.get("package") or {}).get("version")
    elif path.name == "pyproject.toml":
        value = tomllib.loads(path.read_text(encoding="utf-8"))
        version = (value.get("project") or {}).get("version")
    else:
        fail(f"unsupported package manifest {path.name!r}")
        raise AssertionError("unreachable")
    if not isinstance(version, str) or not version:
        fail(f"{path}: package version is missing")
    return version


def compiler_digest(script: Path, schema: Path) -> str:
    digest = hashlib.sha256()
    digest.update(b"iii-workers-deployment-compiler\0")
    digest.update(script.read_bytes())
    digest.update(b"\0deployment-descriptor-schema\0")
    digest.update(schema.read_bytes())
    return digest.hexdigest()


def validate_public_defaults(value: Any, field: str) -> None:
    if isinstance(value, dict):
        for key, nested in value.items():
            if not isinstance(key, str):
                fail(f"{field}: configuration keys must be strings")
            normalized = key.lower().replace("-", "_")
            if key.startswith("III_"):
                fail(f"{field}.{key}: III_* defaults cannot be released")
            secret_key = normalized in SECRET_KEYS or normalized.endswith(
                ("_api_key", "_token", "_secret", "_password", "_private_key", "_credentials")
            )
            if secret_key and nested not in (None, ""):
                fail(f"{field}.{key}: secret defaults cannot be released")
            validate_public_defaults(nested, f"{field}.{key}")
    elif isinstance(value, list):
        for index, nested in enumerate(value):
            validate_public_defaults(nested, f"{field}[{index}]")
    elif isinstance(value, str):
        references = re.findall(r"\$\{([A-Za-z_][A-Za-z0-9_]*)", value)
        if any(name.startswith("III_") for name in references):
            fail(f"{field}: III_* defaults cannot be released")
        if any(
            name.lower() in SECRET_KEYS
            or name.lower().endswith(("_token", "_secret", "_password", "_api_key"))
            for name in references
        ):
            fail(f"{field}: secret defaults cannot be released")


def normalize_dependencies(value: Any, field: str) -> list[dict[str, str]]:
    if value is None:
        return []
    if not isinstance(value, dict):
        fail(f"{field} must be a mapping")
    rows: list[dict[str, str]] = []
    for name, version in value.items():
        if not isinstance(name, str) or not name or not isinstance(version, str):
            fail(f"{field} must map worker names to semver strings")
        rows.append({"name": name, "version": version})
    return sorted(rows, key=lambda row: (row["name"], row["version"]))


def normalize_tags(value: Any, field: str) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list) or any(not isinstance(tag, str) or not tag.strip() for tag in value):
        fail(f"{field} must be an array of non-empty strings")
    return sorted(dict.fromkeys(tag.strip().lower() for tag in value))


def normalize_config(worker_dir: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    if "config" in manifest and manifest["config"] is not None:
        config = manifest["config"]
    else:
        config_path = worker_dir / "config.yaml"
        config = read_yaml(config_path) if config_path.is_file() else {}
    if not isinstance(config, dict):
        fail(f"{worker_dir}/iii.worker.yaml: public config must be a mapping")
    validate_public_defaults(config, f"{worker_dir.name}.config")
    return config


def normalize_runtime(worker: str, manifest: dict[str, Any], kind: str) -> dict[str, Any]:
    env = manifest.get("env") or {}
    if not isinstance(env, dict) or any(not isinstance(key, str) or not isinstance(value, str) for key, value in env.items()):
        fail(f"{worker}/iii.worker.yaml: env must map strings to strings")
    validate_public_defaults(env, f"{worker}.env")
    resources = manifest.get("resources") or {}
    runtime = manifest.get("runtime") or {}
    scripts = manifest.get("scripts") or {}
    for name, value in (("resources", resources), ("runtime", runtime), ("scripts", scripts)):
        if not isinstance(value, dict):
            fail(f"{worker}/iii.worker.yaml: {name} must be a mapping")
    result: dict[str, Any] = {
        "environment": dict(sorted(env.items())),
        "resources": resources,
    }
    if kind == "rust-binary":
        result["exec"] = [str(manifest.get("bin") or worker)]
    elif kind in {"javascript-bundle", "python-bundle"}:
        start = scripts.get("start")
        if not isinstance(start, str) or not start.strip():
            fail(f"{worker}/iii.worker.yaml: bundle scripts.start is required")
        result.update(
            base_image=runtime.get("base_image"),
            install=scripts.get("install") or "",
            start=start,
        )
    elif kind == "oci-image":
        result.update(image_runtime=runtime, scripts=scripts)
    return result


def validate_artifact(
    root: Path,
    worker: str,
    worker_dir: Path,
    artifact: Any,
    manifest: dict[str, Any],
) -> dict[str, Any]:
    if not isinstance(artifact, dict):
        fail(f"workers.{worker}.artifact must be a mapping")
    artifact = json.loads(json.dumps(artifact))
    kind = artifact.get("kind")
    if kind not in ARTIFACT_KINDS:
        fail(f"workers.{worker}.artifact.kind is unsupported")
    allowed_fields = {
        "rust-binary": {
            "kind", "toolchain", "binary", "candidate_targets", "stable_targets",
            "windows_exception", "frontends",
        },
        "javascript-bundle": {
            "kind", "workspace_root", "runtime", "package_manager", "lockfile",
            "install_command", "build_command", "include",
        },
        "python-bundle": {
            "kind", "workspace_root", "runtime", "package_manager", "lockfile",
            "install_command", "build_command", "include",
        },
        "oci-image": {"kind", "context", "dockerfile", "platforms", "platform_exception"},
    }[kind]
    unknown = set(artifact) - allowed_fields
    if unknown:
        fail(f"workers.{worker}.artifact has unknown fields: {sorted(unknown)}")
    deploy = manifest.get("deploy")
    expected = DEPLOY_KIND.get(deploy)
    matches = kind in expected if isinstance(expected, set) else kind == expected
    if not matches:
        fail(f"{worker}: iii.worker.yaml deploy={deploy!r} conflicts with artifact.kind={kind!r}")
    if kind == "rust-binary":
        binary = artifact.get("binary")
        if not isinstance(binary, str) or not binary:
            fail(f"workers.{worker}.artifact.binary is required")
        if binary != str(manifest.get("bin") or worker):
            fail(f"{worker}: public bin and private artifact.binary differ")
        for field in ("candidate_targets", "stable_targets"):
            targets = artifact.get(field)
            if not isinstance(targets, list) or not targets or any(
                not isinstance(target, str) or not target for target in targets
            ):
                fail(f"workers.{worker}.artifact.{field} must be a non-empty string array")
            if len(set(targets)) != len(targets):
                fail(f"workers.{worker}.artifact.{field} contains duplicate targets")
        candidate_targets = artifact["candidate_targets"]
        stable_targets = artifact["stable_targets"]
        # Candidates are the nightly rc surface: Windows bytes only ever exist
        # for finalized stable versions, built as the stable-delta profile.
        windows_targets = [target for target in candidate_targets if "windows" in target]
        if windows_targets:
            fail(f"workers.{worker}.artifact.candidate_targets cannot include Windows targets: {windows_targets}")
        missing = [target for target in candidate_targets if target not in stable_targets]
        if missing:
            fail(f"workers.{worker}: candidate_targets must be a subset of stable_targets, missing {missing}")
        # Windows absence must be a justified decision, never a silent omission.
        windows_declared = "x86_64-pc-windows-msvc" in stable_targets
        windows_exception = artifact.get("windows_exception")
        if windows_declared and windows_exception is not None:
            fail(f"workers.{worker}: declares both Windows support and a windows_exception")
        if not windows_declared and (not isinstance(windows_exception, str) or not windows_exception.strip()):
            fail(
                f"workers.{worker}: must either include x86_64-pc-windows-msvc in stable_targets "
                "or justify its absence with windows_exception"
            )
        public_targets = manifest.get("targets")
        if public_targets is not None and public_targets != stable_targets:
            fail(f"{worker}: public targets and private artifact.stable_targets differ")
        toolchain = artifact.get("toolchain")
        if not isinstance(toolchain, dict) or set(toolchain) != {"name", "version"}:
            fail(f"workers.{worker}.artifact.toolchain must contain only name and version")
        if toolchain.get("name") != "rust" or not isinstance(toolchain.get("version"), str):
            fail(f"workers.{worker}.artifact.toolchain must pin a Rust version")
        frontends = artifact.get("frontends", [])
        if not isinstance(frontends, list):
            fail(f"workers.{worker}.artifact.frontends must be an array")
        frontend_fields = {
            "workspace_root", "source_path", "runtime", "package_manager", "lockfile",
            "install_command", "build_command", "outputs",
        }
        for index, frontend in enumerate(frontends):
            if not isinstance(frontend, dict) or set(frontend) != frontend_fields:
                fail(f"workers.{worker}.artifact.frontends[{index}] differs from the compiler contract")
            workspace = safe_relative(
                root, frontend["workspace_root"],
                f"workers.{worker}.artifact.frontends[{index}].workspace_root",
                directory=True,
            )
            safe_relative(
                root, frontend["source_path"],
                f"workers.{worker}.artifact.frontends[{index}].source_path",
                directory=True,
            )
            lockfile = safe_relative(
                workspace, frontend["lockfile"],
                f"workers.{worker}.artifact.frontends[{index}].lockfile",
            )
            if not lockfile.is_file():
                fail(f"workers.{worker}.artifact.frontends[{index}].lockfile does not exist")
            for name in ("install_command", "build_command"):
                command = frontend[name]
                if not isinstance(command, list) or not command or any(
                    not isinstance(part, str) or not part for part in command
                ):
                    fail(f"workers.{worker}.artifact.frontends[{index}].{name} must be a non-empty argv")
            outputs = frontend["outputs"]
            if not isinstance(outputs, list) or not outputs or any(
                not isinstance(output, str) or not output for output in outputs
            ):
                fail(f"workers.{worker}.artifact.frontends[{index}].outputs must be non-empty")
    elif kind in {"javascript-bundle", "python-bundle"}:
        for name in ("runtime", "package_manager"):
            nested = artifact.get(name)
            if not isinstance(nested, dict) or set(nested) != {"name", "version"}:
                fail(f"workers.{worker}.artifact.{name} must contain only name and version")
        for name in ("install_command", "build_command"):
            command = artifact.get(name)
            if not isinstance(command, list) or not command or any(
                not isinstance(part, str) or not part for part in command
            ):
                fail(f"workers.{worker}.artifact.{name} must be a non-empty argv")
        workspace = safe_relative(
            root, artifact.get("workspace_root"),
            f"workers.{worker}.artifact.workspace_root", directory=True,
        )
        lockfile = safe_relative(
            workspace, artifact.get("lockfile"),
            f"workers.{worker}.artifact.lockfile",
        )
        if not lockfile.is_file():
            fail(f"workers.{worker}.artifact.lockfile does not exist")
        includes = artifact.get("include")
        if not isinstance(includes, list) or not includes or any(not isinstance(path, str) for path in includes):
            fail(f"workers.{worker}.artifact.include must be an explicit non-empty array")
        forbidden = {"node_modules", "tests", "test", "docs", "doc", ".cache", "__pycache__"}
        for index, value in enumerate(includes):
            path = Path(value)
            if path.is_absolute() or ".." in path.parts or any(part in forbidden for part in path.parts):
                fail(f"workers.{worker}.artifact.include[{index}] is forbidden: {value}")
            candidate = worker_dir / path
            if not candidate.is_file() and not value.startswith("dist/"):
                fail(f"workers.{worker}.artifact.include[{index}] is not a source file or prepared output")
    else:
        context = safe_relative(
            worker_dir, artifact.get("context"),
            f"workers.{worker}.artifact.context", directory=True,
        )
        dockerfile = safe_relative(
            worker_dir, artifact.get("dockerfile"),
            f"workers.{worker}.artifact.dockerfile",
        )
        if not dockerfile.is_file() or not dockerfile.is_relative_to(context):
            fail(f"workers.{worker}.artifact.dockerfile must be a file inside the OCI context")
        platforms = artifact.get("platforms")
        required = {"linux/amd64", "linux/arm64"}
        if not isinstance(platforms, list) or any(not isinstance(platform, str) for platform in platforms):
            fail(f"workers.{worker}.artifact.platforms must be an array")
        if not required.issubset(platforms) and artifact.get("platform_exception") is not True:
            fail(f"workers.{worker}.artifact.platforms must contain linux/amd64 and linux/arm64")
    return artifact


def build_units(worker: str, artifact: dict[str, Any]) -> list[dict[str, Any]]:
    kind = str(artifact["kind"])
    if kind == "rust-binary":
        # Build units describe the candidate profile: release candidates only
        # ever build these. Stable-delta units (Windows) are derived at
        # finalization from stable_targets - candidate_targets.
        return [
            {"id": f"{worker}-{target}", "kind": kind, "target": target}
            for target in artifact["candidate_targets"]
        ]
    if kind == "oci-image":
        return [
            {"id": f"{worker}-{platform.replace('/', '-')}", "kind": kind, "platform": platform}
            for platform in artifact["platforms"]
        ]
    return [{"id": f"{worker}-bundle", "kind": kind}]


def compile_worker(root: Path, worker: str, value: Any, source_sha: str, compiler_sha: str) -> dict[str, Any]:
    if not WORKER_RE.fullmatch(worker) or not isinstance(value, dict):
        fail(f"invalid release worker entry {worker!r}")
    if set(value) != DEPLOYMENT_SECTIONS:
        fail(
            f"workers.{worker} must contain exactly {sorted(DEPLOYMENT_SECTIONS)}; "
            f"got {sorted(value) if isinstance(value, dict) else type(value).__name__}"
        )
    source = value["source"]
    if not isinstance(source, dict) or set(source) != {"path", "package_manifest"}:
        fail(f"workers.{worker}.source must contain only path and package_manifest")
    worker_dir = safe_relative(root, source["path"], f"workers.{worker}.source.path", directory=True)
    manifest_name = source["package_manifest"]
    package_manifest = safe_relative(worker_dir, manifest_name, f"workers.{worker}.source.package_manifest")
    if not package_manifest.is_file():
        fail(f"{package_manifest}: package manifest does not exist")
    public_path = worker_dir / "iii.worker.yaml"
    manifest = read_yaml(public_path)
    if not isinstance(manifest, dict):
        fail(f"{public_path}: expected a mapping")
    if manifest.get("name") != worker:
        fail(f"{public_path}: name must be {worker!r}")
    if manifest.get("manifest") != manifest_name:
        fail(f"{public_path}: manifest must match the private package_manifest")
    publish = value["publish"]
    if not isinstance(publish, bool):
        fail(f"workers.{worker}.publish must be a boolean")
    if publish:
        for field in ("description", "license"):
            if not isinstance(manifest.get(field), str) or not manifest[field].strip():
                fail(f"{public_path}: {field} is required for Registry publication")
    artifact = validate_artifact(root, worker, worker_dir, value["artifact"], manifest)
    package_manifest_version = read_version(package_manifest)
    projection = {
        "worker_name": worker,
        "type": manifest["deploy"],
        "description": str(manifest.get("description") or ""),
        "license": str(manifest.get("license") or ""),
        "tags": normalize_tags(manifest.get("tags"), f"{worker}.tags"),
        "dependencies": normalize_dependencies(manifest.get("dependencies"), f"{worker}.dependencies"),
        "config": normalize_config(worker_dir, manifest),
        "experimental": bool(manifest.get("experimental", False)),
        "readme": (worker_dir / "README.md").read_text(encoding="utf-8")
        if (worker_dir / "README.md").is_file()
        else "",
    }
    descriptor: dict[str, Any] = {
        "contract": "deployment-descriptor",
        "worker": worker,
        "package_manifest_version": package_manifest_version,
        "source_sha": source_sha,
        "deployment_spec_sha256": json_sha256(value),
        "public_manifest_sha256": file_sha256(public_path),
        "registry_projection_sha256": json_sha256(projection),
        "compiler_digest": compiler_sha,
        "source": source,
        "artifact": artifact,
        "runtime": normalize_runtime(worker, manifest, str(artifact["kind"])),
        "publish": publish,
        "build_units": build_units(worker, artifact),
        "registry_projection": projection,
    }
    descriptor["descriptor_sha256"] = json_sha256(descriptor)
    return descriptor


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")


def compile_index(args: argparse.Namespace) -> int:
    root = args.root.resolve()
    deployment_spec = (root / args.deployment_spec).resolve()
    schema = args.schema.resolve()
    if not SHA_RE.fullmatch(args.source_sha):
        fail("source_sha must be a full lowercase commit SHA")
    if not SHA_RE.fullmatch(args.compiler_commit):
        fail("compiler_commit must be a full lowercase commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.compiler_repository):
        fail("compiler_repository must be an owner/name pair")
    document = read_yaml(deployment_spec)
    if not isinstance(document, dict) or set(document) != {"workers"} or not isinstance(document["workers"], dict):
        fail(f"{deployment_spec}: expected exactly one non-empty workers mapping")
    if not document["workers"]:
        fail(f"{deployment_spec}: workers must not be empty")
    digest = compiler_digest(Path(__file__).resolve(), schema)
    output = args.output_dir.resolve()
    if output.exists() and any(output.iterdir()):
        fail(f"{output}: output directory must be absent or empty")
    descriptors: dict[str, dict[str, Any]] = {}
    for worker, value in sorted(document["workers"].items()):
        if isinstance(value, dict) and value.get("publish") is False:
            continue
        descriptors[worker] = compile_worker(root, worker, value, args.source_sha, digest)
    entries: dict[str, dict[str, Any]] = {}
    for worker, descriptor in descriptors.items():
        relative = f"descriptors/{worker}.json"
        write_json(output / relative, descriptor)
        entries[worker] = {
            "path": relative,
            "digest": descriptor["descriptor_sha256"],
            "package_manifest_version": descriptor["package_manifest_version"],
            "publish": descriptor["publish"],
            "deployment_spec_sha256": descriptor["deployment_spec_sha256"],
            "public_manifest_sha256": descriptor["public_manifest_sha256"],
            "registry_projection_sha256": descriptor["registry_projection_sha256"],
        }
    index = {
        "contract": "deployment-descriptor-index",
        "source_sha": args.source_sha,
        "compiler": {
            "repository": args.compiler_repository,
            "commit": args.compiler_commit,
            "digest": digest,
        },
        "workers": entries,
    }
    write_json(output / "deployment-descriptor-index.json", index)
    print(json.dumps({"workers": len(entries), "publishable": sum(row["publish"] for row in entries.values()), "compiler_digest": digest}, sort_keys=True))
    return 0


def show_digest(args: argparse.Namespace) -> int:
    digest = compiler_digest(Path(__file__).resolve(), args.schema.resolve())
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"digest={digest}\n")
    print(digest)
    return 0


def parser() -> argparse.ArgumentParser:
    root = Path(__file__).resolve().parents[2]
    schema = Path(__file__).resolve().parents[1] / "contracts" / "deployment-descriptor.schema.json"
    command = argparse.ArgumentParser(description=__doc__)
    subparsers = command.add_subparsers(dest="command", required=True)
    compile_command = subparsers.add_parser("compile-index")
    compile_command.add_argument("--root", type=Path, default=root)
    compile_command.add_argument("--deployment-spec", type=Path, default=Path(".deploy/workers.yaml"))
    compile_command.add_argument("--source-sha", required=True)
    compile_command.add_argument("--compiler-repository", required=True)
    compile_command.add_argument("--compiler-commit", required=True)
    compile_command.add_argument("--schema", type=Path, default=schema)
    compile_command.add_argument("--output-dir", type=Path, required=True)
    compile_command.set_defaults(handler=compile_index)
    digest_command = subparsers.add_parser("digest")
    digest_command.add_argument("--schema", type=Path, default=schema)
    digest_command.add_argument("--github-output", type=Path)
    digest_command.set_defaults(handler=show_digest)
    return command


def main() -> int:
    try:
        args = parser().parse_args()
        return args.handler(args)
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"deployment compiler: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
