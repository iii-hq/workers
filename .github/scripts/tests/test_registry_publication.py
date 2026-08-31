from __future__ import annotations

import copy

import pytest

import registry_publication
from registry_publication import RegistryPublicationError, TransportError


API = "https://registry.test"
KEY = "secret"
WORKER = "smoke"
VERSION = "1.2.3"


def payload() -> dict[str, object]:
    return {
        "worker_name": WORKER,
        "version": VERSION,
        "type": "binary",
        "readme": "# Smoke\n",
        "repo": "https://github.com/iii-hq/workers",
        "description": "Smoke worker",
        "license": "Apache-2.0",
        "tags": ["test", "smoke"],
        "dependencies": [{"name": "configuration", "version": "^1.0.0"}],
        "config": {"mode": "safe"},
        "functions": [
            {
                "name": "smoke::run",
                "description": "Run smoke",
                "request_schema": {"type": "object"},
                "response_schema": {"type": "object"},
                "metadata": {},
            }
        ],
        "triggers": [
            {
                "name": "smoke-event",
                "description": "Smoke event",
                "invocation_schema": {"type": "object"},
                "return_schema": {"type": "object"},
                "metadata": {},
            }
        ],
        "experimental": False,
        "binaries": {
            "x86_64-unknown-linux-gnu": {
                "url": "https://example.test/smoke.tar.gz",
                "sha256": "a" * 64,
            }
        },
    }


def detail_body(source: dict[str, object] | None = None) -> dict[str, object]:
    source = source or payload()
    worker: dict[str, object] = {
        "name": source["worker_name"],
        "version": source["version"],
        "type": source["type"],
        "readme": source["readme"],
        "repo": source["repo"],
        "description": source["description"],
        "license": source["license"],
        "tags": list(reversed(source["tags"])),
        "dependencies": source["dependencies"],
        "config": source["config"],
        "functions": source["functions"],
        "triggers": source["triggers"],
        "supported_targets": list(reversed(source["binaries"])),
        "total_downloads": 42,
        "author": {"name": "ignored"},
    }
    return {"worker": worker}


def resolve_body(source: dict[str, object] | None = None) -> dict[str, object]:
    source = source or payload()
    node = {
        "name": source["worker_name"],
        "version": source["version"],
        "type": source["type"],
        "repo": source["repo"],
        "config": source["config"],
        "dependencies": {row["name"]: row["version"] for row in source["dependencies"]},
        "binaries": source["binaries"],
    }
    return {
        "root": {"name": source["worker_name"], "version": source["version"]},
        "graph": [node],
        "edges": [],
    }


def exact_readback(method: str, url: str, _body=None, **_kwargs):
    if method == "GET" and "?version=" in url:
        return 200, detail_body()
    if method == "POST" and url.endswith("/resolve"):
        return 200, resolve_body()
    raise AssertionError((method, url))


def no_sleep(monkeypatch) -> None:
    monkeypatch.setattr(registry_publication, "_retry_delay", lambda _attempt: None)


def test_fresh_publish_requires_exact_detail_and_resolve_readback(monkeypatch) -> None:
    calls: list[tuple[str, str]] = []

    def request(method, url, body=None, **kwargs):
        calls.append((method, url))
        if url.endswith("/publish"):
            assert body == payload()
            assert kwargs["api_key"] == KEY
            return 200, {"version": {"version": VERSION}}
        return exact_readback(method, url, body, **kwargs)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())
    assert result["state"] == "changed"
    assert [method for method, _url in calls] == ["POST", "GET", "POST"]


def test_409_continues_only_when_every_public_field_and_artifact_matches(monkeypatch) -> None:
    def request(method, url, body=None, **kwargs):
        if url.endswith("/publish"):
            return 409, {"error": "exists"}
        return exact_readback(method, url, body, **kwargs)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())
    assert result["state"] == "unchanged"


@pytest.mark.parametrize("failure", ["hidden", "divergent", "artifact"])
def test_409_fails_closed_when_equivalence_cannot_be_proved(monkeypatch, failure: str) -> None:
    def request(method, url, body=None, **_kwargs):
        if url.endswith("/publish"):
            return 409, {"error": "exists"}
        if method == "GET":
            if failure == "hidden":
                return 404, {"error": "not found"}
            detail = detail_body()
            if failure == "divergent":
                detail["worker"]["description"] = "different"
            return 200, detail
        resolved = resolve_body()
        if failure == "artifact":
            resolved["graph"][0]["binaries"]["x86_64-unknown-linux-gnu"]["sha256"] = "b" * 64
        return 200, resolved

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="not provably equivalent"):
        registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())


