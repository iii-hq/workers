import argparse
import json
from pathlib import Path

import pytest

import deployment_interface
from deployment_train import normalized_tar


def write_prepared_inputs(
    tmp_path: Path,
    *,
    worker: str = "web",
    interface_capture: str = "required",
) -> tuple[Path, Path]:
    descriptor_path = tmp_path / "deployment-descriptor.json"
    descriptor_path.write_text(
        json.dumps({
            "worker": worker,
            "source_sha": "a" * 40,
            "descriptor_sha256": "b" * 64,
            "interface_capture": interface_capture,
        }),
        encoding="utf-8",
    )
    artifact = tmp_path / f"{worker}.bin"
    artifact.write_bytes(b"immutable prepared worker")
    prepared_path = tmp_path / "prepared-artifacts.json"
    prepared_path.write_text(
        json.dumps({
            "contract": "prepared-artifacts",
            "worker": worker,
            "source_sha": "a" * 40,
            "descriptor_sha256": "b" * 64,
            "artifacts": [{
                "unit": "linux",
                "name": artifact.name,
                "role": "binary",
                "size": artifact.stat().st_size,
                "sha256": deployment_interface.sha256(artifact),
            }],
        }),
        encoding="utf-8",
    )
    return descriptor_path, prepared_path


def snapshot_args(
    descriptor: Path,
    prepared: Path,
    interface: Path | None,
    out: Path,
) -> argparse.Namespace:
    return argparse.Namespace(
        descriptor=descriptor,
        prepared=prepared,
        interface=interface,
        out=out,
    )


def write_archive_prepared_inputs(
    tmp_path: Path,
    *,
    selected: dict,
    member_name: str,
    member_bytes: bytes,
    unit: str,
    role: str,
) -> tuple[Path, Path]:
    descriptor_path = tmp_path / "deployment-descriptor.json"
    descriptor_path.write_text(json.dumps(selected), encoding="utf-8")
    member = tmp_path / "archive-member"
    member.write_bytes(member_bytes)
    member.chmod(0o755)
    archive = tmp_path / f"{selected['worker']}-{unit}.tar.gz"
    normalized_tar([(member, member_name)], archive)
    prepared_path = tmp_path / "prepared-artifacts.json"
    prepared_path.write_text(
        json.dumps({
            "contract": "prepared-artifacts",
            "worker": selected["worker"],
            "source_sha": selected["source_sha"],
            "descriptor_sha256": selected["descriptor_sha256"],
            "artifacts": [{
                "unit": unit,
                "name": archive.name,
                "role": role,
                "size": archive.stat().st_size,
                "sha256": deployment_interface.sha256(archive),
            }],
        }),
        encoding="utf-8",
    )
    return descriptor_path, prepared_path


def test_required_rust_capture_stages_only_prepared_bytes_with_absolute_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    selected = {
        "worker": "smoke",
        "source_sha": "a" * 40,
        "descriptor_sha256": "b" * 64,
        "interface_capture": "required",
        "source": {
            "path": "source-must-not-be-read",
            "package_manifest": "Cargo.toml",
        },
        "artifact": {
            "kind": "rust-binary",
            "binary": "smoke",
            "targets": ["x86_64-unknown-linux-gnu"],
            "toolchain": {"name": "rust", "version": "1.97.1"},
        },
        "runtime": {
            "exec": ["smoke", "--capture-interface"],
            "environment": {},
            "resources": {},
        },
        "build_units": [{
            "id": "rust-x86_64-unknown-linux-gnu",
            "kind": "rust-binary",
            "target": "x86_64-unknown-linux-gnu",
        }],
    }
    descriptor, prepared = write_archive_prepared_inputs(
        tmp_path,
        selected=selected,
        member_name="smoke",
        member_bytes=b"prepared executable",
        unit="rust-x86_64-unknown-linux-gnu",
        role="binary",
    )
    monkeypatch.chdir(tmp_path)
    stage_path = Path("interface") / "stage.json"

    assert deployment_interface.stage(argparse.Namespace(
        descriptor=descriptor,
        prepared=prepared,
        out=stage_path,
        github_output=None,
    )) == 0

    stage = json.loads(stage_path.read_text(encoding="utf-8"))
    runtime = Path(stage["cwd"])
    executable = Path(stage["command"][0])
    assert stage["interface_capture"] == "required"
    assert stage["kind"] == "rust-binary"
    assert runtime.is_absolute()
    assert executable.is_absolute()
    assert executable.is_relative_to(runtime)
    assert stage["command"][1:] == ["--capture-interface"]
    assert executable.read_bytes() == b"prepared executable"
    assert not (tmp_path / "source-must-not-be-read").exists()


