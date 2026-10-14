"""Go binaries share the immutable binary release contract with Rust workers."""
import argparse
import hashlib
import json
import os
import subprocess
import tarfile
from pathlib import Path

import pytest
import yaml

import _lib
import build_manifest
import deployment_compiler
import deployment_interface
import deployment_targets
import deployment_train
import discover_changed_workers
import validate_worker
from build_publish_payload import build_payload


ROOT = Path(__file__).resolve().parents[3]


def test_whatsapp_release_metadata_is_compatible_with_version_sync():
    worker = _lib.read_worker("whatsapp", ROOT / ".deploy/workers.yaml")
    version = _lib.read_version(worker.path / worker.manifest)
    _lib.parse_semver(version)
    assert version != worker.artifact["toolchain"]["version"]


def test_compiler_digest_changes_when_go_target_policy_changes(tmp_path, monkeypatch):
    policy = tmp_path / "deployment_targets.py"
    policy.write_text("GOOS = 'linux'\n")
    monkeypatch.setattr(deployment_targets, "__file__", str(policy))
    script = ROOT / ".github/scripts/deployment_compiler.py"
    schema = ROOT / ".github/contracts/deployment-descriptor.schema.json"
    before = deployment_compiler.compiler_digest(script, schema)
    policy.write_text("GOOS = 'darwin'\n")
    assert deployment_compiler.compiler_digest(script, schema) != before


def go_worker(root: Path) -> dict:
    worker = root / "smoke"
    worker.mkdir()
    (worker / "package.json").write_text('{"name":"smoke","version":"0.4.2-rc.3","private":true}\n')
    (worker / "go.mod").write_text("module example.test/smoke\n\ngo 1.27.1\n")
    (worker / "go.sum").touch()
    (worker / "config.collect.yaml").write_text("data_dir: /tmp/capture\n")
    (worker / "iii.worker.yaml").write_text(
        "iii: v1\nname: smoke\nlanguage: go\ndeploy: binary\nmanifest: package.json\n"
        "bin: smoke\nlicense: Apache-2.0\ndescription: Go smoke worker\ntags: [smoke]\n"
    )
    return {
        "source": {"path": "smoke", "package_manifest": "package.json"},
        "artifact": {
            "kind": "go-binary", "binary": "smoke",
            "toolchain": {"name": "go", "version": "1.27.1"},
            "targets": ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"],
            "windows_exception": "This fixture only supports Unix.",
        },
        "publish": True,
    }


def compile_go(root: Path, value: dict) -> dict:
    return deployment_compiler.compile_worker(root, "smoke", value, "a" * 40, "b" * 64)


def test_go_descriptor_uses_worker_version_and_binary_runtime(tmp_path):
    descriptor = compile_go(tmp_path, go_worker(tmp_path))
    assert descriptor["package_manifest_version"] == "0.4.2-rc.3"
    assert descriptor["runtime"]["exec"] == ["smoke"]
    assert descriptor["runtime"]["interface_config"]["path"] == "config.collect.yaml"
    assert descriptor["build_units"] == [
        {"id": "smoke-x86_64-unknown-linux-gnu", "kind": "go-binary", "target": "x86_64-unknown-linux-gnu"},
        {"id": "smoke-aarch64-apple-darwin", "kind": "go-binary", "target": "aarch64-apple-darwin"},
    ]
    schema = json.loads((ROOT / ".github/contracts/deployment-descriptor.schema.json").read_text())
    pytest.importorskip("jsonschema").Draft202012Validator(schema).validate(descriptor)


@pytest.mark.parametrize(("field", "value", "message"), [
    ("toolchain", {"name": "rust", "version": "1.97.1"}, "Go version"),
    ("targets", ["invented-go-target"], "unknown release target"),
])
def test_go_descriptor_rejects_invalid_build_inputs(tmp_path, field, value, message):
    entry = go_worker(tmp_path)
    entry["artifact"][field] = value
    with pytest.raises(ValueError, match=message):
        compile_go(tmp_path, entry)


