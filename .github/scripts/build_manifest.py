#!/usr/bin/env python3
"""Write the content-addressed manifest that hands a build to Release Control.

`build.yml` only builds and uploads immutable bytes. This manifest is its single
hand-off: it names every GitHub release asset under `build-<source_sha>` and the
published OCI index digest, and embeds the descriptor, the captured interface
and the evidence inventory so Release Control can assemble the Registry publish
payload later (see build_publish_payload.build_payload) without re-running
anything in a runner.
"""

from __future__ import annotations

import argparse
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import deployment_train

SCHEMA = "workers-build-manifest/1"
SOURCE_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
IMAGE_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
RUN_ID_RE = re.compile(r"^[1-9][0-9]*$")
BUNDLE_KINDS = {"javascript-bundle", "python-bundle"}


def release_tag(source_sha: str) -> str:
    if not SOURCE_SHA_RE.fullmatch(source_sha):
        raise SystemExit("source_sha must be exactly 40 lowercase hex characters")
    return f"build-{source_sha}"


def asset_name(worker: str, name: str) -> str:
    """One release per source SHA is shared by every worker, so each asset carries its worker."""
    if name == worker or name.startswith(f"{worker}-") or name.startswith(f"{worker}."):
        return name
    return f"{worker}-{name}"


def asset_url(repository: str, source_sha: str, asset: str) -> str:
    return f"https://github.com/{repository}/releases/download/{release_tag(source_sha)}/{asset}"


def _read_object(path: Path, label: str) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit(f"{label} must be a JSON object")
    return value


def _check_identity(document: dict[str, Any], expected: dict[str, Any], label: str) -> None:
    if any(document.get(key) != value for key, value in expected.items()):
        raise SystemExit(f"{label} identity differs from the prepared release")


def plan(args: argparse.Namespace) -> int:
    """Print `prepared name, release asset name, sha256, size` rows for the upload job."""
    prepared = _read_object(args.prepared, "prepared artifact inventory")
    for artifact in prepared["artifacts"]:
        name = str(artifact["name"])
        print("\t".join([name, asset_name(args.worker, name), str(artifact["sha256"]), str(artifact["size"])]))
    return 0


def _artifacts(
    descriptor: dict[str, Any], prepared: dict[str, Any], prepared_dir: Path,
    receipt: dict[str, Any], repository: str,
) -> list[dict[str, Any]]:
    targets = {str(unit["id"]): unit.get("target") or unit.get("platform") for unit in descriptor["build_units"]}
    uploaded: dict[str, dict[str, Any]] = {}
    for row in receipt.get("assets", []):
        if not isinstance(row, dict) or not isinstance(row.get("name"), str) or row["name"] in uploaded:
            raise SystemExit("upload receipt assets must be objects with unique prepared names")
        uploaded[row["name"]] = row
    worker = str(descriptor["worker"])
    result: list[dict[str, Any]] = []
    for artifact in prepared["artifacts"]:
        name = str(artifact["name"])
        path = prepared_dir / name
        if not path.is_file() or path.stat().st_size != artifact["size"] or deployment_train.sha256(path) != artifact["sha256"]:
            raise SystemExit(f"prepared artifact bytes differ from inventory: {name}")
        if artifact["unit"] not in targets:
            raise SystemExit(f"prepared artifact {name} names unknown build unit {artifact['unit']!r}")
        asset = asset_name(worker, name)
        row = uploaded.get(name)
        if row is None:
            raise SystemExit(f"upload receipt has no entry for prepared artifact {name}")
        if row.get("asset") != asset or row.get("sha256") != artifact["sha256"] or row.get("size") != artifact["size"]:
            raise SystemExit(f"upload receipt for {name} differs from the prepared bytes")
        result.append({
            "name": asset,
            "prepared_name": name,
            "unit": artifact["unit"],
            "role": artifact["role"],
            "target": targets[artifact["unit"]],
            "sha256": artifact["sha256"],
            "size": artifact["size"],
            "url": asset_url(repository, str(descriptor["source_sha"]), asset),
            "upload_state": row.get("state"),
        })
    return result


def _image(descriptor: dict[str, Any], receipt: dict[str, Any]) -> dict[str, str] | None:
    image = receipt.get("image")
    if descriptor["artifact"]["kind"] != "oci-image":
        if image is not None:
            raise SystemExit("only oci-image releases publish an OCI index")
        return None
    if not isinstance(image, dict):
        raise SystemExit("image release requires the published OCI index in the upload receipt")
    repository, tag, digest = image.get("repository"), image.get("tag"), image.get("digest")
    if not isinstance(repository, str) or not repository.endswith(f"/{descriptor['worker']}"):
        raise SystemExit("OCI repository must be the worker's own ghcr.io repository")
    if tag != release_tag(str(descriptor["source_sha"])):
        raise SystemExit("OCI index tag must be the content-addressed build tag")
    if not isinstance(digest, str) or not IMAGE_DIGEST_RE.fullmatch(digest):
        raise SystemExit("OCI index digest must be sha256:<64 hex>")
    return {"repository": repository, "tag": tag, "digest": digest}


