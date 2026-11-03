"""`iii_engine_version` in iii.worker.yaml is validated and carried into the publish payload."""
import json
from pathlib import Path

import pytest
import yaml

import _lib
import validate_worker
from build_publish_payload import build_payload
from test_build_publish_payload import NON_EMPTY_INTERFACE
from test_go_deployment import ROOT, compile_go, go_worker


def smoke_worker(tmp_path, line=None):
    """A Go worker whose iii.worker.yaml gets `line` appended; returns (entry, spec)."""
    entry = go_worker(tmp_path)
    manifest = tmp_path / "smoke/iii.worker.yaml"
    if line is not None:
        manifest.write_text(manifest.read_text() + line + "\n")
    (tmp_path / "smoke/README.md").write_text("# Smoke\n")
    (tmp_path / "smoke/main_test.go").write_text("package main\n")
    (tmp_path / ".deploy").mkdir()
    catalog = tmp_path / ".deploy/workers.yaml"
    catalog.write_text(yaml.safe_dump({"workers": {"smoke": entry}}))
    return entry, _lib.read_worker("smoke", catalog)


def validation_errors(tmp_path, line):
    _, spec = smoke_worker(tmp_path, line)
    errors = []
    validate_worker.validate_public_manifest("smoke", spec, errors.append)
    return errors


@pytest.mark.parametrize("good", [
    '">=0.26.0"', '"^0.26.0"', '">=0.26.0 <0.28.0"', '"^0.25.0 || ^0.26.0"', '"0.26.0"',
])
def test_valid_range_passes(tmp_path, good):
    assert validation_errors(tmp_path, f"iii_engine_version: {good}") == []


@pytest.mark.parametrize("bad", [
    '"0.26"', '"v0.26.0"', '">=0.26.0-rc.1"', '">=01.2.3"', '"0.26.x"',
    '"1.0.0 - 2.0.0"', '""', '"0.26.0\\n"',
])
def test_malformed_range_is_rejected(tmp_path, bad):
    errors = validation_errors(tmp_path, f"iii_engine_version: {bad}")
    assert any("iii_engine_version must be a semver range" in e for e in errors)


@pytest.mark.parametrize("bad", [
    ">=0.2\u0666.0",  # an Arabic-Indic digit after a leading ASCII one
    ">=0.26.0\x1c<0.28.0",  # a separator Python's Unicode \s would accept
    ">=0.26.0\u00a0<0.28.0",  # no-break space
])
def test_non_ascii_range_is_rejected(bad):
    assert not _lib.III_ENGINE_VERSION_RE.fullmatch(bad)


def test_non_string_range_is_rejected(tmp_path):
    # YAML reads an unquoted 0.26 as a float.
    errors = validation_errors(tmp_path, "iii_engine_version: 0.26")
    assert any("iii_engine_version must be a semver range" in e for e in errors)


def test_projection_carries_iii_engine_version(tmp_path):
    entry, _ = smoke_worker(tmp_path, 'iii_engine_version: ">=0.26.0"')
    descriptor = compile_go(tmp_path, entry)
    assert descriptor["registry_projection"]["iii_engine_version"] == ">=0.26.0"
    schema = json.loads((ROOT / ".github/contracts/deployment-descriptor.schema.json").read_text())
    pytest.importorskip("jsonschema").Draft202012Validator(schema).validate(descriptor)


def test_projection_omits_absent_iii_engine_version(tmp_path):
    entry, _ = smoke_worker(tmp_path)
    assert "iii_engine_version" not in compile_go(tmp_path, entry)["registry_projection"]


def test_build_payload_accepts_iii_engine_version():
    projection = {
        "worker_name": "smoke", "type": "binary", "description": "smoke",
        "license": "Apache-2.0", "tags": [], "dependencies": [], "config": None,
        "experimental": False, "readme": "# Smoke\n", "skills": {},
        "iii_engine_version": ">=0.26.0",
    }
    payload = build_payload(
        registry_projection=projection,
        published_version="1.0.0",
        repo_url="https://github.com/iii-hq/workers",
        interface=NON_EMPTY_INTERFACE,
        interface_capture="required",
        artifacts={"kind": "rust-binary", "binaries": {"x86_64-unknown-linux-gnu": {"url": "https://example.test/s.tgz", "sha256": "b" * 64}}},
    )
    assert payload["iii_engine_version"] == ">=0.26.0"
    assert "skills" not in payload