@pytest.mark.parametrize(
    ("kind", "member_name", "start", "runtime", "package_manager"),
    [
        (
            "javascript-bundle",
            "dist/index.mjs",
            "node dist/index.mjs",
            {"name": "node", "version": "22.20.0"},
            {"name": "pnpm", "version": "11.13.1"},
        ),
        (
            "python-bundle",
            "worker.py",
            "python worker.py",
            {"name": "python", "version": "3.12.13"},
            {"name": "uv", "version": "0.8.15"},
        ),
    ],
)
def test_required_bundle_capture_stages_prepared_archive(
    tmp_path: Path,
    kind: str,
    member_name: str,
    start: str,
    runtime: dict[str, str],
    package_manager: dict[str, str],
) -> None:
    selected = {
        "worker": "bundle",
        "source_sha": "a" * 40,
        "descriptor_sha256": "b" * 64,
        "interface_capture": "required",
        "source": {
            "path": "source-must-not-be-read",
            "package_manifest": "package.json" if kind == "javascript-bundle" else "pyproject.toml",
        },
        "artifact": {
            "kind": kind,
            "runtime": runtime,
            "package_manager": package_manager,
        },
        "runtime": {
            "install": "echo install prepared dependencies",
            "start": start,
            "environment": {"BUNDLE_SETTING": "prepared"},
            "resources": {},
        },
        "build_units": [{"id": "bundle", "kind": kind}],
    }
    descriptor, prepared = write_archive_prepared_inputs(
        tmp_path,
        selected=selected,
        member_name=member_name,
        member_bytes=b"prepared bundle",
        unit="bundle",
        role="bundle",
    )
    stage_path = tmp_path / "interface" / "stage.json"

    assert deployment_interface.stage(argparse.Namespace(
        descriptor=descriptor,
        prepared=prepared,
        out=stage_path,
        github_output=None,
    )) == 0

    stage = json.loads(stage_path.read_text(encoding="utf-8"))
    assert stage["kind"] == kind
    assert stage["command"] == ["sh", "-c", start]
    assert stage["prepare"] == [["sh", "-c", "echo install prepared dependencies"]]
    assert stage["runtime_name"] == runtime["name"]
    assert stage["runtime_version"] == runtime["version"]
    assert stage["package_manager_name"] == package_manager["name"]
    assert stage["package_manager_version"] == package_manager["version"]
    assert (Path(stage["cwd"]) / member_name).read_bytes() == b"prepared bundle"
    assert not (tmp_path / "source-must-not-be-read").exists()