def _registry_artifacts(kind: str, artifacts: list[dict[str, Any]], image: dict[str, str] | None) -> dict[str, Any]:
    """The `--artifacts-json` document build_publish_payload.py consumes, precomputed."""
    payload_artifacts = [artifact for artifact in artifacts if artifact["role"] != "checksum"]
    if kind == "rust-binary":
        binaries: dict[str, dict[str, str]] = {}
        for artifact in payload_artifacts:
            target = artifact["target"]
            if not isinstance(target, str) or target in binaries:
                raise SystemExit(f"binary release needs exactly one archive per target, got {target!r} twice or unset")
            binaries[target] = {"url": artifact["url"], "sha256": artifact["sha256"]}
        return {"kind": kind, "binaries": binaries}
    if kind in BUNDLE_KINDS:
        if len(payload_artifacts) != 1:
            raise SystemExit("bundle release requires exactly one prepared archive")
        return {"kind": kind, "archive_url": payload_artifacts[0]["url"], "sha256": payload_artifacts[0]["sha256"]}
    if kind == "oci-image":
        if image is None:
            raise SystemExit("image release requires a published OCI index digest")
        return {"kind": kind, "image_tag": f"{image['repository']}:{image['tag']}@{image['digest']}"}
    raise SystemExit(f"unsupported artifact kind {kind!r}")


def _interface(descriptor: dict[str, Any], snapshot: dict[str, Any], identity: dict[str, Any]) -> dict[str, Any]:
    policy = descriptor["interface_capture"]
    _check_identity(snapshot, {"contract": "deployment-interface", **identity, "interface_capture": policy}, "captured interface")
    captured = snapshot.get("interface")
    if policy == "required":
        if not isinstance(captured, dict) or set(captured) != {"functions", "triggers"}:
            raise SystemExit("captured interface must contain only functions and triggers")
        if not captured["functions"] and not captured["triggers"]:
            raise SystemExit("required interface capture must contain a function or trigger")
        return captured
    if captured is not None:
        raise SystemExit("skipped interface capture must remain null")
    return {"functions": [], "triggers": []}


def write(args: argparse.Namespace) -> int:
    tag = release_tag(args.source_sha)
    if not RUN_ID_RE.fullmatch(args.descriptor_run_id):
        raise SystemExit("descriptor_run_id must be a positive integer")
    if not re.fullmatch(r"[^/\s]+/[^/\s]+", args.repository):
        raise SystemExit("repository must be owner/name")
    prepared_dir: Path = args.prepared_dir
    descriptor = deployment_train.verify_descriptor(
        _read_object(prepared_dir / "deployment-descriptor.json", "deployment descriptor")
    )
    identity = {"worker": args.worker, "source_sha": args.source_sha}
    _check_identity(descriptor, identity, "deployment descriptor")
    identity["descriptor_sha256"] = descriptor["descriptor_sha256"]
    prepared = _read_object(prepared_dir / "prepared-artifacts.json", "prepared artifact inventory")
    _check_identity(prepared, {"contract": "prepared-artifacts", **identity}, "prepared artifact inventory")
    snapshot = _read_object(prepared_dir / "deployment-interface.json", "captured interface")
    evidence = _read_object(prepared_dir / "deployment-evidence.json", "release evidence")
    _check_identity(evidence, {"contract": "deployment-evidence", **identity}, "release evidence")
    receipt = _read_object(args.receipt, "upload receipt")
    _check_identity(receipt, {"contract": "build-upload-receipt", "worker": args.worker, "source_sha": args.source_sha}, "upload receipt")
    release = receipt.get("release")
    if not isinstance(release, dict) or release.get("tag") != tag:
        raise SystemExit(f"upload receipt release tag differs from {tag}")

    artifacts = _artifacts(descriptor, prepared, prepared_dir, receipt, args.repository)
    image = _image(descriptor, receipt)
    kind = str(descriptor["artifact"]["kind"])
    manifest = {
        "schema": SCHEMA,
        "worker": args.worker,
        "source_sha": args.source_sha,
        "descriptor_sha256": descriptor["descriptor_sha256"],
        "descriptor_run_id": int(args.descriptor_run_id),
        "correlation_id": args.correlation_id or None,
        "repository": args.repository,
        "run_id": args.run_id,
        "run_attempt": args.run_attempt,
        "built_at": args.built_at or datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "package_manifest_version": descriptor["package_manifest_version"],
        "artifact_kind": kind,
        "interface_capture": descriptor["interface_capture"],
        "release": {"tag": tag, "url": f"https://github.com/{args.repository}/releases/tag/{tag}"},
        "artifacts": artifacts,
        "image": image,
        "registry_artifacts": _registry_artifacts(kind, artifacts, image),
        "interface": _interface(descriptor, snapshot, identity),
        "evidence": evidence,
        "descriptor": descriptor,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = {key: value for key, value in manifest.items() if key not in {"descriptor", "evidence", "interface"}}
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    plan_parser = sub.add_parser("plan")
    plan_parser.set_defaults(handler=plan)
    plan_parser.add_argument("--worker", required=True)
    plan_parser.add_argument("--prepared", type=Path, required=True)
    write_parser = sub.add_parser("write")
    write_parser.set_defaults(handler=write)
    write_parser.add_argument("--worker", required=True)
    write_parser.add_argument("--source-sha", required=True)
    write_parser.add_argument("--descriptor-run-id", required=True)
    write_parser.add_argument("--correlation-id", default="")
    write_parser.add_argument("--repository", required=True)
    write_parser.add_argument("--run-id", type=int, required=True)
    write_parser.add_argument("--run-attempt", type=int, required=True)
    write_parser.add_argument("--prepared-dir", type=Path, required=True)
    write_parser.add_argument("--receipt", type=Path, required=True)
    write_parser.add_argument("--built-at", default="")
    write_parser.add_argument("--out", type=Path, required=True)
    return parser


if __name__ == "__main__":
    arguments = make_parser().parse_args()
    raise SystemExit(arguments.handler(arguments))
