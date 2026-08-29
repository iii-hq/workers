import argparse
import json
from pathlib import Path

import pytest

import release_interface
from release_train import normalized_tar


def descriptor(validation: str = "required") -> dict:
    return {
        "contract": "release-descriptor",
        "worker": "smoke",
        "version": "1.0.0-rc.1",
        "source_sha": "a" * 40,
        "descriptor_sha256": "b" * 64,
        "package": {
            "source": {"path": "source-must-not-be-read", "package_manifest": "Cargo.toml"},
            "artifact": {
                "kind": "rust-binary",
                "binary": "smoke",
                "targets": ["x86_64-unknown-linux-gnu"],
                "toolchain": {"name": "rust", "version": "1.97.1"},
            },
            "runtime": {"exec": ["smoke"]},
            "registry": {"publish": True},
            "validation": {"interface": validation},
        },
        "build_units": [{
            "id": "rust-x86_64-unknown-linux-gnu",
            "kind": "rust-binary",
            "target": "x86_64-unknown-linux-gnu",
        }],
    }


def write_prepared(tmp_path: Path, selected: dict) -> tuple[Path, Path]:
    descriptor_path = tmp_path / "release-descriptor.json"
    descriptor_path.write_text(json.dumps(selected), encoding="utf-8")
    executable = tmp_path / "smoke"
    executable.write_bytes(b"prepared executable")
    executable.chmod(0o755)
    archive = tmp_path / "smoke-x86_64-unknown-linux-gnu.tar.gz"
    normalized_tar([(executable, "smoke")], archive)
    prepared = {
        "contract": "prepared-artifacts",
        "worker": "smoke",
        "source_sha": "a" * 40,
        "descriptor_sha256": "b" * 64,
        "artifacts": [{
            "unit": "rust-x86_64-unknown-linux-gnu",
            "name": archive.name,
            "role": "binary",
            "sha256": release_interface.sha256(archive),
            "size": archive.stat().st_size,
        }],
    }
    prepared_path = tmp_path / "prepared-artifacts.json"
    prepared_path.write_text(json.dumps(prepared), encoding="utf-8")
    return descriptor_path, prepared_path


def test_required_interface_stages_only_prepared_artifact_bytes(tmp_path: Path) -> None:
    descriptor_path, prepared_path = write_prepared(tmp_path, descriptor())
    stage_path = tmp_path / "interface" / "stage.json"

    release_interface.stage(argparse.Namespace(
        descriptor=descriptor_path,
        prepared=prepared_path,
        out=stage_path,
        github_output=None,
    ))

    stage = json.loads(stage_path.read_text())
    assert stage["validation"] == "required"
    assert stage["kind"] == "rust-binary"
    assert stage["command"] == [str(stage_path.parent / "runtime" / "smoke")]
    assert (stage_path.parent / "runtime" / "smoke").read_bytes() == b"prepared executable"
    assert not (tmp_path / "source-must-not-be-read").exists()


def test_skipped_interface_records_explicit_evidence_without_staging_runtime(tmp_path: Path) -> None:
    selected = descriptor("skipped")
    descriptor_path, prepared_path = write_prepared(tmp_path, selected)
    stage_path = tmp_path / "interface" / "stage.json"
    snapshot_path = tmp_path / "release-interface.json"

    release_interface.stage(argparse.Namespace(
        descriptor=descriptor_path,
        prepared=prepared_path,
        out=stage_path,
        github_output=None,
    ))
    release_interface.snapshot(argparse.Namespace(
        descriptor=descriptor_path,
        interface=None,
        out=snapshot_path,
    ))

    assert json.loads(stage_path.read_text())["validation"] == "skipped"
    assert not (stage_path.parent / "runtime").exists()
    assert json.loads(snapshot_path.read_text()) == {
        "contract": "release-interface",
        "worker": "smoke",
        "source_sha": "a" * 40,
        "descriptor_sha256": "b" * 64,
        "validation": "skipped",
        "interface": None,
    }


