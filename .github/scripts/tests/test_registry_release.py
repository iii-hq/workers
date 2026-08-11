from __future__ import annotations

import pytest

import registry_release
from registry_release import RegistryError, promotion_payload, resolved_root_version


def test_promotion_payload_uses_source_and_destination_preconditions():
    assert promotion_payload("1.2.3", "1.2.2") == {
        "version": "1.2.3",
        "expected_tag": "next",
        "expected_current_version": "1.2.2",
    }


def test_first_promotion_omits_missing_latest_precondition():
    assert promotion_payload("1.0.0", None) == {
        "version": "1.0.0",
        "expected_tag": "next",
    }


def test_resolved_root_version_reads_resolver_contract():
    assert resolved_root_version({"root": {"name": "pdf", "version": "0.2.0"}}) == "0.2.0"


def test_resolved_root_version_rejects_malformed_response():
    with pytest.raises(RegistryError, match="root.version"):
        resolved_root_version({"graph": []})


def test_exact_cas_promotes_observed_channels(monkeypatch):
    latest_calls = 0

    def resolve(_api_url, _worker, selector, *, allow_missing=False):
        nonlocal latest_calls
        if selector == "next":
            return "1.2.3"
        latest_calls += 1
        return "1.2.2" if latest_calls == 1 else "1.2.3"

    monkeypatch.setattr(registry_release, "resolve_version", resolve)
    monkeypatch.setattr(
        registry_release,
        "request_json",
        lambda *_args, **_kwargs: (200, {"changed": False}),
    )
    result = registry_release.promote(
        "https://registry.test", "key", "pdf", "1.2.3", "1.2.3", "1.2.2"
    )
    assert result["latest"] == "1.2.3"
    assert result["next"] == "1.2.3"
    assert result["changed"] is False


def test_stale_candidate_is_rejected_before_tag_update(monkeypatch):
    def resolve(_api_url, _worker, selector, *, allow_missing=False):
        assert allow_missing is True
        return "1.2.2" if selector == "latest" else "1.2.4"

    monkeypatch.setattr(registry_release, "resolve_version", resolve)
    called = False

    def request(*_args, **_kwargs):
        nonlocal called
        called = True
        return 200, {}

    monkeypatch.setattr(registry_release, "request_json", request)
    with pytest.raises(RegistryError, match="next points to 1.2.4"):
        registry_release.promote(
            "https://registry.test", "key", "pdf", "1.2.3", "1.2.3", "1.2.2"
        )
    assert called is False


def test_completed_promotion_is_idempotent_for_recovery(monkeypatch):
    def resolve(_api_url, _worker, selector, *, allow_missing=False):
        return "1.2.3"

    monkeypatch.setattr(registry_release, "resolve_version", resolve)

    def request(*_args, **_kwargs):
        raise AssertionError("an already-promoted target must not be written again")

    monkeypatch.setattr(registry_release, "request_json", request)
    result = registry_release.promote(
        "https://registry.test", "key", "pdf", "1.2.3", "1.2.3", "1.2.2"
    )
    assert result == {
        "worker": "pdf",
        "version": "1.2.3",
        "previous_latest": "1.2.3",
        "next": "1.2.3",
        "latest": "1.2.3",
        "changed": False,
        "registry_response": {"changed": False, "idempotent": True},
    }


def test_stale_latest_is_rejected_before_tag_update(monkeypatch):
    def resolve(_api_url, _worker, selector, *, allow_missing=False):
        return "1.2.3" if selector == "next" else "1.2.1"

    monkeypatch.setattr(registry_release, "resolve_version", resolve)

    def request(*_args, **_kwargs):
        raise AssertionError("a stale destination must not be updated")

    monkeypatch.setattr(registry_release, "request_json", request)
    with pytest.raises(RegistryError, match="latest points to 1.2.1"):
        registry_release.promote(
            "https://registry.test", "key", "pdf", "1.2.3", "1.2.3", "1.2.2"
        )