def test_go_catalog_and_pr_validation_accept_adjacent_go_tests(tmp_path, monkeypatch, capsys):
    entry = go_worker(tmp_path)
    (tmp_path / "smoke/README.md").write_text("# Smoke\n")
    (tmp_path / "smoke/main_test.go").write_text("package main\n")
    (tmp_path / ".deploy").mkdir()
    catalog = tmp_path / ".deploy/workers.yaml"
    catalog.write_text(yaml.safe_dump({"workers": {"smoke": entry}}))
    spec = _lib.read_worker("smoke", catalog)
    assert spec.runtime["exec"] == ["smoke"]
    assert discover_changed_workers.language_of(spec) == "go"
    errors = []
    validate_worker.validate_public_manifest("smoke", spec, errors.append)
    assert errors == []
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(_lib, "worker_catalog_path", lambda: catalog)
    assert validate_worker.main(["--worker", "smoke", "--base-ref", "HEAD", "--source-changed", '["smoke"]']) == 0


def test_discovery_routes_go_changes_to_go_job(tmp_path, monkeypatch, capsys):
    entry = go_worker(tmp_path)
    (tmp_path / ".deploy").mkdir()
    (tmp_path / ".deploy/workers.yaml").write_text(yaml.safe_dump({"workers": {"smoke": entry}}))
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(discover_changed_workers, "changed_files", lambda *args: ["smoke/main.go"])
    monkeypatch.setenv("GITHUB_OUTPUT", str(tmp_path / "outputs"))
    assert discover_changed_workers.main(["--base", "HEAD"]) == 0
    assert json.loads(capsys.readouterr().out)["by_language"]["go"] == ["smoke"]
    assert 'go=["smoke"]' in (tmp_path / "outputs").read_text()


@pytest.mark.parametrize(("target", "expected"), [
    ("x86_64-unknown-linux-gnu", {"GOOS": "linux", "GOARCH": "amd64"}),
    ("x86_64-unknown-linux-musl", {"GOOS": "linux", "GOARCH": "amd64"}),
    ("aarch64-unknown-linux-gnu", {"GOOS": "linux", "GOARCH": "arm64"}),
    ("armv7-unknown-linux-gnueabihf", {"GOOS": "linux", "GOARCH": "arm", "GOARM": "7"}),
    ("x86_64-apple-darwin", {"GOOS": "darwin", "GOARCH": "amd64"}),
    ("aarch64-apple-darwin", {"GOOS": "darwin", "GOARCH": "arm64"}),
    ("x86_64-pc-windows-msvc", {"GOOS": "windows", "GOARCH": "amd64"}),
    ("aarch64-pc-windows-msvc", {"GOOS": "windows", "GOARCH": "arm64"}),
])
def test_go_build_maps_registry_targets_to_go_environment(target, expected):
    assert deployment_targets.go_target_env(target) == expected


def test_go_metadata_pins_toolchain_and_matrix_uses_target_runners(tmp_path, capsys):
    descriptor = compile_go(tmp_path, go_worker(tmp_path))
    path = tmp_path / "descriptor.json"
    path.write_text(json.dumps(descriptor))
    assert deployment_train.build_metadata(argparse.Namespace(descriptor=path, unit=descriptor["build_units"][0]["id"], github_output=None)) == 0
    metadata = json.loads(capsys.readouterr().out)
    assert (metadata["toolchain_name"], metadata["toolchain_version"]) == ("go", "1.27.1")
    assert deployment_train.matrix(argparse.Namespace(descriptor=path)) == 0
    assert [unit["runner"] for unit in json.loads(capsys.readouterr().out)["include"]] == [
        "workers-release-linux-8core", "workers-release-macos-arm-5core",
    ]