def test_409_fails_when_detail_hides_license(monkeypatch) -> None:
    def request(method, url, body=None, **kwargs):
        if url.endswith("/publish"):
            return 409, {}
        if method == "GET":
            detail = detail_body()
            del detail["worker"]["license"]
            return 200, detail
        return exact_readback(method, url, body, **kwargs)

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="hides required fields: license"):
        registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())


def test_5xx_reads_back_before_each_of_at_most_three_publish_attempts(monkeypatch) -> None:
    calls: list[str] = []
    no_sleep(monkeypatch)

    def request(method, url, _body=None, **_kwargs):
        calls.append(f"{method} {url.split('/')[-1].split('?')[0]}")
        if url.endswith("/publish"):
            return 503, {"error": "retry"}
        if method == "GET":
            return 404, {"error": {"code": "version_not_found"}}
        return 422, {"error": {"code": "version_not_found"}}

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="after 3 attempts"):
        registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())
    assert calls == [
        "POST publish", "GET smoke", "POST resolve",
        "POST publish", "GET smoke", "POST resolve",
        "POST publish", "GET smoke", "POST resolve",
    ]


def test_worker_not_found_on_both_read_surfaces_is_absent_for_initial_publish(monkeypatch) -> None:
    attempts = 0
    no_sleep(monkeypatch)

    def request(method, url, _body=None, **_kwargs):
        nonlocal attempts
        if url.endswith("/publish"):
            attempts += 1
            return 503, {}
        return 404, {"error": {"code": "worker_not_found"}}

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="after 3 attempts"):
        registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())
    assert attempts == 3


@pytest.mark.parametrize("readback", ["unavailable", "divergent"])
def test_publish_does_not_repeat_when_readback_is_not_absent(monkeypatch, readback: str) -> None:
    attempts = 0

    def request(method, url, _body=None, **_kwargs):
        nonlocal attempts
        if url.endswith("/publish"):
            attempts += 1
            return 503, {}
        if readback == "unavailable":
            return 503, {}
        if method == "GET":
            detail = detail_body()
            detail["worker"]["description"] = "different"
            return 200, detail
        return 200, resolve_body()

    monkeypatch.setattr(registry_publication, "request_json", request)
    error = "cannot be proved" if readback == "unavailable" else "diverges"
    with pytest.raises(RegistryPublicationError, match=error):
        registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())
    assert attempts == 1


def test_timeout_is_recovered_when_version_readback_is_exact(monkeypatch) -> None:
    def request(method, url, body=None, **kwargs):
        if url.endswith("/publish"):
            raise TransportError("timeout")
        return exact_readback(method, url, body, **kwargs)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.publish_version(API, KEY, WORKER, VERSION, payload())
    assert result["state"] == "recovered"
    assert result["attempt"] == 1


def skills_response(skills: dict[str, str]) -> dict[str, object]:
    return {
        "name": WORKER,
        "version": VERSION,
        "skills": [{"path": path, "content": content} for path, content in sorted(skills.items())],
        "prompts": [],
    }


@pytest.mark.parametrize("skills", [{}, {"SKILL.md": "# Smoke\n", "skills/run.md": "Run\n"}])
def test_skills_replace_and_read_back_the_full_snapshot(monkeypatch, skills: dict[str, str]) -> None:
    desired = {"version": VERSION, "skills": skills}
    calls: list[str] = []

    def request(method, url, body=None, **_kwargs):
        calls.append(method)
        if method == "POST":
            assert body == desired
            return 200, {"version": VERSION}
        return 200, skills_response(skills)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.publish_skills(API, KEY, WORKER, VERSION, desired)
    assert result["skills"] == len(skills)
    assert calls == ["POST", "GET"]


def test_skills_5xx_is_recovered_only_after_matching_readback(monkeypatch) -> None:
    desired = {"version": VERSION, "skills": {"SKILL.md": "# Smoke\n"}}

    def request(method, _url, _body=None, **_kwargs):
        if method == "POST":
            return 500, {}
        return 200, skills_response(desired["skills"])

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.publish_skills(API, KEY, WORKER, VERSION, desired)
    assert result["state"] == "recovered"


