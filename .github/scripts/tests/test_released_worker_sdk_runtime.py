"""Regression tests for runtime SDK selection in released workers."""
from __future__ import annotations

import json
import tomllib
from pathlib import Path



REPO_ROOT = Path(__file__).resolve().parents[3]


def test_opengantry_uses_current_sdk_release() -> None:
    package = json.loads(
        (REPO_ROOT / "opengantry" / "package.json").read_text(encoding="utf-8")
    )

    assert package["dependencies"]["iii-sdk"] == "0.23.0"


def test_scrapling_uses_current_sdk_release() -> None:
    pyproject = tomllib.loads(
        (REPO_ROOT / "scrapling" / "pyproject.toml").read_text(encoding="utf-8")
    )
    dependencies = pyproject["project"]["dependencies"]

    assert "iii-sdk==0.23.0" in dependencies
    assert "iii-helpers==0.23.0" in dependencies