def test_required_capture_needs_present_non_empty_interface(tmp_path: Path) -> None:
    descriptor, prepared = write_prepared_inputs(tmp_path)
    snapshot = tmp_path / "deployment-interface.json"

    with pytest.raises(SystemExit, match="needs collected interface bytes"):
        deployment_interface.snapshot(
            snapshot_args(descriptor, prepared, None, snapshot)
        )

    collected = tmp_path / "collected-interface.json"
    collected.write_text('{"functions":[],"triggers":[]}', encoding="utf-8")
    with pytest.raises(SystemExit, match="must contain a function or trigger"):
        deployment_interface.snapshot(
            snapshot_args(descriptor, prepared, collected, snapshot)
        )

    collected.write_text(
        json.dumps({
            "functions": [{"name": "web::fetch"}],
            "triggers": [],
        }),
        encoding="utf-8",
    )
    assert deployment_interface.snapshot(
        snapshot_args(descriptor, prepared, collected, snapshot)
    ) == 0
    captured = json.loads(snapshot.read_text(encoding="utf-8"))
    assert captured["interface_capture"] == "required"
    assert captured["prepared_sha256"] == deployment_interface.sha256(prepared)
    assert captured["interface"]["functions"] == [{"name": "web::fetch"}]


def test_required_capture_compares_canonical_published_surface(tmp_path: Path) -> None:
    descriptor, prepared = write_prepared_inputs(tmp_path)
    collected = tmp_path / "collected-interface.json"
    collected.write_text(
        json.dumps({
            "functions": [
                {"name": "web::z", "request_schema": {}, "response_schema": {}},
                {"name": "web::a", "request_schema": {}, "response_schema": {}},
            ],
            "triggers": [],
        }),
        encoding="utf-8",
    )
    snapshot = tmp_path / "deployment-interface.json"
    deployment_interface.snapshot(
        snapshot_args(descriptor, prepared, collected, snapshot)
    )
    published = tmp_path / "published-interface.json"
    published.write_text(
        json.dumps({
            "functions": list(reversed(json.loads(collected.read_text())["functions"])),
            "triggers": [],
        }),
        encoding="utf-8",
    )

    assert deployment_interface.compare(argparse.Namespace(
        descriptor=descriptor,
        expected=snapshot,
        actual=published,
    )) == 0

    published.write_text(
        '{"functions":[{"name":"web::different"}],"triggers":[]}',
        encoding="utf-8",
    )
    with pytest.raises(SystemExit, match="differs"):
        deployment_interface.compare(argparse.Namespace(
            descriptor=descriptor,
            expected=snapshot,
            actual=published,
        ))


def test_required_trigger_only_interface_is_valid(tmp_path: Path) -> None:
    descriptor, prepared = write_prepared_inputs(tmp_path)
    collected = tmp_path / "trigger-only.json"
    collected.write_text(
        json.dumps({
            "functions": [],
            "triggers": [{
                "name": "web::on-event",
                "invocation_schema": {"type": "object"},
            }],
        }),
        encoding="utf-8",
    )
    snapshot = tmp_path / "deployment-interface.json"

    assert deployment_interface.snapshot(
        snapshot_args(descriptor, prepared, collected, snapshot)
    ) == 0
    captured = json.loads(snapshot.read_text(encoding="utf-8"))
    assert captured["interface"]["functions"] == []
    assert captured["interface"]["triggers"][0]["name"] == "web::on-event"


@pytest.mark.parametrize("worker", ["acp", "lsp"])
def test_skipped_capture_records_explicit_null_interface(
    tmp_path: Path, worker: str
) -> None:
    descriptor, prepared = write_prepared_inputs(
        tmp_path,
        worker=worker,
        interface_capture="skipped",
    )
    snapshot = tmp_path / "deployment-interface.json"

    assert deployment_interface.snapshot(
        snapshot_args(descriptor, prepared, None, snapshot)
    ) == 0
    captured = json.loads(snapshot.read_text(encoding="utf-8"))
    assert captured["interface_capture"] == "skipped"
    assert captured["interface"] is None


