"""`min_iii_version` in iii.worker.yaml is validated and carried into the publish payload."""
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


def test_valid_min_iii_version_passes(tmp_path):
    assert validation_errors(tmp_path, "min_iii_version: 0.26.0") == []


def test_two_part_min_iii_version_is_rejected(tmp_path):
    errors = validation_errors(tmp_path, 'min_iii_version: "0.26"')
    assert any("min_iii_version must be MAJOR.MINOR.PATCH" in e for e in errors)


def test_prerelease_min_iii_version_is_rejected(tmp_path):
    errors = validation_errors(tmp_path, 'min_iii_version: "0.26.0-rc.2"')
    assert any("min_iii_version must be MAJOR.MINOR.PATCH" in e for e in errors)


@pytest.mark.parametrize("bad", ['"0.26.0\\n"', '"01.2.3"'])
def test_trailing_newline_and_leading_zero_are_rejected(tmp_path, bad):
    errors = validation_errors(tmp_path, f"min_iii_version: {bad}")
    assert any("min_iii_version must be MAJOR.MINOR.PATCH" in e for e in errors)


def test_non_string_min_iii_version_is_rejected(tmp_path):
    # YAML reads an unquoted 0.26 as a float.
    assert validation_errors(tmp_path, "min_iii_version: 0.26")


def test_projection_carries_min_iii_version(tmp_path):
    entry, _ = smoke_worker(tmp_path, "min_iii_version: 0.26.0")
    descriptor = compile_go(tmp_path, entry)
    assert descriptor["registry_projection"]["min_iii_version"] == "0.26.0"
    schema = json.loads((ROOT / ".github/contracts/deployment-descriptor.schema.json").read_text())
    pytest.importorskip("jsonschema").Draft202012Validator(schema).validate(descriptor)


def test_projection_omits_absent_min_iii_version(tmp_path):
    entry, _ = smoke_worker(tmp_path)
    assert "min_iii_version" not in compile_go(tmp_path, entry)["registry_projection"]


def test_build_payload_accepts_min_iii_version():
    projection = {
        "worker_name": "smoke", "type": "binary", "description": "smoke",
        "license": "Apache-2.0", "tags": [], "dependencies": [], "config": None,
        "experimental": False, "readme": "# Smoke\n", "skills": {},
        "min_iii_version": "0.26.0",
    }
    payload = build_payload(
        registry_projection=projection,
        published_version="1.0.0",
        repo_url="https://github.com/iii-hq/workers",
        interface=NON_EMPTY_INTERFACE,
        interface_capture="required",
        artifacts={"kind": "rust-binary", "binaries": {"x86_64-unknown-linux-gnu": {"url": "https://example.test/s.tgz", "sha256": "b" * 64}}},
    )
    assert payload["min_iii_version"] == "0.26.0"
    assert "skills" not in payload
