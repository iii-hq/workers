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
    r"(?:-(?:(?P<rc>rc)\.(?P<rc_number>[1-9][0-9]*)|(?P<maturity>experimental|alpha|beta)))?$"
)
RELEASE_MATURITIES = ("experimental", "alpha", "beta", "rc", "stable")
RELEASE_SUFFIXES = ("none", "experimental", "alpha", "beta")
_MATURITY_RANK = {name: idx for idx, name in enumerate(RELEASE_MATURITIES)}


@dataclass(frozen=True)
class ReleaseVersion:
    """The deliberately small version grammar supported by worker releases."""

    major: int
    minor: int
    patch: int
    maturity: str
    rc: int | None = None

    @property
    def core(self) -> tuple[int, int, int]:
        return (self.major, self.minor, self.patch)

    @property
    def core_text(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}"


def parse_release_version(version: str) -> ReleaseVersion:
    """Parse a worker release version and reject unsupported SemVer shapes.

    Release Control intentionally exposes only the product maturity ladder.
    Build metadata and arbitrary prerelease labels are excluded; numbered RCs
    are represented explicitly so the workflow and UI share one unambiguous
    contract.
    """
    match = RELEASE_VERSION_RE.fullmatch(version.strip())
    if not match:
        raise ValueError(
            "version must be MAJOR.MINOR.PATCH with an optional "
            "-rc.N, -experimental, -alpha, or -beta suffix"
        )
    return ReleaseVersion(
        major=int(match.group("major")),
        minor=int(match.group("minor")),
        patch=int(match.group("patch")),
        maturity=("rc" if match.group("rc") else match.group("maturity") or "stable"),
        rc=int(match.group("rc_number")) if match.group("rc_number") else None,
    )


def release_maturity(version: str) -> str:
    return parse_release_version(version).maturity


def validate_release_transition(current: str, target: str) -> None:
    """Require monotonic cores and forward-only maturity at the same core.

    Equality is allowed: `bump=none` is the supported way to tag a version
    that a merged change already wrote into its package manifest. Immutable tag
    availability is checked by the authenticated release executor effect probe.
    """
    before = parse_release_version(current)
    after = parse_release_version(target)
    if after.core < before.core:
        raise ValueError(f"version core cannot move backwards: {current} -> {target}")
    if after.core == before.core and target != current:
        # A manifest at an unreleased stable core is the bootstrap point for
        # its first candidate. Once a prerelease exists, movement is forward
        # only through the maturity ladder and RC counter.
        if before.maturity != "stable" and _MATURITY_RANK[after.maturity] < _MATURITY_RANK[before.maturity]:
            raise ValueError(f"maturity cannot repeat or move backwards: {current} -> {target}")
        if after.maturity == before.maturity == "rc" and (after.rc or 0) <= (before.rc or 0):
            raise ValueError(f"rc number cannot repeat or move backwards: {current} -> {target}")


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

    Tags outside the strict release grammar do not participate in the maturity
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
        if same_version and version.maturity == "rc" and (version.rc or 0) >= (wanted.rc or 0):
            raise ValueError(f"rc {wanted.rc} cannot follow rc {version.rc} for {wanted.core_text}")
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
    if not pre:
        return (tuple(parts), 1, "")
    if pre.startswith("rc.") and pre[3:].isdigit():
        return (tuple(parts), 0, f"rc.{int(pre[3:]):020d}")
    return (tuple(parts), 0, pre)


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
class WorkerSpec:
    """One entry from the private release catalog.

    ``runtime`` and ``registry`` are compatibility projections derived from
    the public manifest; they are never stored in ``.deploy/workers.yaml``.
    """

    catalog_id: str
    path: Path
    publish: bool
    manifest: str | None
    artifact_kind: str
    binary: str | None
    source: dict[str, object]
    artifact: dict[str, object]
    runtime: dict[str, object]
    registry: dict[str, object]
    validation: dict[str, object]
    raw: dict[str, object]


@dataclass(frozen=True)
class WorkerManifest:
    """Parsed view of the public `<worker>/iii.worker.yaml` contract.

    Release workflows must use :class:`WorkerSpec` instead. This reader exists
    for normal CI, scaffolding and the `iii worker` development surface.
    """

    name: str
    language: str | None
    deploy: str | None
    manifest: str | None
    bin: str | None
    raw: dict[str, object]


def read_iii_worker_yaml(worker_dir: Path) -> WorkerManifest:
    """Load a public worker manifest without making it a release input."""
    import yaml

    path = worker_dir / "iii.worker.yaml"
    if not path.is_file():
        raise FileNotFoundError(f"{path} does not exist")
    raw = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
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


def worker_catalog_path() -> Path:
    return Path(__file__).resolve().parents[2] / ".deploy" / "workers.yaml"