def test_required_interface_snapshot_compares_canonical_published_surface(tmp_path: Path) -> None:
    descriptor_path, _prepared_path = write_prepared(tmp_path, descriptor())
    actual = tmp_path / "actual.json"
    actual.write_text(json.dumps({
        "functions": [
            {"name": "z", "request_schema": {}, "response_schema": {}},
            {"name": "a", "request_schema": {}, "response_schema": {}},
        ],
        "triggers": [],
    }), encoding="utf-8")
    snapshot_path = tmp_path / "release-interface.json"
    release_interface.snapshot(argparse.Namespace(
        descriptor=descriptor_path,
        interface=actual,
        out=snapshot_path,
    ))

    published = tmp_path / "published.json"
    published.write_text(json.dumps({
        "functions": list(reversed(json.loads(actual.read_text())["functions"])),
        "triggers": [],
    }), encoding="utf-8")
    release_interface.compare(argparse.Namespace(
        descriptor=descriptor_path,
        expected=snapshot_path,
        actual=published,
    ))

    published.write_text(json.dumps({"functions": [{"name": "different"}], "triggers": []}))
    with pytest.raises(SystemExit, match="differs"):
        release_interface.compare(argparse.Namespace(
            descriptor=descriptor_path,
            expected=snapshot_path,
            actual=published,
        ))


def test_skipped_interface_rejects_runtime_collection(tmp_path: Path) -> None:
    selected = descriptor("skipped")
    descriptor_path, _prepared_path = write_prepared(tmp_path, selected)
    snapshot_path = tmp_path / "release-interface.json"
    release_interface.snapshot(argparse.Namespace(
        descriptor=descriptor_path,
        interface=None,
        out=snapshot_path,
    ))
    actual = tmp_path / "actual.json"
    actual.write_text('{"functions":[],"triggers":[]}', encoding="utf-8")

    with pytest.raises(SystemExit, match="explicit and empty"):
        release_interface.compare(argparse.Namespace(
            descriptor=descriptor_path,
            expected=snapshot_path,
            actual=actual,
        ))


def test_required_trigger_only_interface_is_valid(tmp_path: Path) -> None:
    descriptor_path, _prepared_path = write_prepared(tmp_path, descriptor())
    actual = tmp_path / "trigger-only.json"
    actual.write_text(json.dumps({
        "functions": [],
        "triggers": [{"name": "smoke::on-event", "invocation_schema": {"type": "object"}}],
    }), encoding="utf-8")
    snapshot_path = tmp_path / "release-interface.json"

    release_interface.snapshot(argparse.Namespace(
        descriptor=descriptor_path,
        interface=actual,
        out=snapshot_path,
    ))

    assert json.loads(snapshot_path.read_text())["interface"]["triggers"][0]["name"] == "smoke::on-event"


def test_final_evidence_inventory_covers_descriptor_interface_and_build_bytes(tmp_path: Path) -> None:
    descriptor_path, prepared_path = write_prepared(tmp_path, descriptor())
    interface_path = tmp_path / "release-interface.json"
    interface_path.write_text(json.dumps({
        "contract": "release-interface",
        "worker": "smoke",
        "source_sha": "a" * 40,
        "descriptor_sha256": "b" * 64,
        "validation": "required",
        "interface": {"functions": [{"name": "smoke"}], "triggers": []},
    }), encoding="utf-8")
    evidence_path = tmp_path / "release-evidence.json"

    release_interface.build_evidence(argparse.Namespace(
        descriptor=descriptor_path,
        prepared=prepared_path,
        interface=interface_path,
        out=evidence_path,
    ))
    release_interface.verify_evidence(argparse.Namespace(
        descriptor=descriptor_path,
        evidence=evidence_path,
    ))

    roles = {artifact["role"] for artifact in json.loads(evidence_path.read_text())["artifacts"]}
    assert {"binary", "descriptor", "prepared-inventory", "interface"}.issubset(roles)
    interface_path.write_text("{}")
    with pytest.raises(SystemExit, match="bytes differ"):
        release_interface.verify_evidence(argparse.Namespace(
            descriptor=descriptor_path,
            evidence=evidence_path,
        ))


def test_adapter_applies_only_descriptor_runtime_environment_plus_engine_url(tmp_path: Path) -> None:
    metadata = tmp_path / "stage.json"
    metadata.write_text(json.dumps({
        "contract": "release-interface-stage",
        "validation": "required",
        "kind": "javascript-bundle",
        "cwd": str(tmp_path),
        "prepare": [],
        "command": [
            "python3", "-c",
            "import os; assert os.environ['WORKER_SETTING']=='prepared'; "
            "assert os.environ['III_URL']=='ws://127.0.0.1:1234'",
        ],
        "environment": {"WORKER_SETTING": "prepared"},
    }), encoding="utf-8")

    assert release_interface.run_adapter(argparse.Namespace(
        metadata=metadata,
        engine_url="ws://127.0.0.1:1234",
        log=tmp_path / "adapter.log",
    )) == 0
