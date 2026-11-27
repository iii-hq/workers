#!/usr/bin/env python3
"""Decide and record the next npm version of a workspace package.

Used by publish-npm-packages.yml for the packages under packages/ that leave
the repository (@iii-dev/console-ui, @iii-dev/native-ui). npm is the source of
truth for what was released; package.json follows it through the same bot
commit subject sync-manifest-versions.yml pushes.

Subcommands:
    plan --package-dir D --tarball T [--bump auto|patch|minor|major]
         [--github-output F]
        Compare the freshly packed tarball T against the registry's `latest`
        and print one of:
          publish  a new version: the shipped files changed (auto bump from
                   the commits since the last version change), a hand bump in
                   package.json is ahead of the registry, the package was never
                   published, or a fixed bump was asked for
          sync     nothing to ship, but the registry is ahead of package.json
                   (a publish whose version commit never landed)
          none     nothing to ship and package.json is current
    set-version --package-dir D --version V
        Rewrite package.json's version in place, keeping its formatting.

Auto bump, from conventional commit messages touching the package directory:
a breaking change (`type!:` or a `BREAKING CHANGE:` footer) is major (minor
below 1.0.0), `feat` is minor, anything else is patch.

Exit codes: 0 on success, 1 on a usage, registry or consistency error.
"""
from __future__ import annotations

import argparse
import io
import json
import re
import subprocess
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path

REGISTRY = "https://registry.npmjs.org"
BUMPS = ("auto", "patch", "minor", "major")
VERSION_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
VERSION_LINE_RE = re.compile(r'^(\s*"version"\s*:\s*")([^"]*)(")', re.MULTILINE)
BREAKING_SUBJECT_RE = re.compile(r"^[A-Za-z]+(\([^)]*\))?!:")
BREAKING_FOOTER_RE = re.compile(r"^BREAKING[ -]CHANGE:", re.MULTILINE)
FEAT_SUBJECT_RE = re.compile(r"^feat(\([^)]*\))?:")
# package.json fields that change between two packs of the same sources
# without changing what a consumer installs: the version itself, the dev-only
# dependency ranges (a hand `npm publish` shipped them as literal `catalog:`),
# and the git head npm may stamp.
IGNORED_MANIFEST_FIELDS = ("version", "devDependencies", "gitHead")


@dataclass(frozen=True)
class Plan:
    action: str  # publish | sync | none
    version: str
    reason: str


def parse_version(version: str) -> tuple[int, int, int]:
    match = VERSION_RE.fullmatch(version.strip())
    if not match:
        raise ValueError(f"version {version!r} is not MAJOR.MINOR.PATCH")
    return int(match[1]), int(match[2]), int(match[3])


def bump_version(version: str, kind: str) -> str:
    major, minor, patch = parse_version(version)
    if kind == "major" and major == 0:
        # Below 1.0.0 a breaking change moves the minor (semver §4); 1.0.0 is
        # a deliberate hand bump, never a side effect of a commit message.
        kind = "minor"
    if kind == "major":
        return f"{major + 1}.0.0"
    if kind == "minor":
        return f"{major}.{minor + 1}.0"
    if kind == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise ValueError(f"unknown bump {kind!r}")


def bump_kind(messages: list[str]) -> str:
    """The largest bump any of the commit messages asks for."""
    kind = "patch"
    for message in messages:
        subject = message.strip().splitlines()[0] if message.strip() else ""
        if BREAKING_SUBJECT_RE.match(subject) or BREAKING_FOOTER_RE.search(message):
            return "major"
        if FEAT_SUBJECT_RE.match(subject):
            kind = "minor"
    return kind


def read_manifest(package_dir: Path) -> dict:
    return json.loads((package_dir / "package.json").read_text(encoding="utf-8"))


def write_version(package_dir: Path, version: str) -> None:
    parse_version(version)
    path = package_dir / "package.json"
    text = path.read_text(encoding="utf-8")
    new, count = VERSION_LINE_RE.subn(lambda m: f"{m[1]}{version}{m[3]}", text, count=1)
    if count != 1:
        raise ValueError(f"{path}: no top-level version field")
    path.write_text(new, encoding="utf-8")
    if read_manifest(package_dir).get("version") != version:
        raise ValueError(f"{path}: the first version field is not the package's own")


def tarball_contents(data: bytes) -> dict[str, bytes]:
    """Path → bytes of an npm tarball, with package.json reduced to what a
    consumer installs (see IGNORED_MANIFEST_FIELDS)."""
    files: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive.getmembers():
            if not member.isfile():
                continue
            # npm always roots a tarball at package/, but tolerate any root.
            name = member.name.split("/", 1)[1] if "/" in member.name else member.name
            extracted = archive.extractfile(member)
            files[name] = extracted.read() if extracted else b""
    manifest = files.get("package.json")
    if manifest is not None:
        fields = json.loads(manifest)
        for key in IGNORED_MANIFEST_FIELDS:
            fields.pop(key, None)
        files["package.json"] = json.dumps(fields, sort_keys=True).encode()
    return files


def fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=60) as response:  # noqa: S310 - fixed https registry
        return response.read()


