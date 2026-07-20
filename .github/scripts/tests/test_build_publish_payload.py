"""Tests for registry payload metadata assembled from iii.worker.yaml."""
from pathlib import Path

import pytest
from build_publish_payload import build_payload


def build_binary_payload(tmp_path: Path, manifest: str) -> dict[str, object]:
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir(parents=True)
    (worker_dir / "iii.worker.yaml").write_text(manifest)
    (worker_dir / "README.md").write_text("# smoke\n")
    return build_payload(
        repo_root=tmp_path,
        worker="smoke",
        version="1.0.0",
        registry_tag="latest",
        deploy="binary",
        repo_url="https://github.com/iii-hq/workers",
        interface={"functions": [], "triggers": []},
        binaries={"x86_64-unknown-linux-gnu": {"url": "https://example.test/smoke.tgz"}},
        image_tag="",
    )


def test_manifest_tags_are_normalized_validated_and_optional(tmp_path: Path) -> None:
    payload = build_binary_payload(
        tmp_path,
        "name: smoke\ntags:\n  - ' SQL '\n  - postgres\n  - sql\n  - ' '\n",
    )
    assert payload["tags"] == ["sql", "postgres"]

    without_tags = tmp_path / "without-tags"
    payload = build_binary_payload(without_tags, "name: smoke\n")
    assert "tags" not in payload

    empty_tags = tmp_path / "empty-tags"
    payload = build_binary_payload(empty_tags, "name: smoke\ntags: []\n")
    assert "tags" not in payload

    whitespace_tags = tmp_path / "whitespace-tags"
    payload = build_binary_payload(whitespace_tags, "name: smoke\ntags:\n  - ' '\n")
    assert "tags" not in payload

    scalar = tmp_path / "scalar"
    with pytest.raises(ValueError, match="`tags` must be a list"):
        build_binary_payload(scalar, "name: smoke\ntags: sql\n")

    empty_scalar = tmp_path / "empty-scalar"
    with pytest.raises(ValueError, match="`tags` must be a list"):
        build_binary_payload(empty_scalar, "name: smoke\ntags: ''\n")

    invalid = tmp_path / "invalid"
    with pytest.raises(ValueError, match="tags entries must be strings"):
        build_binary_payload(invalid, "name: smoke\ntags:\n  - sql\n  - 7\n")
