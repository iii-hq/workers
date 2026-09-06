"""Tests for the build manifest that hands immutable bytes to Release Control."""
import argparse
import hashlib
import json
from pathlib import Path

import pytest

import build_manifest
import deployment_train
from build_publish_payload import build_payload

SOURCE_SHA = "a" * 40
TAG = f"build-{SOURCE_SHA}"
REPOSITORY = "iii-hq/workers"
INTERFACE = {
    "functions": [{
        "name": "smoke::run", "description": "", "request_schema": {},
        "response_schema": {}, "metadata": {},
    }],
    "triggers": [],
}


def seal(value: dict) -> dict:
    value.pop("descriptor_sha256", None)
    value["descriptor_sha256"] = hashlib.sha256(deployment_train.canonical_bytes(value)).hexdigest()
    return value


def descriptor(worker: str, kind: str, *, interface_capture: str = "required") -> dict:
    base = {
        "contract": "deployment-descriptor",
        "worker": worker,
        "package_manifest_version": "1.4.0",
        "source_sha": SOURCE_SHA,
        "deployment_spec_sha256": "1" * 64,
        "public_manifest_sha256": "2" * 64,
        "registry_projection_sha256": "3" * 64,
        "compiler_digest": "4" * 64,
        "source": {"path": worker, "package_manifest": "Cargo.toml"},
        "runtime": {"environment": {}, "resources": {}, "exec": [worker]},
        "interface_capture": interface_capture,
        "publish": True,
        "registry_projection": {
            "worker_name": worker, "type": "binary", "description": "Smoke",
            "license": "Apache-2.0", "tags": [], "dependencies": [], "config": {},
            "experimental": False, "readme": "",
        },
    }
    if kind == "rust-binary":
        targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
        base["artifact"] = {"kind": kind, "binary": "tool", "targets": targets, "toolchain": {"name": "rust", "version": "1.97.1"}}
        base["build_units"] = [{"id": f"{worker}-{target}", "kind": kind, "target": target} for target in targets]
    elif kind == "oci-image":
        base["artifact"] = {"kind": kind, "context": ".", "dockerfile": "Dockerfile"}
        base["build_units"] = [
            {"id": f"{worker}-linux-amd64", "kind": kind, "platform": "linux/amd64"},
            {"id": f"{worker}-linux-arm64", "kind": kind, "platform": "linux/arm64"},
        ]
        base["runtime"] = {"environment": {}, "resources": {}}
        base["registry_projection"]["type"] = "image"
    else:
        base["artifact"] = {
            "kind": kind, "workspace_root": ".", "runtime": {"name": "node", "version": "22.20.0"},
            "package_manager": {"name": "pnpm", "version": "11.13.1"}, "lockfile": "pnpm-lock.yaml",
            "install_command": ["true"], "build_command": ["true"], "include": ["dist/index.mjs"],
        }
        base["build_units"] = [{"id": "bundle", "kind": kind}]
        base["runtime"] = {"environment": {}, "resources": {}, "install": "", "start": "node dist/index.mjs"}
        base["registry_projection"]["type"] = "bundle"
    return seal(base)


def prepared_release(tmp_path: Path, selected: dict, files: dict[str, tuple[str, str]]) -> Path:
    """Write a deploy-prepared directory; `files` maps prepared name -> (unit, role)."""
    root = tmp_path / "deploy-prepared"
    root.mkdir()
    (root / "deployment-descriptor.json").write_text(json.dumps(selected), encoding="utf-8")
    entries = []
    for name, (unit, role) in files.items():
        path = root / name
        path.write_bytes(f"bytes of {name}".encode())
        entries.append({"unit": unit, "name": name, "role": role, "sha256": deployment_train.sha256(path), "size": path.stat().st_size})
    identity = {"worker": selected["worker"], "source_sha": SOURCE_SHA, "descriptor_sha256": selected["descriptor_sha256"]}
    prepared = {"contract": "prepared-artifacts", **identity, "artifacts": sorted(entries, key=lambda e: (e["unit"], e["name"]))}
    (root / "prepared-artifacts.json").write_text(json.dumps(prepared), encoding="utf-8")
    identity.update(prepared_sha256=deployment_train.sha256(root / "prepared-artifacts.json"), interface_capture=selected["interface_capture"])
    captured = INTERFACE if selected["interface_capture"] == "required" else None
    (root / "deployment-interface.json").write_text(
        json.dumps({"contract": "deployment-interface", **identity, "interface": captured}), encoding="utf-8",
    )
    (root / "deployment-evidence.json").write_text(
        json.dumps({"contract": "deployment-evidence", **identity, "artifacts": entries}), encoding="utf-8",
    )
    return root


