"""Regression tests for runtime SDK selection in released workers."""
from __future__ import annotations

import json
import tomllib
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]


def test_opengantry_uses_current_sdk_release() -> None:
    package = json.loads(
        (REPO_ROOT / "opengantry" / "package.json").read_text(encoding="utf-8")
    )

    assert package["dependencies"]["iii-sdk"] == "0.22.1-alpha.25"


def test_scrapling_install_does_not_replace_current_sdk_with_vendor_copy() -> None:
    manifest = yaml.safe_load(
        (REPO_ROOT / "scrapling" / "iii.worker.yaml").read_text(encoding="utf-8")
    )
    install = manifest["scripts"]["install"]

    assert "vendor/iii_sdk" not in install
    assert "vendor/iii_helpers" not in install

    pyproject = tomllib.loads(
        (REPO_ROOT / "scrapling" / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert "iii-sdk==0.22.1a25" in pyproject["project"]["dependencies"]