def read_worker_catalog(path: Path | None = None) -> dict[str, WorkerSpec]:
    """Load private build metadata and join its public manifest projection."""
    import yaml  # imported lazily so version-only helpers need no PyYAML

    catalog_path = (path or worker_catalog_path()).resolve()
    if not catalog_path.is_file():
        raise FileNotFoundError(f"{catalog_path} does not exist")
    document = yaml.safe_load(catalog_path.read_text(encoding="utf-8")) or {}
    if not isinstance(document, dict):
        raise ValueError(f"{catalog_path}: expected a mapping at top level")
    unknown = set(document) - {"workers"}
    if unknown:
        raise ValueError(f"{catalog_path}: unknown top-level keys: {sorted(unknown)}")
    workers = document.get("workers")
    if not isinstance(workers, dict) or not workers:
        raise ValueError(f"{catalog_path}: workers must be a non-empty mapping")

    root = catalog_path.parent.parent if catalog_path.parent.name == ".deploy" else catalog_path.parent
    parsed: dict[str, WorkerSpec] = {}
    paths: set[Path] = set()
    publish_names: set[str] = set()
    for catalog_id, value in workers.items():
        if not isinstance(catalog_id, str) or not catalog_id.strip():
            raise ValueError(f"{catalog_path}: worker IDs must be non-empty strings")
        if not isinstance(value, dict):
            raise ValueError(f"{catalog_path}: workers.{catalog_id} must be a mapping")
        raw = dict(value)
        expected_sections = {"source", "artifact", "publish"}
        legacy = {"language", "deploy", "manifest", "bin", "scripts", "interface_smoke"} & set(raw)
        if legacy:
            raise ValueError(f"{catalog_path}: workers.{catalog_id} uses legacy fields: {sorted(legacy)}")
        unknown_sections = set(raw) - expected_sections
        missing_sections = expected_sections - set(raw)
        if unknown_sections or missing_sections:
            raise ValueError(
                f"{catalog_path}: workers.{catalog_id} sections differ; "
                f"missing={sorted(missing_sections)} unknown={sorted(unknown_sections)}"
            )
        source = raw["source"]
        artifact = raw["artifact"]
        for section_name, section in (
            ("source", source),
            ("artifact", artifact),
        ):
            if not isinstance(section, dict):
                raise ValueError(f"{catalog_path}: workers.{catalog_id}.{section_name} must be a mapping")
        relative = source.get("path")
        if not isinstance(relative, str) or not relative.strip():
            raise ValueError(f"{catalog_path}: workers.{catalog_id}.path must be a non-empty string")
        worker_path = (root / relative).resolve()
        try:
            worker_path.relative_to(root)
        except ValueError as error:
            raise ValueError(f"{catalog_path}: workers.{catalog_id}.path escapes the repository") from error
        if worker_path in paths:
            raise ValueError(f"{catalog_path}: duplicate worker path {relative!r}")
        paths.add(worker_path)
        publish = raw.get("publish")
        if not isinstance(publish, bool):
            raise ValueError(f"{catalog_path}: workers.{catalog_id}.publish must be a boolean")
        if publish:
            if catalog_id in publish_names:
                raise ValueError(f"{catalog_path}: duplicate publishable worker name {catalog_id!r}")
            publish_names.add(catalog_id)
        artifact_kind = artifact.get("kind")
        if artifact_kind not in {"rust-binary", "javascript-bundle", "python-bundle", "oci-image"}:
            raise ValueError(f"{catalog_path}: workers.{catalog_id}.artifact.kind is unsupported")
        manifest = source.get("package_manifest")
        binary = artifact.get("binary")
        if not isinstance(manifest, str) or not manifest:
            raise ValueError(f"{catalog_path}: workers.{catalog_id}.source.package_manifest is required")
        if artifact_kind == "rust-binary":
            targets = artifact.get("targets")
            if not isinstance(targets, list) or not targets or not all(isinstance(target, str) for target in targets):
                raise ValueError(f"{catalog_path}: workers.{catalog_id}.artifact.targets must be non-empty")
            if not isinstance(binary, str) or not binary:
                raise ValueError(f"{catalog_path}: workers.{catalog_id}.artifact.binary is required")
        public = read_iii_worker_yaml(worker_path)
        public_dependencies = public.raw.get("dependencies") or {}
        runtime = {
            "exec": [binary or catalog_id] if artifact_kind == "rust-binary" else [],
            "manifest": public.raw.get("runtime") or {},
            "scripts": public.raw.get("scripts") or {},
            "environment": public.raw.get("env") or {},
            "resources": public.raw.get("resources") or {},
        }
        registry = {
            "description": public.raw.get("description") or "",
            "license": public.raw.get("license") or "",
            "tags": public.raw.get("tags") or [],
            "dependencies": public_dependencies,
            "publish": publish,
        }
        parsed[catalog_id] = WorkerSpec(
            catalog_id=catalog_id,
            path=worker_path,
            publish=publish,
            manifest=manifest if isinstance(manifest, str) else None,
            artifact_kind=artifact_kind,
            binary=binary if isinstance(binary, str) else None,
            source=source,
            artifact=artifact,
            runtime=runtime,
            registry=registry,
            validation={"interface": "skipped" if public.raw.get("interface_smoke") is False else "required"},
            raw=raw,
        )
    return parsed


def read_worker(worker: str, path: Path | None = None) -> WorkerSpec:
    """Return one canonical catalog entry by its exact catalog ID."""
    workers = read_worker_catalog(path)
    try:
        return workers[worker]
    except KeyError as error:
        raise ValueError(f"worker {worker!r} is not declared in {path or worker_catalog_path()}") from error


def read_worker_by_path(worker_dir: Path, path: Path | None = None) -> WorkerSpec:
    """Return the catalog entry owning an exact repository directory."""
    wanted = worker_dir.resolve()
    matches = [worker for worker in read_worker_catalog(path).values() if worker.path == wanted]
    if len(matches) != 1:
        raise ValueError(f"worker path {worker_dir} is not declared exactly once in {path or worker_catalog_path()}")
    return matches[0]


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