def receipt(root: Path, worker: str, image: dict | None = None, **overrides) -> Path:
    prepared = json.loads((root / "prepared-artifacts.json").read_text(encoding="utf-8"))
    document = {
        "contract": "build-upload-receipt",
        "worker": worker,
        "source_sha": SOURCE_SHA,
        "release": {"tag": TAG, "id": 1, "url": f"https://github.com/{REPOSITORY}/releases/tag/{TAG}"},
        "assets": [
            {"name": a["name"], "asset": build_manifest.asset_name(worker, a["name"]), "sha256": a["sha256"], "size": a["size"], "state": "uploaded"}
            for a in prepared["artifacts"]
        ],
        "image": image,
    }
    document.update(overrides)
    path = root.parent / "upload-receipt.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


def write_args(root: Path, receipt_path: Path, worker: str, **overrides) -> argparse.Namespace:
    values = dict(
        worker=worker, source_sha=SOURCE_SHA, descriptor_run_id="123456", correlation_id="batch-7",
        repository=REPOSITORY, run_id=42, run_attempt=1, prepared_dir=root, receipt=receipt_path,
        out=root.parent / "manifest.json", built_at="2026-09-03T00:00:00Z",
    )
    values.update(overrides)
    return argparse.Namespace(**values)


def written(args: argparse.Namespace) -> dict:
    assert build_manifest.write(args) == 0
    return json.loads(args.out.read_text(encoding="utf-8"))


RUST_FILES = {
    "tool-x86_64-unknown-linux-gnu.tar.gz": ("web-x86_64-unknown-linux-gnu", "binary"),
    "tool-x86_64-unknown-linux-gnu.sha256": ("web-x86_64-unknown-linux-gnu", "checksum"),
    "tool-aarch64-apple-darwin.tar.gz": ("web-aarch64-apple-darwin", "binary"),
    "tool-aarch64-apple-darwin.sha256": ("web-aarch64-apple-darwin", "checksum"),
}


def test_asset_names_carry_the_worker_exactly_once() -> None:
    assert build_manifest.asset_name("web", "web-x86_64-unknown-linux-gnu.tar.gz") == "web-x86_64-unknown-linux-gnu.tar.gz"
    assert build_manifest.asset_name("web", "web.tar.gz") == "web.tar.gz"
    assert build_manifest.asset_name("web", "web-image-linux-amd64.oci.tar.gz") == "web-image-linux-amd64.oci.tar.gz"
    assert build_manifest.asset_name("web", "tool-x86_64-unknown-linux-gnu.tar.gz") == "web-tool-x86_64-unknown-linux-gnu.tar.gz"
    # A binary that merely shares a prefix with the worker is still qualified.
    assert build_manifest.asset_name("web", "webhook-x86_64.tar.gz") == "web-webhook-x86_64.tar.gz"


def test_release_tag_is_content_addressed_by_source_sha() -> None:
    assert build_manifest.release_tag(SOURCE_SHA) == TAG
    with pytest.raises(SystemExit):
        build_manifest.release_tag("main")
    with pytest.raises(SystemExit):
        build_manifest.release_tag("A" * 40)


