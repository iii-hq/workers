"""Tests for registry payload metadata assembled from iii.worker.yaml."""
from pathlib import Path

import pytest
from build_publish_payload import build_payload


def build_binary_payload(tmp_path: Path, manifest: str, **kwargs) -> dict[str, object]:
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
        **kwargs,
    )


def test_manifest_license_is_published(tmp_path: Path) -> None:
    payload = build_binary_payload(
        tmp_path,
        "name: smoke\nlicense: Apache-2.0\n",
    )
    assert payload["license"] == "Apache-2.0"


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


def test_experimental_is_always_sent_as_a_bool(tmp_path: Path) -> None:
    # The registry clears the badge when the key is missing and 422s on a
    # string, so the payload must always carry a real boolean.
    default = build_binary_payload(tmp_path / "default", "name: smoke\n")
    assert default["experimental"] is False

    marked = build_binary_payload(tmp_path / "marked", "name: smoke\n", experimental=True)
    assert marked["experimental"] is True


def test_engine_builtins_are_not_release_targets() -> None:
    """A candidate install can enable an engine-hosted worker mid-boot (harness
    turns on `iii-stream`), which lands it in the workers-baseline diff. Its
    interface is not part of the released worker's surface and must not reach
    the typed-schema gate."""
    from build_publish_payload import _resolve_target_worker_names

    target = _resolve_target_worker_names(
        workers=[
            {"name": "iii-worker-ops"},
            {"name": "harness"},
            {"name": "iii-stream"},
        ],
        worker_name="harness",
        functions=[],
        baseline_workers_json={"workers": [{"name": "iii-worker-ops"}]},
    )
    assert target == {"harness"}


def test_builtin_only_diff_falls_back_to_name_match() -> None:
    """When the baseline diff contains nothing but engine builtins, resolution
    falls through to matching the released worker by name."""
    from build_publish_payload import _resolve_target_worker_names

    target = _resolve_target_worker_names(
        workers=[
            {"name": "iii-worker-ops"},
            {"name": "iii-stream"},
            {"name": "harness", "functions": ["harness::send"]},
        ],
        worker_name="harness",
        functions=[],
        baseline_workers_json={
            "workers": [{"name": "iii-worker-ops"}, {"name": "harness"}]
        },
    )
    assert target == {"harness"}