def test_evidence_verification_rejects_missing_interface_artifact(tmp_path: Path) -> None:
    descriptor, prepared = write_prepared_inputs(tmp_path)
    collected = tmp_path / "collected-interface.json"
    collected.write_text(
        '{"functions":[{"name":"web::fetch"}],"triggers":[]}',
        encoding="utf-8",
    )
    snapshot = tmp_path / "deployment-interface.json"
    deployment_interface.snapshot(
        snapshot_args(descriptor, prepared, collected, snapshot)
    )
    evidence = tmp_path / "deployment-evidence.json"
    deployment_interface.build_evidence(argparse.Namespace(
        descriptor=descriptor,
        prepared=prepared,
        interface=snapshot,
        out=evidence,
    ))
    snapshot.unlink()

    with pytest.raises(SystemExit, match="release evidence file is missing"):
        deployment_interface.verify_evidence(argparse.Namespace(
            descriptor=descriptor,
            evidence=evidence,
        ))


def test_evidence_verification_rejects_required_empty_interface(tmp_path: Path) -> None:
    descriptor, prepared = write_prepared_inputs(tmp_path)
    snapshot = tmp_path / "deployment-interface.json"
    snapshot.write_text(
        json.dumps({
            "contract": "deployment-interface",
            "worker": "web",
            "source_sha": "a" * 40,
            "descriptor_sha256": "b" * 64,
            "prepared_sha256": deployment_interface.sha256(prepared),
            "interface_capture": "required",
            "interface": {"functions": [], "triggers": []},
        }),
        encoding="utf-8",
    )
    evidence = tmp_path / "deployment-evidence.json"
    deployment_interface.build_evidence(argparse.Namespace(
        descriptor=descriptor,
        prepared=prepared,
        interface=snapshot,
        out=evidence,
    ))

    with pytest.raises(SystemExit, match="required release interface is empty"):
        deployment_interface.verify_evidence(argparse.Namespace(
            descriptor=descriptor,
            evidence=evidence,
        ))


def test_evidence_inventory_covers_interface_and_prepared_bytes_and_detects_tamper(
    tmp_path: Path,
) -> None:
    descriptor, prepared = write_prepared_inputs(tmp_path)
    collected = tmp_path / "collected-interface.json"
    collected.write_text(
        '{"functions":[{"name":"web::fetch"}],"triggers":[]}',
        encoding="utf-8",
    )
    snapshot = tmp_path / "deployment-interface.json"
    deployment_interface.snapshot(
        snapshot_args(descriptor, prepared, collected, snapshot)
    )
    evidence = tmp_path / "deployment-evidence.json"
    deployment_interface.build_evidence(argparse.Namespace(
        descriptor=descriptor,
        prepared=prepared,
        interface=snapshot,
        out=evidence,
    ))

    assert deployment_interface.verify_evidence(argparse.Namespace(
        descriptor=descriptor,
        evidence=evidence,
    )) == 0
    roles = {
        artifact["role"]
        for artifact in json.loads(evidence.read_text(encoding="utf-8"))["artifacts"]
    }
    assert {"binary", "descriptor", "prepared-inventory", "interface"}.issubset(roles)

    snapshot.write_text("{}", encoding="utf-8")
    with pytest.raises(SystemExit, match="bytes differ"):
        deployment_interface.verify_evidence(argparse.Namespace(
            descriptor=descriptor,
            evidence=evidence,
        ))


def test_run_worker_applies_descriptor_environment_and_engine_url(tmp_path: Path) -> None:
    metadata = tmp_path / "stage.json"
    metadata.write_text(
        json.dumps({
            "contract": "deployment-interface-stage",
            "interface_capture": "required",
            "kind": "javascript-bundle",
            "cwd": str(tmp_path),
            "prepare": [],
            "command": [
                "python3",
                "-c",
                "import os; assert os.environ['WORKER_SETTING'] == 'prepared'; "
                "assert os.environ['III_URL'] == 'ws://127.0.0.1:1234'",
            ],
            "environment": {"WORKER_SETTING": "prepared"},
        }),
        encoding="utf-8",
    )

    assert deployment_interface.run_worker(argparse.Namespace(
        metadata=metadata,
        engine_url="ws://127.0.0.1:1234",
        log=tmp_path / "worker.log",
    )) == 0