def test_skills_5xx_repeats_only_while_snapshot_is_absent(monkeypatch) -> None:
    desired = {"version": VERSION, "skills": {}}
    attempts = 0
    no_sleep(monkeypatch)

    def request(method, _url, _body=None, **_kwargs):
        nonlocal attempts
        if method == "POST":
            attempts += 1
            return 503, {}
        return 404, {}

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="after 3 attempts"):
        registry_publication.publish_skills(API, KEY, WORKER, VERSION, desired)
    assert attempts == 3


@pytest.mark.parametrize("readback", ["unavailable", "divergent"])
def test_skills_does_not_repeat_when_readback_is_not_absent(monkeypatch, readback: str) -> None:
    desired = {"version": VERSION, "skills": {"SKILL.md": "# Smoke\n"}}
    attempts = 0

    def request(method, _url, _body=None, **_kwargs):
        nonlocal attempts
        if method == "POST":
            attempts += 1
            return 503, {}
        if readback == "unavailable":
            return 503, {}
        return 200, skills_response({"SKILL.md": "different\n"})

    monkeypatch.setattr(registry_publication, "request_json", request)
    error = "cannot be proved" if readback == "unavailable" else "diverges"
    with pytest.raises(RegistryPublicationError, match=error):
        registry_publication.publish_skills(API, KEY, WORKER, VERSION, desired)
    assert attempts == 1


def versions_response(tag: str, version: str) -> dict[str, object]:
    return {"versions": [{"version": version, "tag": tag, "tags": [tag], "dependencies": []}]}


def test_channel_cas_is_last_state_change_and_uses_raw_pointer_readback(monkeypatch) -> None:
    calls: list[tuple[str, str, object]] = []

    def request(method, url, body=None, **_kwargs):
        calls.append((method, url, copy.deepcopy(body)))
        if method == "PUT":
            assert body == {"version": VERSION, "expected_current_version": "1.2.2"}
            return 200, {"tag": {"version": VERSION}, "changed": True}
        return 200, versions_response("next", VERSION)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.assign_channel(API, KEY, WORKER, VERSION, "next", "1.2.2")
    assert result["state"] == "changed"
    assert [call[0] for call in calls] == ["PUT", "GET"]
    assert calls[1][1].endswith("/w/smoke/versions")


def test_blank_channel_precondition_is_rejected_instead_of_snapshotted() -> None:
    with pytest.raises(RegistryPublicationError, match="expected_current_version is required"):
        registry_publication.assign_channel(API, KEY, WORKER, VERSION, "latest", "")


def test_latest_advances_next_when_target_is_ahead(monkeypatch) -> None:
    monkeypatch.setattr(
        registry_publication,
        "read_channel",
        lambda *_args, **_kwargs: registry_publication.Readback("equivalent", "1.2.2"),
    )
    captured = {}

    def assign(_api, _key, _worker, version, channel, expected):
        captured.update(version=version, channel=channel, expected=expected)
        return {"state": "changed"}

    monkeypatch.setattr(registry_publication, "assign_channel", assign)
    result = registry_publication.advance_next_floor(API, KEY, WORKER, VERSION, "1.2.2")
    assert result["next"] == VERSION
    assert captured == {"version": VERSION, "channel": "next", "expected": "1.2.2"}


def test_latest_never_regresses_next_when_it_is_ahead(monkeypatch) -> None:
    monkeypatch.setattr(
        registry_publication,
        "read_channel",
        lambda *_args, **_kwargs: registry_publication.Readback("equivalent", "1.3.0-rc.1"),
    )
    monkeypatch.setattr(
        registry_publication,
        "assign_channel",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("must not mutate next")),
    )
    result = registry_publication.advance_next_floor(API, KEY, WORKER, VERSION, "1.3.0-rc.1")
    assert result["state"] == "unchanged"
    assert result["next"] == "1.3.0-rc.1"


