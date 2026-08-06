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

RELEASE_VERSION_RE = re.compile(
    r"^(?P<major>0|[1-9][0-9]*)\."
    r"(?P<minor>0|[1-9][0-9]*)\."
    r"(?P<patch>0|[1-9][0-9]*)"
    r"(?:-(?P<maturity>experimental|alpha|beta))?$"
)
RELEASE_MATURITIES = ("experimental", "alpha", "beta", "stable")
RELEASE_SUFFIXES = ("none", "experimental", "alpha", "beta")
_MATURITY_RANK = {name: idx for idx, name in enumerate(RELEASE_MATURITIES)}


@dataclass(frozen=True)
class ReleaseVersion:
    """The deliberately small version grammar supported by worker releases."""

    major: int
    minor: int
    patch: int
    maturity: str

    @property
    def core(self) -> tuple[int, int, int]:
        return (self.major, self.minor, self.patch)

    @property
    def core_text(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}"


def parse_release_version(version: str) -> ReleaseVersion:
    """Parse a worker release version and reject unsupported SemVer shapes.

    Release Control intentionally exposes only the product maturity ladder.
    Build metadata, arbitrary prerelease labels and numbered prereleases are
    excluded so the workflow and UI share one unambiguous contract.
    """
    match = RELEASE_VERSION_RE.fullmatch(version.strip())
    if not match:
        raise ValueError(
            "version must be MAJOR.MINOR.PATCH with an optional "
            "-experimental, -alpha, or -beta suffix"
        )
    return ReleaseVersion(
        major=int(match.group("major")),
        minor=int(match.group("minor")),
        patch=int(match.group("patch")),
        maturity=match.group("maturity") or "stable",
    )


def release_maturity(version: str) -> str:
    return parse_release_version(version).maturity


def validate_release_transition(current: str, target: str) -> None:
    """Require monotonic cores and forward-only maturity at the same core.

    Equality is allowed: `bump=none` is the supported way to tag a version
    that a merged change already wrote into its manifest. Tag availability is
    checked separately by create-tag.yml.
    """
    before = parse_release_version(current)
    after = parse_release_version(target)
    if after.core < before.core:
        raise ValueError(f"version core cannot move backwards: {current} -> {target}")
    if after.core == before.core and target != current and before.maturity != "stable":
        if _MATURITY_RANK[after.maturity] <= _MATURITY_RANK[before.maturity]:
            raise ValueError(f"maturity cannot repeat or move backwards: {current} -> {target}")


def list_tagged_versions(worker: str) -> list[str]:
    """Return versions from `<worker>/v<version>` tags in the local checkout."""
    prefix = f"{worker}/v"
    try:
        output = subprocess.check_output(
            ["git", "tag", "--list", f"{prefix}*"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []
    return [line[len(prefix):] for line in output.splitlines() if line.startswith(prefix)]


def validate_release_history(target: str, existing: list[str]) -> None:
    """Reject a target behind an already tagged core or maturity.

    Legacy tags outside the v2 grammar do not participate in the maturity
    ladder. Exact equality is left to the workflow's idempotency check.
    """
    wanted = parse_release_version(target)
    parsed: list[ReleaseVersion] = []
    for version in existing:
        try:
            parsed.append(parse_release_version(version))
        except ValueError:
            continue
    if not parsed:
        return
    highest_core = max(version.core for version in parsed)
    if wanted.core < highest_core:
        raise ValueError(
            f"version core {wanted.core_text} is behind existing "
            f"{'.'.join(str(part) for part in highest_core)}"
        )
    for version in parsed:
        if version.core != wanted.core:
            continue
        same_version = version.maturity == wanted.maturity
        if not same_version and _MATURITY_RANK[version.maturity] >= _MATURITY_RANK[wanted.maturity]:
            raise ValueError(
                f"maturity {wanted.maturity} cannot follow {version.maturity} "
                f"for {wanted.core_text}"
            )


def resolve_release_version(current: str, kind: str, suffix: str, target: str = "") -> str:
    """Resolve exact intent or a human-friendly bump/suffix combination."""
    parsed = parse_release_version(current)
    if target:
        resolved = target.strip()
    else:
        if suffix not in RELEASE_SUFFIXES:
            raise ValueError(f"unknown release suffix: {suffix!r}")
        if kind == "none":
            core = parsed.core_text
        elif kind in ("patch", "minor", "major"):
            core = bump(current, kind)
        else:
            raise ValueError(f"unknown bump kind: {kind!r}")
        resolved = core if suffix == "none" else f"{core}-{suffix}"
    validate_release_transition(current, resolved)
    return resolved


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


def read_cargo_package_name(manifest_path: Path) -> str:
    """Reads `name = "X"` from a Cargo.toml `[package]` section."""
    in_section = False
    for line in manifest_path.read_text().splitlines():
        s = line.strip()
        if s.startswith("["):
            in_section = (s == "[package]")
            continue
        if in_section:
            m = re.match(r'^name\s*=\s*"([^"]+)"', s)
            if m:
                return m.group(1)
    raise ValueError(f"no name field in [package] section of {manifest_path}")


def sync_cargo_lock_self_version(lock_path: Path, name: str, new_version: str) -> bool:
    """Update a worker's own `[[package]]` version in its Cargo.lock to match
    its bumped Cargo.toml.

    Bumping only rewrites Cargo.toml, so the crate's own entry in Cargo.lock
    keeps the old version until the next local `cargo build` rewrites it —
    leaving every developer with a dirty lockfile. Rewriting the matching block
    here, at tag time, keeps the committed lock in sync. Only the crate's own
    version line changes; the dependency graph is untouched (a bump never alters
    resolution).

    Returns True if the version was changed, False if it already matched.
    Raises ValueError if no `[[package]]` block named `name` exists.
    """
    text = lock_path.read_text()
    out: list[str] = []
    in_target = False
    found = replaced = False
    for line in text.splitlines():
        s = line.strip()
        if s == "[[package]]":
            in_target = False
        else:
            m = re.match(r'^name = "([^"]+)"$', s)
            if m:
                in_target = (m.group(1) == name)
                found = found or in_target
            elif in_target and re.match(r'^version = "[^"]+"$', s):
                new_line = f'version = "{new_version}"'
                replaced = replaced or (new_line != s)
                line = new_line
                in_target = False
        out.append(line)
    if not found:
        raise ValueError(f"package {name!r} not found in {lock_path}")
    if replaced:
        trailing = "\n" if text.endswith("\n") else ""
        lock_path.write_text("\n".join(out) + trailing)
    return replaced


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
