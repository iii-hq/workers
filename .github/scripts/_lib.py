"""Shared helpers for .github/scripts/* CLI tools."""
from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

SemverKey = tuple[tuple[int, ...], int, str]
BumpKind = Literal["patch", "minor", "major"]
ManifestKind = Literal["cargo", "node", "python"]


def parse_semver(v: str) -> SemverKey:
    """Returns a tuple suitable for lexicographic compare.

    Shape: (core_tuple, 1 if stable else 0, pre_suffix).
    The middle int makes stable strictly greater than any pre-release at the
    same core (1.2.3 > 1.2.3-rc.1). The trailing string lexically orders
    multiple pre-releases at the same core (rc.1 < rc.2).
    """
    # Strip build metadata (semver 2.0.0 §10: ignored for precedence).
    v_nobuild, _, _ = v.partition("+")
    core, _, pre = v_nobuild.partition("-")
    parts = [int(x) for x in core.split(".")]
    while len(parts) < 3:
        parts.append(0)
    return (tuple(parts), 0 if pre else 1, pre)


def bump(current: str, kind: BumpKind) -> str:
    """Returns `current` with the requested component incremented.

    Pre-release suffixes are stripped (bumping past a pre-release yields the
    next stable). Short versions are padded to three components.
    """
    core, _, _pre = current.partition("-")
    parts = [int(x) for x in core.split(".")]
    while len(parts) < 3:
        parts.append(0)
    major, minor, patch = parts[0], parts[1], parts[2]
    if kind == "major":
        major, minor, patch = major + 1, 0, 0
    elif kind == "minor":
        minor, patch = minor + 1, 0
    elif kind == "patch":
        patch += 1
    else:
        raise ValueError(f"unknown bump kind: {kind!r}")
    return f"{major}.{minor}.{patch}"


def detect_kind(manifest_path: Path) -> ManifestKind:
    """Identifies a manifest file by its filename."""
    name = manifest_path.name
    if name == "Cargo.toml":
        return "cargo"
    if name == "package.json":
        return "node"
    if name == "pyproject.toml":
        return "python"
    raise ValueError(f"unsupported manifest filename: {name!r}")


def _read_toml_section_version(text: str, section: str) -> str:
    """Read `version = "X"` inside a top-level TOML section like `[package]`.

    Subsections like `[package.metadata]` correctly exit the section because
    matching is strict equality on the bracket-stripped header.
    """
    in_section = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_section = (s == section)
            continue
        if in_section:
            m = re.match(r'^version\s*=\s*"([^"]+)"', s)
            if m:
                return m.group(1)
    raise ValueError(f"no version field in {section} section")


def _write_toml_section_version(path: Path, section: str, new_version: str) -> None:
    """Replace `version = "X"` inside a top-level TOML section (first match)."""
    text = path.read_text()
    out: list[str] = []
    in_section = False
    replaced = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_section = (s == section)
        if in_section and not replaced and re.match(r'^version\s*=\s*"[^"]+"', s):
            line = re.sub(r'^version\s*=\s*"[^"]+"', f'version = "{new_version}"', line)
            replaced = True
        out.append(line)
    if not replaced:
        raise ValueError(f"could not find version in {section}")
    trailing = "\n" if text.endswith("\n") else ""
    path.write_text("\n".join(out) + trailing)


def _read_node_version(text: str) -> str:
    data = json.loads(text)
    if "version" not in data:
        raise ValueError("no version key in package.json")
    return str(data["version"])


def _write_node_version(path: Path, new_version: str) -> None:
    data = json.loads(path.read_text())
    data["version"] = new_version
    path.write_text(json.dumps(data, indent=2) + "\n")


def read_version(manifest_path: Path) -> str:
    """Reads the manifest's package version. Dispatches on filename."""
    kind = detect_kind(manifest_path)
    text = manifest_path.read_text()
    if kind == "cargo":
        return _read_toml_section_version(text, "[package]")
    if kind == "node":
        return _read_node_version(text)
    if kind == "python":
        return _read_toml_section_version(text, "[project]")
    raise ValueError(f"unhandled manifest kind: {kind}")


def write_version(manifest_path: Path, new_version: str) -> None:
    """Writes a new version to the manifest. Dispatches on filename."""
    kind = detect_kind(manifest_path)
    if kind == "cargo":
        _write_toml_section_version(manifest_path, "[package]", new_version)
        return
    if kind == "node":
        _write_node_version(manifest_path, new_version)
        return
    if kind == "python":
        _write_toml_section_version(manifest_path, "[project]", new_version)
        return
    raise ValueError(f"unhandled manifest kind: {kind}")


@dataclass(frozen=True)
class WorkerManifest:
    """Parsed view of a `<worker>/iii.worker.yaml` file."""

    name: str
    language: str | None
    deploy: str | None
    manifest: str | None
    bin: str | None
    raw: dict[str, object]


def read_iii_worker_yaml(worker_dir: Path) -> WorkerManifest:
    """Loads `<worker_dir>/iii.worker.yaml` and returns a `WorkerManifest`."""
    import yaml  # imported here so `_lib` is usable without pyyaml at import time

    path = worker_dir / "iii.worker.yaml"
    if not path.exists():
        raise FileNotFoundError(f"{path} does not exist")
    raw = yaml.safe_load(path.read_text()) or {}
    if not isinstance(raw, dict):
        raise ValueError(f"{path}: expected a mapping at top level")
    name = raw.get("name") or worker_dir.name
    return WorkerManifest(
        name=str(name),
        language=raw.get("language"),
        deploy=raw.get("deploy"),
        manifest=raw.get("manifest"),
        bin=raw.get("bin"),
        raw=raw,
    )


def read_tag_annotation(tag: str) -> dict[str, str]:
    """Parses 'key: value' lines from an annotated tag's body.

    Returns {} on lightweight tags, missing tags, or git failures. Lines that
    start with `#` (subject lines like the release name) or are blank are
    skipped. Only top-level lines containing a single `:` separator count.
    """
    try:
        msg = subprocess.check_output(
            ["git", "tag", "-l", "--format=%(contents)", tag],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        return {}
    out: dict[str, str] = {}
    for line in msg.splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        if ":" not in s:
            continue
        k, _, v = s.partition(":")
        out[k.strip()] = v.strip()
    return out