def registry_document(name: str, registry: str = REGISTRY) -> dict | None:
    """The package's registry document, or None when it was never published."""
    url = f"{registry}/{urllib.parse.quote(name, safe='@')}"
    try:
        return json.loads(fetch(url))
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise


def _git(package_dir: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=package_dir, capture_output=True, text=True)


def _version_at(package_dir: Path, revision: str, path: str) -> str | None:
    shown = _git(package_dir, "show", f"{revision}:{path}")
    if shown.returncode != 0:
        return None
    try:
        return json.loads(shown.stdout).get("version")
    except ValueError:
        return None


def release_baseline(package_dir: Path) -> str | None:
    """The newest commit that changed package.json's version value: the last
    release point, whether a bot sync or a hand bump. Compared by value, not
    by line, so a field added after `version` (its trailing comma) is not one."""
    prefix = _git(package_dir, "rev-parse", "--show-prefix").stdout.strip()
    path = f"{prefix}package.json"
    log = _git(package_dir, "log", "--format=%H", "--", "package.json")
    log.check_returncode()
    for sha in log.stdout.split():
        if _version_at(package_dir, sha, path) != _version_at(package_dir, f"{sha}^", path):
            return sha
    return None


def commit_messages_since(package_dir: Path, baseline: str | None) -> list[str]:
    revision = f"{baseline}..HEAD" if baseline else "HEAD"
    out = subprocess.run(
        ["git", "log", "--format=%B%x00", revision, "--", "."],
        cwd=package_dir,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return [message.strip() for message in out.split("\0") if message.strip()]


def decide(
    *,
    current: str,
    latest: str | None,
    published: set[str],
    same_as_latest: bool | None,
    messages: list[str],
    bump: str,
) -> Plan:
    """Pure decision; same_as_latest is None when there was nothing to compare."""
    parse_version(current)
    if bump not in BUMPS:
        raise ValueError(f"unknown bump {bump!r}")
    if bump != "auto":
        base = current if latest is None or parse_version(current) > parse_version(latest) else latest
        version = bump_version(base, bump)
        if version in published:
            raise ValueError(f"{version} is already on the registry")
        return Plan("publish", version, f"{bump} bump asked for, from {base}")
    if latest is None:
        return Plan("publish", current, "first publication")
    if parse_version(current) > parse_version(latest):
        if current in published:
            raise ValueError(
                f"package.json {current} is published but `latest` is {latest}; fix the dist-tag first"
            )
        return Plan("publish", current, f"package.json {current} is ahead of the registry's {latest}")
    if same_as_latest:
        if current == latest:
            return Plan("none", current, f"the packed files match {latest}")
        return Plan("sync", latest, f"the packed files match {latest}; package.json is behind at {current}")
    kind = bump_kind(messages)
    return Plan("publish", bump_version(latest, kind), f"the packed files differ from {latest} ({kind})")


def cmd_plan(args: argparse.Namespace) -> int:
    package_dir = Path(args.package_dir).resolve()
    manifest = read_manifest(package_dir)
    name, current = manifest["name"], manifest["version"]
    document = registry_document(name, args.registry)
    versions = set((document or {}).get("versions", {}))
    latest = ((document or {}).get("dist-tags") or {}).get("latest")

    same_as_latest = None
    if latest is not None and args.bump == "auto" and parse_version(current) <= parse_version(latest):
        published_tarball = fetch(document["versions"][latest]["dist"]["tarball"])
        local = tarball_contents(Path(args.tarball).read_bytes())
        remote = tarball_contents(published_tarball)
        same_as_latest = local == remote
        if not same_as_latest:
            changed = sorted(p for p in local.keys() | remote.keys() if local.get(p) != remote.get(p))
            print(f"changed since {latest}: {', '.join(changed)}", file=sys.stderr)

    messages = commit_messages_since(package_dir, release_baseline(package_dir))
    plan = decide(
        current=current,
        latest=latest,
        published=versions,
        same_as_latest=same_as_latest,
        messages=messages,
        bump=args.bump,
    )
    outputs = {
        "name": name,
        "current": current,
        "latest": latest or "",
        "action": plan.action,
        "version": plan.version,
        "tag": f"{name}@{plan.version}",
        "reason": plan.reason,
    }
    for key, value in outputs.items():
        print(f"{key}={value}")
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as fh:
            for key, value in outputs.items():
                fh.write(f"{key}={value}\n")
    return 0


def cmd_set_version(args: argparse.Namespace) -> int:
    write_version(Path(args.package_dir), args.version)
    print(args.version)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    p_plan = sub.add_parser("plan", help="decide whether and as what to publish")
    p_plan.add_argument("--package-dir", required=True)
    p_plan.add_argument("--tarball", required=True, help="the output of `pnpm pack` for the package")
    p_plan.add_argument("--bump", choices=BUMPS, default="auto")
    p_plan.add_argument("--registry", default=REGISTRY)
    p_plan.add_argument("--github-output")
    p_plan.set_defaults(func=cmd_plan)

    p_set = sub.add_parser("set-version", help="write a version into package.json")
    p_set.add_argument("--package-dir", required=True)
    p_set.add_argument("--version", required=True)
    p_set.set_defaults(func=cmd_set_version)

    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