def test_plan_prints_one_row_per_prepared_artifact(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    root = prepared_release(tmp_path, descriptor("web", "rust-binary"), RUST_FILES)
    assert build_manifest.plan(argparse.Namespace(worker="web", prepared=root / "prepared-artifacts.json")) == 0
    rows = [line.split("\t") for line in capsys.readouterr().out.splitlines()]
    assert len(rows) == 4
    assert rows[0][0] == "tool-aarch64-apple-darwin.sha256"
    assert rows[0][1] == "web-tool-aarch64-apple-darwin.sha256"
    assert all(len(row[2]) == 64 and row[3].isdigit() for row in rows)


def test_rust_manifest_maps_every_target_to_a_release_asset_url(tmp_path: Path) -> None:
    selected = descriptor("web", "rust-binary")
    root = prepared_release(tmp_path, selected, RUST_FILES)
    manifest = written(write_args(root, receipt(root, "web"), "web"))

    assert manifest["schema"] == "workers-build-manifest/1"
    assert manifest["worker"] == "web"
    assert manifest["source_sha"] == SOURCE_SHA
    assert manifest["descriptor_sha256"] == selected["descriptor_sha256"]
    assert manifest["descriptor_run_id"] == 123456
    assert manifest["correlation_id"] == "batch-7"
    assert manifest["run_id"] == 42 and manifest["run_attempt"] == 1
    assert manifest["built_at"] == "2026-09-03T00:00:00Z"
    assert manifest["package_manifest_version"] == "1.4.0"
    assert manifest["artifact_kind"] == "rust-binary"
    assert manifest["release"] == {"tag": TAG, "url": f"https://github.com/{REPOSITORY}/releases/tag/{TAG}"}
    assert manifest["image"] is None
    assert manifest["descriptor"] == selected
    assert manifest["interface"] == INTERFACE

    by_name = {artifact["name"]: artifact for artifact in manifest["artifacts"]}
    assert set(by_name) == {
        "web-tool-x86_64-unknown-linux-gnu.tar.gz", "web-tool-x86_64-unknown-linux-gnu.sha256",
        "web-tool-aarch64-apple-darwin.tar.gz", "web-tool-aarch64-apple-darwin.sha256",
    }
    linux = by_name["web-tool-x86_64-unknown-linux-gnu.tar.gz"]
    assert linux["url"] == f"https://github.com/{REPOSITORY}/releases/download/{TAG}/web-tool-x86_64-unknown-linux-gnu.tar.gz"
    assert linux["target"] == "x86_64-unknown-linux-gnu"
    assert linux["role"] == "binary"
    assert linux["prepared_name"] == "tool-x86_64-unknown-linux-gnu.tar.gz"
    assert linux["sha256"] == deployment_train.sha256(root / "tool-x86_64-unknown-linux-gnu.tar.gz")
    assert linux["size"] == (root / "tool-x86_64-unknown-linux-gnu.tar.gz").stat().st_size

    assert manifest["registry_artifacts"]["kind"] == "rust-binary"
    assert set(manifest["registry_artifacts"]["binaries"]) == {"x86_64-unknown-linux-gnu", "aarch64-apple-darwin"}
    assert manifest["registry_artifacts"]["binaries"]["aarch64-apple-darwin"] == {
        "url": by_name["web-tool-aarch64-apple-darwin.tar.gz"]["url"],
        "sha256": by_name["web-tool-aarch64-apple-darwin.tar.gz"]["sha256"],
    }


def test_release_control_can_build_the_registry_payload_from_the_manifest_alone(tmp_path: Path) -> None:
    root = prepared_release(tmp_path, descriptor("web", "rust-binary"), RUST_FILES)
    manifest = written(write_args(root, receipt(root, "web"), "web"))
    payload = build_payload(
        registry_projection=manifest["descriptor"]["registry_projection"],
        published_version="1.4.0-rc.1",
        repo_url=f"https://github.com/{manifest['repository']}",
        interface=manifest["interface"],
        interface_capture=manifest["interface_capture"],
        artifacts=manifest["registry_artifacts"],
    )
    assert payload["version"] == "1.4.0-rc.1"
    assert payload["binaries"] == manifest["registry_artifacts"]["binaries"]
    assert payload["functions"][0]["name"] == "smoke::run"


def test_bundle_manifest_exposes_archive_url_and_sha256(tmp_path: Path) -> None:
    root = prepared_release(
        tmp_path, descriptor("smoke", "javascript-bundle"),
        {"smoke.tar.gz": ("bundle", "bundle"), "smoke.sha256": ("bundle", "checksum")},
    )
    manifest = written(write_args(root, receipt(root, "smoke"), "smoke", correlation_id=""))
    assert manifest["correlation_id"] is None
    assert [artifact["name"] for artifact in manifest["artifacts"]] == ["smoke.sha256", "smoke.tar.gz"]
    assert all(artifact["target"] is None for artifact in manifest["artifacts"])
    assert manifest["registry_artifacts"] == {
        "kind": "javascript-bundle",
        "archive_url": f"https://github.com/{REPOSITORY}/releases/download/{TAG}/smoke.tar.gz",
        "sha256": deployment_train.sha256(root / "smoke.tar.gz"),
    }
    payload = build_payload(
        registry_projection=manifest["descriptor"]["registry_projection"], published_version="1.4.0",
        repo_url="https://github.com/iii-hq/workers", interface=manifest["interface"],
        interface_capture="required", artifacts=manifest["registry_artifacts"],
    )
    assert payload["archive_url"] == manifest["registry_artifacts"]["archive_url"]


def test_skipped_interface_capture_yields_an_empty_interface(tmp_path: Path) -> None:
    root = prepared_release(
        tmp_path, descriptor("smoke", "javascript-bundle", interface_capture="skipped"),
        {"smoke.tar.gz": ("bundle", "bundle"), "smoke.sha256": ("bundle", "checksum")},
    )
    manifest = written(write_args(root, receipt(root, "smoke"), "smoke"))
    assert manifest["interface_capture"] == "skipped"
    assert manifest["interface"] == {"functions": [], "triggers": []}


OCI_FILES = {
    "img-image-linux-amd64.oci.tar.gz": ("img-linux-amd64", "oci-platform"),
    "img-image-linux-amd64.oci.tar.sha256": ("img-linux-amd64", "checksum"),
    "img-image-linux-arm64.oci.tar.gz": ("img-linux-arm64", "oci-platform"),
    "img-image-linux-arm64.oci.tar.sha256": ("img-linux-arm64", "checksum"),
}
IMAGE = {"repository": "ghcr.io/iii-hq/img", "tag": TAG, "digest": "sha256:" + "c" * 64, "state": "pushed"}


def test_oci_manifest_records_the_index_digest_and_a_digest_pinned_image_tag(tmp_path: Path) -> None:
    root = prepared_release(tmp_path, descriptor("img", "oci-image"), OCI_FILES)
    manifest = written(write_args(root, receipt(root, "img", image=IMAGE), "img"))
    assert manifest["image"] == {"repository": "ghcr.io/iii-hq/img", "tag": TAG, "digest": "sha256:" + "c" * 64}
    assert manifest["registry_artifacts"] == {"kind": "oci-image", "image_tag": f"ghcr.io/iii-hq/img:{TAG}@sha256:{'c' * 64}"}
    assert {artifact["target"] for artifact in manifest["artifacts"]} == {"linux/amd64", "linux/arm64"}
    payload = build_payload(
        registry_projection=manifest["descriptor"]["registry_projection"], published_version="1.4.0-rc.2",
        repo_url="https://github.com/iii-hq/workers", interface=manifest["interface"],
        interface_capture="required", artifacts=manifest["registry_artifacts"],
    )
    assert payload["image_tag"] == manifest["registry_artifacts"]["image_tag"]


@pytest.mark.parametrize("image", [
    None,
    {**IMAGE, "digest": "sha256:short"},
    {**IMAGE, "tag": "1.4.0"},
    {**IMAGE, "repository": "ghcr.io/iii-hq/other"},
])
def test_oci_manifest_requires_the_content_addressed_index(tmp_path: Path, image: dict | None) -> None:
    root = prepared_release(tmp_path, descriptor("img", "oci-image"), OCI_FILES)
    with pytest.raises(SystemExit):
        build_manifest.write(write_args(root, receipt(root, "img", image=image), "img"))


def test_non_oci_manifest_rejects_a_stray_image(tmp_path: Path) -> None:
    root = prepared_release(tmp_path, descriptor("web", "rust-binary"), RUST_FILES)
    with pytest.raises(SystemExit):
        build_manifest.write(write_args(root, receipt(root, "web", image=IMAGE), "web"))


def test_manifest_rejects_a_receipt_that_differs_from_the_prepared_bytes(tmp_path: Path) -> None:
    root = prepared_release(tmp_path, descriptor("web", "rust-binary"), RUST_FILES)
    good = json.loads(receipt(root, "web").read_text(encoding="utf-8"))

    tampered = {**good, "assets": [{**good["assets"][0], "sha256": "f" * 64}, *good["assets"][1:]]}
    (root.parent / "upload-receipt.json").write_text(json.dumps(tampered), encoding="utf-8")
    with pytest.raises(SystemExit, match="differs from the prepared bytes"):
        build_manifest.write(write_args(root, root.parent / "upload-receipt.json", "web"))

    missing = {**good, "assets": good["assets"][1:]}
    (root.parent / "upload-receipt.json").write_text(json.dumps(missing), encoding="utf-8")
    with pytest.raises(SystemExit, match="no entry for prepared artifact"):
        build_manifest.write(write_args(root, root.parent / "upload-receipt.json", "web"))

    renamed = {**good, "assets": [{**good["assets"][0], "asset": good["assets"][0]["name"]}, *good["assets"][1:]]}
    (root.parent / "upload-receipt.json").write_text(json.dumps(renamed), encoding="utf-8")
    with pytest.raises(SystemExit, match="differs from the prepared bytes"):
        build_manifest.write(write_args(root, root.parent / "upload-receipt.json", "web"))

    other_tag = {**good, "release": {**good["release"], "tag": "web/v1.4.0"}}
    (root.parent / "upload-receipt.json").write_text(json.dumps(other_tag), encoding="utf-8")
    with pytest.raises(SystemExit, match="release tag differs"):
        build_manifest.write(write_args(root, root.parent / "upload-receipt.json", "web"))


def test_manifest_rejects_tampered_prepared_bytes_and_identity_drift(tmp_path: Path) -> None:
    root = prepared_release(tmp_path, descriptor("web", "rust-binary"), RUST_FILES)
    receipt_path = receipt(root, "web")
    with pytest.raises(SystemExit, match="identity differs"):
        build_manifest.write(write_args(root, receipt_path, "other"))
    with pytest.raises(SystemExit, match="identity differs"):
        build_manifest.write(write_args(root, receipt_path, "web", source_sha="b" * 40))
    with pytest.raises(SystemExit, match="descriptor_run_id"):
        build_manifest.write(write_args(root, receipt_path, "web", descriptor_run_id="latest"))
    (root / "tool-aarch64-apple-darwin.tar.gz").write_bytes(b"tampered")
    with pytest.raises(SystemExit, match="bytes differ from inventory"):
        build_manifest.write(write_args(root, receipt_path, "web"))
