"""Tests for the strict descriptor-native Registry payload."""
from build_publish_payload import build_payload


def build_binary_payload() -> dict[str, object]:
    package = {
        "name": "smoke",
        "version": "1.0.0",
        "source": {"path": "smoke", "package_manifest": "Cargo.toml"},
        "artifact": {"kind": "rust-binary", "binary": "smoke", "targets": ["x86_64-unknown-linux-gnu"]},
        "runtime": {"exec": ["smoke"]},
        "registry": {"description": "smoke", "license": "Apache-2.0", "tags": [], "dependencies": {}, "publish": True},
        "validation": {"interface": "required"},
    }
    return build_payload(
        package_descriptor=package,
        descriptor_sha256="a" * 64,
        channel="next",
        repo_url="https://github.com/iii-hq/workers",
        interface={"functions": [], "triggers": []},
        artifacts={"kind": "rust-binary", "binaries": {"x86_64-unknown-linux-gnu": {"url": "https://example.test/smoke.tgz", "sha256": "b" * 64}}},
    )


def test_payload_contains_only_strict_registry_contract() -> None:
    payload = build_binary_payload()
    assert set(payload) == {
        "package_descriptor", "descriptor_sha256", "channel", "repo", "interface", "artifacts"
    }
    assert payload["channel"] == "next"
    assert payload["package_descriptor"]["registry"]["license"] == "Apache-2.0"
    assert payload["artifacts"]["kind"] == "rust-binary"


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
        baseline_workers_json={"workers": [{"name": "iii-worker-ops"}]},
    )
    assert target == {"harness"}


def test_builtin_only_diff_uses_exact_worker_name() -> None:
    """When the baseline diff contains only engine builtins, use exact identity."""
    from build_publish_payload import _resolve_target_worker_names

    target = _resolve_target_worker_names(
        workers=[
            {"name": "iii-worker-ops"},
            {"name": "iii-stream"},
            {"name": "harness"},
        ],
        worker_name="harness",
        baseline_workers_json={
            "workers": [{"name": "iii-worker-ops"}, {"name": "harness"}]
        },
    )
    assert target == {"harness"}
