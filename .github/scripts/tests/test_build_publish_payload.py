"""Tests for projecting prepared deployments onto the current Registry API."""
from build_publish_payload import build_payload


def build_binary_payload() -> dict[str, object]:
    projection = {
        "worker_name": "smoke",
        "type": "binary",
        "description": "smoke",
        "license": "Apache-2.0",
        "tags": [],
        "dependencies": [],
        "config": {},
        "experimental": False,
        "readme": "# Smoke\n",
    }
    return build_payload(
        registry_projection=projection,
        published_version="1.0.0-beta",
        repo_url="https://github.com/iii-hq/workers",
        interface={"functions": [], "triggers": []},
        artifacts={"kind": "rust-binary", "binaries": {"x86_64-unknown-linux-gnu": {"url": "https://example.test/smoke.tgz", "sha256": "b" * 64}}},
    )


def test_payload_contains_only_current_registry_contract() -> None:
    payload = build_binary_payload()
    assert set(payload) == {
        "worker_name", "version", "type", "description", "license", "tags",
        "dependencies", "config", "experimental", "readme", "repo",
        "functions", "triggers", "binaries",
    }
    assert payload["license"] == "Apache-2.0"
    assert payload["binaries"]["x86_64-unknown-linux-gnu"]["sha256"] == "b" * 64
    assert "package_descriptor" not in payload
    assert "descriptor_sha256" not in payload
    assert "channel" not in payload
    assert "tag" not in payload
    assert payload["version"] == "1.0.0-beta"


def test_target_version_is_independent_from_manifest_metadata_and_has_no_implicit_channel() -> None:
    payload = build_binary_payload()
    projection = {key: payload[key] for key in (
        "worker_name", "type", "description", "license", "tags", "dependencies",
        "config", "experimental", "readme",
    )}
    target = build_payload(
        registry_projection=projection,
        published_version="2.0.0-alpha",
        repo_url="https://github.com/iii-hq/workers",
        interface={"functions": [], "triggers": []},
        artifacts={"kind": "rust-binary", "binaries": payload["binaries"]},
    )
    assert target["version"] == "2.0.0-alpha"
    assert "tag" not in target


def test_new_publish_target_rejects_legacy_numbered_rc() -> None:
    projection = {
        "worker_name": "smoke", "type": "binary", "description": "smoke",
        "license": "Apache-2.0", "tags": [], "dependencies": [], "config": {},
        "experimental": False, "readme": "# Smoke\n",
    }
    try:
        build_payload(
            registry_projection=projection,
            published_version="2.0.0-rc.4",
            repo_url="https://github.com/iii-hq/workers",
            interface={"functions": [], "triggers": []},
            artifacts={
                "kind": "rust-binary",
                "binaries": {"x86_64-unknown-linux-gnu": {"url": "https://example.test/smoke.tgz", "sha256": "b" * 64}},
            },
        )
    except ValueError as error:
        assert "deployment target version" in str(error)
    else:
        raise AssertionError("legacy numbered rc target was accepted")


def test_engine_builtins_are_not_deployment_targets() -> None:
    """A target install can enable an engine-hosted worker mid-boot (harness
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