def test_latest_keeps_same_core_next_when_product_maturity_is_ahead(monkeypatch) -> None:
    monkeypatch.setattr(
        registry_publication,
        "read_channel",
        lambda *_args, **_kwargs: registry_publication.Readback("equivalent", "1.2.3-beta"),
    )
    monkeypatch.setattr(
        registry_publication,
        "assign_channel",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("must not regress next")),
    )
    result = registry_publication.advance_next_floor(
        API, KEY, WORKER, "1.2.3-alpha", "1.2.3-beta"
    )
    assert result["state"] == "unchanged"
    assert result["next"] == "1.2.3-beta"


def test_latest_advances_same_core_next_through_product_maturity(monkeypatch) -> None:
    monkeypatch.setattr(
        registry_publication,
        "read_channel",
        lambda *_args, **_kwargs: registry_publication.Readback("equivalent", "1.2.3-alpha"),
    )
    captured = {}

    def assign(_api, _key, _worker, version, channel, expected):
        captured.update(version=version, channel=channel, expected=expected)
        return {"state": "changed"}

    monkeypatch.setattr(registry_publication, "assign_channel", assign)
    result = registry_publication.advance_next_floor(
        API, KEY, WORKER, "1.2.3-beta", "1.2.3-alpha"
    )
    assert result["next"] == "1.2.3-beta"
    assert captured == {
        "version": "1.2.3-beta", "channel": "next", "expected": "1.2.3-alpha"
    }


def test_latest_rejects_next_drift_from_authorized_plan(monkeypatch) -> None:
    monkeypatch.setattr(
        registry_publication,
        "read_channel",
        lambda *_args, **_kwargs: registry_publication.Readback("equivalent", "1.2.1"),
    )
    with pytest.raises(RegistryPublicationError, match="outside the authorized plan"):
        registry_publication.advance_next_floor(API, KEY, WORKER, VERSION, "1.2.2")


def test_channel_5xx_is_recovered_when_raw_pointer_already_matches(monkeypatch) -> None:
    def request(method, _url, _body=None, **_kwargs):
        if method == "PUT":
            return 503, {}
        return 200, versions_response("next", VERSION)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.assign_channel(API, KEY, WORKER, VERSION, "next", "1.2.2")
    assert result["state"] == "recovered"


def test_channel_5xx_repeats_only_while_raw_pointer_is_absent(monkeypatch) -> None:
    attempts = 0
    no_sleep(monkeypatch)

    def request(method, _url, _body=None, **_kwargs):
        nonlocal attempts
        if method == "PUT":
            attempts += 1
            return 503, {}
        return 200, {"versions": []}

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="after 3 attempts"):
        registry_publication.assign_channel(API, KEY, WORKER, VERSION, "next", "1.2.2")
    assert attempts == 3


def test_channel_409_is_idempotent_when_raw_pointer_already_matches(monkeypatch) -> None:
    def request(method, _url, _body=None, **_kwargs):
        if method == "PUT":
            return 409, {"error": "stale"}
        return 200, versions_response("next", VERSION)

    monkeypatch.setattr(registry_publication, "request_json", request)
    result = registry_publication.assign_channel(API, KEY, WORKER, VERSION, "next", "1.2.2")
    assert result["state"] == "unchanged"


@pytest.mark.parametrize("readback", ["unavailable", "divergent"])
def test_channel_does_not_repeat_when_raw_pointer_is_not_absent(monkeypatch, readback: str) -> None:
    attempts = 0

    def request(method, _url, _body=None, **_kwargs):
        nonlocal attempts
        if method == "PUT":
            attempts += 1
            return 503, {}
        if readback == "unavailable":
            return 503, {}
        return 200, {"versions": "hidden"}

    monkeypatch.setattr(registry_publication, "request_json", request)
    error = "cannot be proved" if readback == "unavailable" else "no versions array"
    with pytest.raises(RegistryPublicationError, match=error):
        registry_publication.assign_channel(API, KEY, WORKER, VERSION, "next", "1.2.2")
    assert attempts == 1


def test_channel_409_fails_even_when_a_readback_is_available(monkeypatch) -> None:
    def request(method, _url, _body=None, **_kwargs):
        if method == "PUT":
            return 409, {"error": "stale"}
        return 200, versions_response("next", "1.2.4")

    monkeypatch.setattr(registry_publication, "request_json", request)
    with pytest.raises(RegistryPublicationError, match="CAS conflict"):
        registry_publication.assign_channel(API, KEY, WORKER, VERSION, "next", "1.2.2")
