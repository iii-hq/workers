#!/usr/bin/env python3
"""Publish immutable Registry versions and CAS deployment channels.

Version publication never assigns a channel implicitly. Mutating requests
retry at most three times, and a timeout or 5xx is followed by exact version
and channel readback before another write.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any, Literal

import _lib


MAX_ATTEMPTS = 3


class RegistryPublicationError(RuntimeError):
    pass


class TransportError(RuntimeError):
    pass


@dataclass(frozen=True)
class Readback:
    state: Literal["equivalent", "absent", "unavailable", "divergent"]
    detail: str


def request_json(
    method: str,
    url: str,
    payload: dict[str, Any] | None = None,
    *,
    api_key: str | None = None,
) -> tuple[int, dict[str, Any]]:
    body = json.dumps(payload, separators=(",", ":")).encode() if payload is not None else None
    headers = {"Accept": "application/json"}
    if payload is not None:
        headers["Content-Type"] = "application/json"
    if api_key:
        headers["X-API-Key"] = api_key
    request = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            raw = response.read().decode()
            return response.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        raw = error.read().decode(errors="replace")
        try:
            response = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            response = {"error": raw or f"HTTP {error.code}"}
        return error.code, response
    except (TimeoutError, urllib.error.URLError, OSError) as error:
        raise TransportError(str(error)) from error
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise TransportError(f"invalid JSON response: {error}") from error


def _json_file(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RegistryPublicationError(f"cannot read JSON payload {path}: {error}") from error
    if not isinstance(value, dict):
        raise RegistryPublicationError(f"JSON payload {path} must be an object")
    return value


def _write_receipt(path: pathlib.Path, value: dict[str, Any]) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _required_string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise RegistryPublicationError(f"{field} must be a non-empty string")
    return value


def _object(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RegistryPublicationError(f"{field} must be an object")
    return value


def _array(value: Any, field: str) -> list[Any]:
    if not isinstance(value, list):
        raise RegistryPublicationError(f"{field} must be an array")
    return value


def _sorted_named_rows(value: Any, field: str) -> list[dict[str, Any]]:
    rows = _array(value, field)
    normalized: list[dict[str, Any]] = []
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            raise RegistryPublicationError(f"{field}[{index}] must be an object")
        _required_string(row.get("name"), f"{field}[{index}].name")
        normalized.append(row)
    return sorted(normalized, key=lambda row: (str(row["name"]), json.dumps(row, sort_keys=True)))


def _dependencies(value: Any, field: str) -> list[dict[str, str]]:
    rows = _array(value, field)
    normalized: list[dict[str, str]] = []
    seen: set[str] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            raise RegistryPublicationError(f"{field}[{index}] must be an object")
        name = _required_string(row.get("name"), f"{field}[{index}].name")
        version = row.get("version")
        if not isinstance(version, str):
            raise RegistryPublicationError(f"{field}[{index}].version must be a string")
        if name in seen:
            raise RegistryPublicationError(f"{field} contains duplicate dependency {name}")
        seen.add(name)
        normalized.append({"name": name, "version": version})
    return sorted(normalized, key=lambda row: (row["name"], row["version"]))


def _dependency_map(value: Any, field: str) -> dict[str, str]:
    return {row["name"]: row["version"] for row in _dependencies(value, field)}


def _validate_publish_payload(payload: dict[str, Any], worker: str, version: str) -> None:
    allowed = {
        "worker_name",
        "version",
        "tag",
        "type",
        "readme",
        "repo",
        "description",
        "license",
        "tags",
        "dependencies",
        "config",
        "functions",
        "triggers",
        "experimental",
        "binaries",
        "image_tag",
        "archive_url",
        "sha256",
    }
    unknown = sorted(set(payload) - allowed)
    if unknown:
        raise RegistryPublicationError(f"publish payload contains unprovable fields: {', '.join(unknown)}")
    if payload.get("worker_name") != worker or payload.get("version") != version:
        raise RegistryPublicationError("publish payload identity does not match requested worker/version")
    if payload.get("type") not in {"binary", "image", "bundle"}:
        raise RegistryPublicationError("publish payload type must be binary, image, or bundle")
    if "tag" in payload:
        _required_string(payload["tag"], "payload.tag")


def prove_publication(api_url: str, worker: str, version: str, payload: dict[str, Any]) -> Readback:
    version_proof = prove_version(api_url, worker, version, payload)
    if version_proof.state != "equivalent":
        return version_proof
    tag = payload.get("tag")
    if tag is None:
        return version_proof
    channel = read_channel(api_url, worker, str(tag))
    if channel.state == "equivalent" and channel.detail == version:
        return Readback("equivalent", f"{version_proof.detail}; raw {tag} pointer equals {version}")
    if channel.state == "equivalent":
        return Readback("divergent", f"channel {tag} points to {channel.detail}, expected {version}")
    return channel


def _expected_detail(payload: dict[str, Any]) -> dict[str, Any]:
    expected = {
        "name": payload["worker_name"],
        "version": payload["version"],
        "type": payload["type"],
        "description": payload.get("description", ""),
        "license": payload.get("license", ""),
        "repo": payload.get("repo", ""),
        "config": payload.get("config") or {},
        "readme": payload.get("readme", ""),
        "dependencies": _dependencies(payload.get("dependencies", []), "payload.dependencies"),
        "functions": _sorted_named_rows(payload.get("functions", []), "payload.functions"),
        "triggers": _sorted_named_rows(payload.get("triggers", []), "payload.triggers"),
        "experimental": payload.get("experimental") is True,
    }
    if "tags" in payload:
        tags = _array(payload["tags"], "payload.tags")
        if not all(isinstance(tag, str) for tag in tags):
            raise RegistryPublicationError("payload.tags must contain only strings")
        expected["tags"] = sorted(tags)
    if payload["type"] == "binary":
        expected["supported_targets"] = sorted(_object(payload.get("binaries"), "payload.binaries"))
    elif payload["type"] == "image":
        expected["image"] = _required_string(payload.get("image_tag"), "payload.image_tag")
    return expected


def _actual_detail(body: dict[str, Any], *, compare_tags: bool) -> dict[str, Any]:
    worker = _object(body.get("worker"), "detail.worker")
    required = {
        "name",
        "version",
        "type",
        "description",
        "license",
        "repo",
        "config",
        "readme",
        "dependencies",
        "functions",
        "triggers",
    }
    missing = sorted(required - set(worker))
    if missing:
        raise RegistryPublicationError(f"detail.worker hides required fields: {', '.join(missing)}")
    actual = {
        "name": worker.get("name"),
        "version": worker.get("version"),
        "type": worker.get("type"),
        "description": worker.get("description", ""),
        "license": worker.get("license", ""),
        "repo": worker.get("repo", ""),
        "config": worker.get("config") or {},
        "readme": worker.get("readme", ""),
        "dependencies": _dependencies(worker.get("dependencies", []), "detail.worker.dependencies"),
        "functions": _sorted_named_rows(worker.get("functions", []), "detail.worker.functions"),
        "triggers": _sorted_named_rows(worker.get("triggers", []), "detail.worker.triggers"),
        "experimental": worker.get("experimental") is True,
    }
    if compare_tags:
        if "tags" not in worker:
            raise RegistryPublicationError("detail.worker hides required field: tags")
        tags = worker.get("tags", [])
        if not isinstance(tags, list) or not all(isinstance(tag, str) for tag in tags):
            raise RegistryPublicationError("detail.worker.tags must be an array of strings")
        actual["tags"] = sorted(tags)
    if worker.get("type") == "binary":
        targets = worker.get("supported_targets")
        if not isinstance(targets, list) or not all(isinstance(target, str) for target in targets):
            raise RegistryPublicationError("detail.worker.supported_targets must be an array of strings")
        actual["supported_targets"] = sorted(targets)
    elif worker.get("type") == "image":
        actual["image"] = worker.get("image")
    return actual


def _expected_resolved(payload: dict[str, Any]) -> dict[str, Any]:
    expected: dict[str, Any] = {
        "name": payload["worker_name"],
        "version": payload["version"],
        "type": payload["type"],
        "repo": payload.get("repo", ""),
        "config": payload.get("config") or {},
        "dependencies": _dependency_map(payload.get("dependencies", []), "payload.dependencies"),
    }
    if payload["type"] == "binary":
        expected["binaries"] = _object(payload.get("binaries"), "payload.binaries")
    elif payload["type"] == "image":
        expected["image"] = _required_string(payload.get("image_tag"), "payload.image_tag")
    elif payload["type"] == "bundle":
        expected["archive_url"] = _required_string(payload.get("archive_url"), "payload.archive_url")
        expected["sha256"] = _required_string(payload.get("sha256"), "payload.sha256")
    else:
        raise RegistryPublicationError(f"unsupported payload type {payload['type']!r}")
    return expected


def _actual_resolved(body: dict[str, Any], worker: str, version: str) -> dict[str, Any]:
    root = _object(body.get("root"), "resolve.root")
    if root.get("name") != worker or root.get("version") != version:
        raise RegistryPublicationError("resolve.root does not match the exact worker/version")
    graph = _array(body.get("graph"), "resolve.graph")
    matches = [node for node in graph if isinstance(node, dict) and node.get("name") == worker]
    if len(matches) != 1 or matches[0].get("version") != version:
        raise RegistryPublicationError("resolve.graph has no unique exact root node")
    node = matches[0]
    actual: dict[str, Any] = {
        "name": node.get("name"),
        "version": node.get("version"),
        "type": node.get("type"),
        "repo": node.get("repo"),
        "config": node.get("config"),
        "dependencies": node.get("dependencies"),
    }
    kind = node.get("type")
    if kind == "binary":
        actual["binaries"] = node.get("binaries")
    elif kind == "image":
        actual["image"] = node.get("image")
    elif kind == "bundle":
        actual["archive_url"] = node.get("archive_url")
        actual["sha256"] = node.get("sha256")
    return actual


def _difference(expected: dict[str, Any], actual: dict[str, Any]) -> str | None:
    for field in sorted(expected):
        if field not in actual:
            return f"field {field} is unavailable"
        if actual[field] != expected[field]:
            return (
                f"field {field} differs: expected "
                f"{json.dumps(expected[field], sort_keys=True)}, got {json.dumps(actual[field], sort_keys=True)}"
            )
    return None


def prove_version(api_url: str, worker: str, version: str, payload: dict[str, Any]) -> Readback:
    encoded_worker = urllib.parse.quote(worker, safe="")
    encoded_version = urllib.parse.quote(version, safe="")
    try:
        detail_status, detail_body = request_json(
            "GET", f"{api_url.rstrip('/')}/w/{encoded_worker}?version={encoded_version}"
        )
        resolve_status, resolve_body = request_json(
            "POST", f"{api_url.rstrip('/')}/resolve", {"worker": worker, "version": version}
        )
    except TransportError as error:
        return Readback("unavailable", f"version readback transport failed: {error}")

    resolve_error = resolve_body.get("error")
    resolve_error_code = resolve_error.get("code") if isinstance(resolve_error, dict) else None
    resolve_absent = resolve_status == 404 or (
        resolve_status == 422 and resolve_error_code in {"version_not_found", "worker_not_found"}
    )
    if detail_status == 404 and resolve_absent:
        return Readback("absent", "exact version is absent from both read surfaces")
    if detail_status != 200 or resolve_status != 200:
        return Readback(
            "unavailable",
            f"exact version readback returned detail={detail_status}, resolve={resolve_status}",
        )

    try:
        detail_difference = _difference(
            _expected_detail(payload),
            _actual_detail(detail_body, compare_tags="tags" in payload),
        )
        if detail_difference:
            return Readback("divergent", f"detail {detail_difference}")
        resolve_difference = _difference(
            _expected_resolved(payload),
            _actual_resolved(resolve_body, worker, version),
        )
        if resolve_difference:
            return Readback("divergent", f"resolve {resolve_difference}")
    except RegistryPublicationError as error:
        return Readback("divergent", str(error))
    return Readback("equivalent", "detail and exact resolved root match the publish payload")


def _retry_delay(attempt: int) -> None:
    time.sleep(attempt)


def _target_version(version: str) -> str:
    try:
        return _lib.validate_deployment_target_version(version)
    except ValueError as error:
        raise RegistryPublicationError(str(error)) from error


def publish_version(
    api_url: str,
    api_key: str,
    worker: str,
    version: str,
    payload: dict[str, Any],
) -> dict[str, Any]:
    _target_version(version)
    _validate_publish_payload(payload, worker, version)
    last = "publication did not run"
    for attempt in range(1, MAX_ATTEMPTS + 1):
        transport_error: str | None = None
        try:
            status, body = request_json("POST", f"{api_url.rstrip('/')}/publish", payload, api_key=api_key)
        except TransportError as error:
            status, body = 0, {}
            transport_error = str(error)

        if status == 200:
            response_version = _object(body.get("version"), "publish response.version")
            if response_version.get("version") != version:
                raise RegistryPublicationError("publish response returned a different version")
        elif status == 409:
            proof = prove_publication(api_url, worker, version, payload)
            if proof.state == "equivalent":
                return {"state": "unchanged", "attempt": attempt, "proof": proof.detail}
            raise RegistryPublicationError(f"409 duplicate publish is not provably equivalent: {proof.detail}")
        elif status != 0 and not 500 <= status <= 599:
            raise RegistryPublicationError(f"publish failed with HTTP {status}: {json.dumps(body, sort_keys=True)}")

        proof = prove_publication(api_url, worker, version, payload)
        if proof.state == "equivalent":
            return {
                "state": "changed" if status == 200 else "recovered",
                "attempt": attempt,
                "proof": proof.detail,
            }
        if proof.state == "divergent":
            raise RegistryPublicationError(f"published version diverges from the payload: {proof.detail}")
        if proof.state == "unavailable":
            raise RegistryPublicationError(f"published version cannot be proved: {proof.detail}")
        last = proof.detail if transport_error is None else f"{transport_error}; {proof.detail}"
        if attempt < MAX_ATTEMPTS:
            _retry_delay(attempt)
    raise RegistryPublicationError(f"publish was not verified after {MAX_ATTEMPTS} attempts: {last}")


def _skills_map(payload: dict[str, Any], version: str) -> dict[str, str]:
    if set(payload) != {"version", "skills"} or payload.get("version") != version:
        raise RegistryPublicationError("skills payload must contain only the exact version and full skills snapshot")
    skills = _object(payload.get("skills"), "skills payload.skills")
    if not all(isinstance(path, str) and isinstance(content, str) and content for path, content in skills.items()):
        raise RegistryPublicationError("skills payload must map paths to non-empty strings")
    return skills


def read_skills(api_url: str, worker: str, version: str, expected: dict[str, str]) -> Readback:
    encoded_worker = urllib.parse.quote(worker, safe="")
    encoded_version = urllib.parse.quote(version, safe="")
    try:
        status, body = request_json(
            "GET", f"{api_url.rstrip('/')}/w/{encoded_worker}/skills?version={encoded_version}"
        )
    except TransportError as error:
        return Readback("unavailable", f"skills readback transport failed: {error}")
    if status == 404:
        return Readback("absent", "skills snapshot or exact version is absent")
    if status != 200:
        return Readback("unavailable", f"skills readback returned HTTP {status}")
    if body.get("name") != worker or body.get("version") != version:
        return Readback("divergent", "skills readback identity differs")
    rows = body.get("skills")
    if not isinstance(rows, list):
        return Readback("divergent", "skills readback has no skills array")
    actual: dict[str, str] = {}
    for index, row in enumerate(rows):
        if (
            not isinstance(row, dict)
            or not isinstance(row.get("path"), str)
            or not isinstance(row.get("content"), str)
        ):
            return Readback("divergent", f"skills[{index}] is malformed")
        if row["path"] in actual:
            return Readback("divergent", f"skills readback repeats path {row['path']}")
        actual[row["path"]] = row["content"]
    if actual != expected:
        return Readback("divergent", "skills readback does not match the full requested snapshot")
    return Readback("equivalent", "exact skills snapshot matches")


def publish_skills(
    api_url: str,
    api_key: str,
    worker: str,
    version: str,
    payload: dict[str, Any],
) -> dict[str, Any]:
    _target_version(version)
    expected = _skills_map(payload, version)
    last = "skills publication did not run"
    encoded_worker = urllib.parse.quote(worker, safe="")
    for attempt in range(1, MAX_ATTEMPTS + 1):
        transport_error: str | None = None
        try:
            status, body = request_json(
                "POST",
                f"{api_url.rstrip('/')}/w/{encoded_worker}/skills",
                payload,
                api_key=api_key,
            )
        except TransportError as error:
            status, body = 0, {}
            transport_error = str(error)
        if status == 200 and body.get("version") != version:
            raise RegistryPublicationError("skills response returned a different version")
        if status != 200 and status != 0 and not 500 <= status <= 599:
            raise RegistryPublicationError(
                f"skills publish failed with HTTP {status}: {json.dumps(body, sort_keys=True)}"
            )

        proof = read_skills(api_url, worker, version, expected)
        if proof.state == "equivalent":
            return {
                "state": "changed" if status == 200 else "recovered",
                "attempt": attempt,
                "proof": proof.detail,
                "skills": len(expected),
            }
        if proof.state == "divergent":
            raise RegistryPublicationError(f"skills snapshot diverges: {proof.detail}")
        if proof.state == "unavailable":
            raise RegistryPublicationError(f"skills snapshot cannot be proved: {proof.detail}")
        last = proof.detail if transport_error is None else f"{transport_error}; {proof.detail}"
        if attempt < MAX_ATTEMPTS:
            _retry_delay(attempt)
    raise RegistryPublicationError(f"skills were not verified after {MAX_ATTEMPTS} attempts: {last}")


def read_channel(api_url: str, worker: str, tag: str) -> Readback:
    encoded_worker = urllib.parse.quote(worker, safe="")
    try:
        status, body = request_json("GET", f"{api_url.rstrip('/')}/w/{encoded_worker}/versions")
    except TransportError as error:
        return Readback("unavailable", f"channel readback transport failed: {error}")
    if status == 404:
        return Readback("absent", "worker has no versions")
    if status != 200:
        return Readback("unavailable", f"channel readback returned HTTP {status}")
    versions = body.get("versions")
    if not isinstance(versions, list):
        return Readback("divergent", "versions readback has no versions array")
    targets: list[str] = []
    for index, entry in enumerate(versions):
        if not isinstance(entry, dict) or not isinstance(entry.get("version"), str):
            return Readback("divergent", f"versions[{index}] is malformed")
        tags = entry.get("tags")
        if not isinstance(tags, list) or not all(isinstance(value, str) for value in tags):
            return Readback("divergent", f"versions[{index}].tags is unavailable")
        if tag in tags:
            targets.append(entry["version"])
    if len(targets) > 1:
        return Readback("divergent", f"channel {tag} points to multiple versions")
    if not targets:
        return Readback("absent", f"channel {tag} is unassigned")
    return Readback("equivalent", targets[0])


def assign_channel(
    api_url: str,
    api_key: str,
    worker: str,
    version: str,
    tag: str,
    expected_current_version: str,
) -> dict[str, Any]:
    _target_version(version)
    if tag not in {"next", "latest"}:
        raise RegistryPublicationError("deployment channel must be next or latest")
    if not expected_current_version:
        raise RegistryPublicationError("expected_current_version is required for channel CAS")
    expected = expected_current_version

    payload: dict[str, Any] = {"version": version}
    payload["expected_current_version"] = None if expected == "none" else expected
    encoded_worker = urllib.parse.quote(worker, safe="")
    encoded_tag = urllib.parse.quote(tag, safe="")
    last = "channel assignment did not run"
    for attempt in range(1, MAX_ATTEMPTS + 1):
        transport_error: str | None = None
        try:
            status, body = request_json(
                "PUT",
                f"{api_url.rstrip('/')}/w/{encoded_worker}/tags/{encoded_tag}",
                payload,
                api_key=api_key,
            )
        except TransportError as error:
            status, body = 0, {}
            transport_error = str(error)
        if status == 200:
            response_tag = body.get("tag")
            if not isinstance(response_tag, dict) or response_tag.get("version") != version:
                raise RegistryPublicationError("channel response returned a different version")
        elif status == 409:
            current = read_channel(api_url, worker, tag)
            if current.state == "equivalent" and current.detail == version:
                return {
                    "state": "unchanged",
                    "attempt": attempt,
                    "proof": f"raw {tag} pointer already equals {version}",
                    "expected_previous": expected,
                }
            raise RegistryPublicationError(f"channel CAS conflict; current pointer: {current.detail}")
        elif status != 0 and not 500 <= status <= 599:
            raise RegistryPublicationError(
                f"channel assignment failed with HTTP {status}: {json.dumps(body, sort_keys=True)}"
            )

        current = read_channel(api_url, worker, tag)
        if current.state == "equivalent" and current.detail == version:
            changed = body.get("changed") if status == 200 else None
            return {
                "state": "unchanged" if changed is False else ("changed" if status == 200 else "recovered"),
                "attempt": attempt,
                "proof": f"raw {tag} pointer equals {version}",
                "expected_previous": expected,
            }
        if current.state == "equivalent" and current.detail != version:
            raise RegistryPublicationError(f"channel {tag} points to {current.detail}, expected {version}")
        if current.state == "divergent":
            raise RegistryPublicationError(current.detail)
        if current.state == "unavailable":
            raise RegistryPublicationError(f"channel pointer cannot be proved: {current.detail}")
        last = current.detail if transport_error is None else f"{transport_error}; {current.detail}"
        if attempt < MAX_ATTEMPTS:
            _retry_delay(attempt)
    raise RegistryPublicationError(f"channel was not verified after {MAX_ATTEMPTS} attempts: {last}")


def advance_next_floor(
    api_url: str,
    api_key: str,
    worker: str,
    version: str,
    expected_next_version: str,
) -> dict[str, Any]:
    """Keep next at or ahead of a latest target without ever moving it backwards."""
    if not expected_next_version:
        raise RegistryPublicationError("expected_next_version is required for latest publication")
    _target_version(version)
    observed = read_channel(api_url, worker, "next")
    if observed.state == "equivalent":
        current = observed.detail
    elif observed.state == "absent":
        current = "none"
    else:
        raise RegistryPublicationError(f"cannot prove current next pointer: {observed.detail}")

    if current == version:
        return {"state": "unchanged", "next": current, "proof": "next already equals latest target"}
    if current != expected_next_version:
        raise RegistryPublicationError(
            f"next changed outside the authorized plan: current={current}, expected={expected_next_version}"
        )
    if current != "none" and _lib.parse_semver(current) > _lib.parse_semver(version):
        return {
            "state": "unchanged",
            "next": current,
            "proof": f"next {current} is already ahead of latest target {version}",
        }
    result = assign_channel(api_url, api_key, worker, version, "next", expected_next_version)
    return {**result, "next": version}


def finalize_release(
    api_url: str,
    api_key: str,
    worker: str,
    candidate_version: str,
    stable_version: str,
    expected_latest_version: str,
) -> dict[str, Any]:
    """Atomically point next and latest at the published stable version.

    Uses the Registry's transactional finalize primitive: every precondition
    is checked under the worker lock server-side, so a rejected call leaves
    both channel pointers untouched. Readback after every attempt proves the
    multi-tag move on the raw pointer surface, never trusting the response.
    """
    try:
        _lib.validate_finalization(candidate_version, stable_version)
    except ValueError as error:
        raise RegistryPublicationError(str(error)) from error
    if not expected_latest_version:
        raise RegistryPublicationError("expected_latest_version is required for release finalization")

    payload: dict[str, Any] = {
        "candidate_version": candidate_version,
        "stable_version": stable_version,
        "expected_latest_version": None if expected_latest_version == "none" else expected_latest_version,
    }
    encoded_worker = urllib.parse.quote(worker, safe="")

    def both_channels_settled() -> Readback:
        next_pointer = read_channel(api_url, worker, "next")
        latest_pointer = read_channel(api_url, worker, "latest")
        for tag, pointer in (("next", next_pointer), ("latest", latest_pointer)):
            if pointer.state in {"divergent", "unavailable"}:
                return Readback(pointer.state, f"channel {tag}: {pointer.detail}")
        if (
            next_pointer.state == "equivalent"
            and latest_pointer.state == "equivalent"
            and next_pointer.detail == latest_pointer.detail == stable_version
        ):
            return Readback("equivalent", f"raw next and latest pointers equal {stable_version}")
        return Readback(
            "absent",
            f"next points to {next_pointer.detail}, latest points to {latest_pointer.detail}",
        )

    last = "release finalization did not run"
    for attempt in range(1, MAX_ATTEMPTS + 1):
        transport_error: str | None = None
        try:
            status, body = request_json(
                "POST",
                f"{api_url.rstrip('/')}/w/{encoded_worker}/releases/finalize",
                payload,
                api_key=api_key,
            )
        except TransportError as error:
            status, body = 0, {}
            transport_error = str(error)
        if status == 200:
            finalize = body.get("finalize")
            if not isinstance(finalize, dict) or finalize.get("stable_version") != stable_version:
                raise RegistryPublicationError("finalize response returned a different stable version")
        elif status == 409:
            current = both_channels_settled()
            if current.state == "equivalent":
                return {
                    "state": "unchanged",
                    "attempt": attempt,
                    "proof": current.detail,
                    "candidate": candidate_version,
                    "expected_latest": expected_latest_version,
                }
            raise RegistryPublicationError(
                f"finalize CAS conflict: {json.dumps(body, sort_keys=True)}; {current.detail}"
            )
        elif status != 0 and not 500 <= status <= 599:
            raise RegistryPublicationError(
                f"release finalization failed with HTTP {status}: {json.dumps(body, sort_keys=True)}"
            )

        current = both_channels_settled()
        if current.state == "equivalent":
            changed = body.get("changed") if status == 200 else None
            return {
                "state": "unchanged" if changed is False else ("changed" if status == 200 else "recovered"),
                "attempt": attempt,
                "proof": current.detail,
                "candidate": candidate_version,
                "expected_latest": expected_latest_version,
            }
        if current.state in {"divergent", "unavailable"}:
            raise RegistryPublicationError(f"finalized channels cannot be proved: {current.detail}")
        last = current.detail if transport_error is None else f"{transport_error}; {current.detail}"
        if attempt < MAX_ATTEMPTS:
            _retry_delay(attempt)
    raise RegistryPublicationError(f"finalization was not verified after {MAX_ATTEMPTS} attempts: {last}")


def _api_key() -> str:
    value = os.environ.get("WORKERS_REGISTRY_API_KEY", "")
    if not value:
        raise RegistryPublicationError("WORKERS_REGISTRY_API_KEY is required")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    def common(command: str) -> argparse.ArgumentParser:
        sub = subparsers.add_parser(command)
        sub.add_argument("--api-url", required=True)
        sub.add_argument("--worker", required=True)
        sub.add_argument("--version", required=True)
        sub.add_argument("--out", type=pathlib.Path, required=True)
        return sub

    version_parser = common("publish-version")
    version_parser.add_argument("--payload", type=pathlib.Path, required=True)
    skills_parser = common("publish-skills")
    skills_parser.add_argument("--payload", type=pathlib.Path, required=True)
    channel_parser = common("assign-channel")
    channel_parser.add_argument("--registry-tag", required=True)
    channel_parser.add_argument("--expected-current-version", required=True)
    next_floor_parser = common("advance-next-floor")
    next_floor_parser.add_argument("--expected-next-version", required=True)
    finalize_parser = subparsers.add_parser("finalize-release")
    finalize_parser.add_argument("--api-url", required=True)
    finalize_parser.add_argument("--worker", required=True)
    finalize_parser.add_argument("--candidate-version", required=True)
    finalize_parser.add_argument("--stable-version", required=True)
    finalize_parser.add_argument("--expected-latest-version", required=True)
    finalize_parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()

    try:
        if args.command == "publish-version":
            result = publish_version(args.api_url, _api_key(), args.worker, args.version, _json_file(args.payload))
        elif args.command == "publish-skills":
            result = publish_skills(args.api_url, _api_key(), args.worker, args.version, _json_file(args.payload))
        elif args.command == "assign-channel":
            result = assign_channel(
                args.api_url,
                _api_key(),
                args.worker,
                args.version,
                args.registry_tag,
                args.expected_current_version,
            )
        elif args.command == "advance-next-floor":
            result = advance_next_floor(
                args.api_url,
                _api_key(),
                args.worker,
                args.version,
                args.expected_next_version,
            )
        else:
            result = finalize_release(
                args.api_url,
                _api_key(),
                args.worker,
                args.candidate_version,
                args.stable_version,
                args.expected_latest_version,
            )
        _write_receipt(args.out, result)
        print(json.dumps(result, sort_keys=True))
        return 0
    except RegistryPublicationError as error:
        print(f"::error::{error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