def test_go_build_produces_versioned_readonly_static_binary_archive(tmp_path, monkeypatch):
    descriptor = compile_go(tmp_path, go_worker(tmp_path))
    path = tmp_path / "descriptor.json"
    path.write_text(json.dumps(descriptor))
    tools = tmp_path / "tools"
    tools.mkdir()
    compiler = tools / "go"
    compiler.write_text(
        '#!/usr/bin/env python3\nimport os, sys\nfrom pathlib import Path\n'
        'assert os.environ["CGO_ENABLED"] == "0"\n'
        'assert os.environ["GOOS"] == "linux" and os.environ["GOARCH"] == "amd64"\n'
        'assert os.environ["GOWORK"] == "off"\n'
        'assert sys.argv[1:5] == ["build", "-mod=readonly", "-trimpath", "-ldflags"]\n'
        'assert sys.argv[5] == "-s -w -X main.version=0.4.2-rc.3"\n'
        'assert sys.argv[6] == "-o" and sys.argv[8] == "."\n'
        'Path(sys.argv[7]).write_text("#!/bin/sh\\necho smoke 0.4.2-rc.3\\n")\n'
    )
    compiler.chmod(0o755)
    monkeypatch.setenv("PATH", f"{tools}:{os.environ['PATH']}")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(subprocess, "check_output", lambda *args, **kwargs: "a" * 40)
    out = tmp_path / "release"
    assert deployment_train.build(argparse.Namespace(descriptor=path, unit=descriptor["build_units"][0]["id"], out=out)) == 0
    archive = out / "smoke-x86_64-unknown-linux-gnu.tar.gz"
    with tarfile.open(archive) as tar:
        assert tar.getnames() == ["smoke"]
        assert tar.getmember("smoke").mode == 0o755
        assert tar.extractfile("smoke").read() == b"#!/bin/sh\necho smoke 0.4.2-rc.3\n"
    result = json.loads((out / "build-result.json").read_text())
    assert [artifact["role"] for artifact in result["artifacts"]] == ["binary", "checksum"]
    assert result["artifacts"][0]["sha256"] == hashlib.sha256(archive.read_bytes()).hexdigest()


def test_go_interface_capture_stages_isolated_prepared_binary(tmp_path):
    descriptor = compile_go(tmp_path, go_worker(tmp_path))
    descriptor_path = tmp_path / "descriptor.json"
    descriptor_path.write_text(json.dumps(descriptor))
    binary = tmp_path / "program"
    binary.write_text("#!/bin/sh\nexit 0\n")
    archive = tmp_path / "smoke.tar.gz"
    deployment_train.normalized_tar([(binary, "smoke")], archive, executables={"smoke"})
    prepared = tmp_path / "prepared-artifacts.json"
    prepared.write_text(json.dumps({
        "contract": "prepared-artifacts", "worker": "smoke", "source_sha": "a" * 40,
        "descriptor_sha256": descriptor["descriptor_sha256"], "artifacts": [{
            "unit": "smoke-x86_64-unknown-linux-gnu", "name": archive.name, "role": "binary",
            "size": archive.stat().st_size, "sha256": deployment_interface.sha256(archive),
        }],
    }))
    output = tmp_path / "capture/stage.json"
    assert deployment_interface.stage(argparse.Namespace(descriptor=descriptor_path, prepared=prepared, source_root=tmp_path, out=output, github_output=None)) == 0
    stage = json.loads(output.read_text())
    assert stage["command"] == [
        str(tmp_path / "capture/runtime/smoke"), "--config", str(tmp_path / "smoke/config.collect.yaml"),
        "--local-config", "--collect-interface",
    ]
    assert stage["prepare"] == []
    assert stage["runtime_name"] is None


def test_go_archives_publish_as_registry_binaries():
    artifacts = [{"role": "binary", "target": "x86_64-unknown-linux-gnu", "url": "https://example.test/smoke.tar.gz", "sha256": "c" * 64}]
    prepared = build_manifest._registry_artifacts("go-binary", artifacts, None)
    payload = build_payload(
        registry_projection={
            "worker_name": "smoke", "type": "binary", "description": "Go worker", "license": "Apache-2.0",
            "tags": ["smoke"], "dependencies": [], "config": {}, "experimental": False, "readme": "",
        },
        published_version="0.4.2", repo_url="https://github.com/iii-hq/workers", interface_capture="required",
        interface={"functions": [{"name": "smoke::run"}], "triggers": []}, artifacts=prepared,
    )
    assert payload["type"] == "binary"
    assert payload["binaries"] == {"x86_64-unknown-linux-gnu": {"url": "https://example.test/smoke.tar.gz", "sha256": "c" * 64}}
