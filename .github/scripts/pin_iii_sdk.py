#!/usr/bin/env python3
"""Pin every project in this repository to one iii SDK release and re-lock it.

Run it through `scripts/pin-iii-sdk.sh`, whose header lists the order of the
last mile of an iii release. Subcommands:

    pin VERSION [--dry-run] [--only GROUPS] [--no-relock] [--no-preflight]
                [--cargo-config FILE]
        Write VERSION (semver, e.g. 0.26.0-rc.1) into every SDK pin, then
        re-lock each project that resolves an iii SDK package. Pins already
        at VERSION (from an earlier `--only` run) stay; the rest move.
    kit [--dry-run] [--no-fetch] [--remote NAME] [WORKER=VERSION ...]
        Set the deepwork kit's worker ranges to the first release of each
        worker whose manifest pins the current iii-sdk version.
    template export DIR
        Copy the synced template files into DIR/iii/harness/ (a checkout of
        iii-hq/templates).
    template record COMMIT [--repo URL]
        Check that COMMIT of iii-hq/templates carries the synced files, then
        record it in template/upstream/sync.json.

Groups (`--only`): rust, node, python, go, engine. `engine` covers the iii
release used by CI (`III_RELEASE_TAG`), the engine archive in hermes/Dockerfile
and the tests that assert both.
"""
from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from itertools import zip_longest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import _lib  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]

RUST_CRATES = ("iii-sdk", "iii-helpers", "iii-observability", "iii-filter", "function-macros")
NODE_PACKAGES = ("iii-sdk", "iii-browser-sdk", "@iii-dev/helpers", "@iii-dev/observability")
PYTHON_PACKAGES = ("iii-sdk", "iii-helpers", "iii-observability")
GO_MODULE = "github.com/iii-hq/iii/sdk/packages/go/iii"
III_REPO = "iii-hq/iii"
GROUPS = ("rust", "node", "python", "go", "engine")
KIT = Path("kits/deepwork/iii.kit.yaml")
SYNC = Path("template/upstream/sync.json")
TEMPLATES_REPO = "https://github.com/iii-hq/templates.git"

SEMVER = r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?"
PEP440 = r"\d+\.\d+\.\d+(?:(?:a|b|rc)\d+)?"


def alt(names: Iterable[str]) -> str:
    return "|".join(re.escape(n) for n in names)


CRATES, NODE, PY = alt(RUST_CRATES), alt(NODE_PACKAGES), alt(PYTHON_PACKAGES)


# --------------------------------------------------------------------------- versions


@dataclass(frozen=True)
class Target:
    semver: str  # 0.26.0-rc.1: crates, npm, engine tag, Go module
    pep440: str  # 0.26.0rc1: PyPI

    @classmethod
    def parse(cls, version: str) -> Target:
        version = version.removeprefix("v")
        _lib.parse_release_version(version)  # the repository's release grammar
        return cls(version, _lib.pep440_version(version))


def semver_of_pep440(version: str) -> str:
    """0.26.0rc1 -> 0.26.0-rc.1, the inverse of `_lib.pep440_version` for rc and stable."""
    m = re.fullmatch(r"(\d+\.\d+\.\d+)(?:rc(\d+))?", version)
    if not m:
        raise ValueError(f"{version}: no semver spelling known")
    return m.group(1) + (f"-rc.{m.group(2)}" if m.group(2) else "")


# --------------------------------------------------------------------------- edit rules


@dataclass(frozen=True)
class Rule:
    """One pin form: files it applies to, and a regex whose `ver` group is the pin.

    `spelling` picks what replaces `ver`: the semver or PEP 440 spelling of the
    target, or the sha256 of the engine archive for the `triple` group.
    `current_only` rules rewrite a value only when it is the version the
    repository pins today, so unrelated versions in the same files stay.
    """

    group: str
    name: str
    matches: Callable[[str], bool]
    pattern: str
    spelling: str = "semver"  # semver | pep440 | digest
    current_only: bool = False
    pin: bool = True  # a dependency pin, read to find the version pinned today


def under(*globs: str) -> Callable[[str], bool]:
    return lambda path: any(fnmatch.fnmatch(path, g) for g in globs)


def named(*names: str) -> Callable[[str], bool]:
    return lambda path: Path(path).name in names


RULES = (
    Rule("rust", "Cargo.toml dependency", named("Cargo.toml"),
         rf'(?m)^(?P<pre>[ \t]*(?:{CRATES})[ \t]*=[ \t]*"(?:=|\^|~|>=)?)(?P<ver>{SEMVER})(?P<post>")'),
    Rule("rust", "Cargo.toml dependency table", named("Cargo.toml"),
         rf'(?m)^(?P<pre>[ \t]*(?:{CRATES})[ \t]*=[ \t]*\{{[^}}\n]*?\bversion[ \t]*=[ \t]*"(?:=|\^|~|>=)?)'
         rf'(?P<ver>{SEMVER})(?P<post>")'),
    Rule("node", "package.json dependency", named("package.json"),
         rf'(?P<pre>"(?:{NODE})"[ \t]*:[ \t]*"(?:\^|~|>=|=)?)(?P<ver>{SEMVER})(?P<post>")'),
    Rule("node", "pnpm minimumReleaseAgeExclude", named("pnpm-workspace.yaml"),
         rf"(?m)^(?P<pre>[ \t]*-[ \t]*['\"]?(?:{NODE})@)(?P<ver>{SEMVER}(?:[ \t]*\|\|[ \t]*{SEMVER})*)"
         r"(?P<post>['\"]?[ \t]*)$", pin=False),
    Rule("node", "generated worker package.json", under("*.rs"),
         r'(?P<pre>\\"iii-sdk\\": \\"\^)(?P<ver>{current})(?P<post>\\")', current_only=True),
    Rule("node", "SDK release test", under(".github/scripts/tests/test_released_worker_sdk_runtime.py"),
         r'(?P<pre>\["iii-sdk"\] == ")(?P<ver>{current})(?P<post>")', current_only=True),
    Rule("python", "pyproject dependency", named("pyproject.toml"),
         rf'(?P<pre>"(?:{PY})(?:\[[^\]"]*\])?[ \t]*(?:==|>=|~=)[ \t]*)(?P<ver>{PEP440})(?P<post>[",;\s])',
         spelling="pep440"),
    Rule("python", "guest SDK requirement", under("*.rs", "sandbox-code-runner/README.md"),
         r"(?P<pre>iii-sdk>=)(?P<ver>{current_pep440})(?P<post>['\"])", spelling="pep440", current_only=True),
    Rule("python", "SDK release test", under(".github/scripts/tests/test_released_worker_sdk_runtime.py"),
         rf'(?P<pre>"(?:{PY})>=)(?P<ver>{{current_pep440}})(?P<post>")', spelling="pep440", current_only=True),
    Rule("go", "go.mod require", named("go.mod"),
         rf"(?m)^(?P<pre>[ \t]*(?:require[ \t]+)?{re.escape(GO_MODULE)}[ \t]+v)(?P<ver>{SEMVER})"
         r"(?P<post>(?:[ \t].*)?)$"),
    Rule("engine", "engine archive URL", under("*Dockerfile*"),
         rf"(?P<pre>github\.com/{re.escape(III_REPO)}/releases/download/iii/v)(?P<ver>{SEMVER})(?P<post>/)"),
    Rule("engine", "engine archive sha256", under("*Dockerfile*"),
         r"(?P<pre>iii_target=(?P<triple>[a-z0-9_]+-[a-z0-9_-]+);[ \t]*iii_sha=)(?P<ver>[0-9a-f]{64})(?P<post>)",
         spelling="digest"),
    Rule("engine", "CI engine release", under(".github/workflows/*.yml"),
         r"(?m)^(?P<pre>[ \t]*(?:III_RELEASE_TAG:|-)[ \t]*['\"]?iii/v)(?P<ver>{current})(?P<post>['\"]?[ \t]*)$",
         current_only=True),
    Rule("engine", "CI engine release test", under(".github/scripts/tests/test_rust_ci_workflows.py"),
         r'(?P<pre>"iii/v)(?P<ver>{current})(?P<post>")', current_only=True),
)


@dataclass
class Edit:
    path: str
    group: str
    before: str
    after: str

    def lines(self) -> list[tuple[int, str, str]]:
        old, new = self.before.splitlines(), self.after.splitlines()
        return [(i + 1, a, b) for i, (a, b) in enumerate(zip_longest(old, new, fillvalue="")) if a != b]


def git_files(root: Path, *patterns: str) -> list[str]:
    out = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z", "--", *patterns], text=True)
    return sorted(p for p in out.split("\0") if p and (root / p).is_file())


def current_versions(root: Path, files: list[str]) -> dict[str, set[str]]:
    """Every SDK version pinned today, keyed by semver spelling, with the files that pin it."""
    found: dict[str, set[str]] = {}
    for rule in RULES:
        if not rule.pin or rule.current_only or rule.group == "engine":
            continue
        for path in filter(rule.matches, files):
            for m in re.finditer(rule.pattern, (root / path).read_text()):
                ver = m.group("ver")
                if rule.spelling == "pep440":
                    ver = semver_of_pep440(ver)
                found.setdefault(ver, set()).add(path)
    return found


def unsupported_forms(root: Path, files: list[str]) -> list[str]:
    """Pins in a form no rule rewrites; the run stops instead of leaving them behind."""
    bad = []
    for path in filter(named("Cargo.toml"), files):
        text = (root / path).read_text()
        tables = rf"(?m)^\[(?:[^\]]*\.)?(?:dependencies|dev-dependencies|build-dependencies)\.(?:{CRATES})\]"
        for m in re.finditer(tables, text):
            bad.append(f"{path}: {m.group(0)}")
    return bad


def plan_edits(
    root: Path,
    files: list[str],
    target: Target,
    current: set[str],
    groups: set[str],
    digest: Callable[[str], str],
) -> list[Edit]:
    subst = {
        "current": alt(sorted(current)) or r"(?!)",
        "current_pep440": alt(sorted(_lib.pep440_version(v) for v in current)) or r"(?!)",
    }
    edits: dict[str, Edit] = {}
    for rule in RULES:
        if rule.group not in groups:
            continue
        pattern = re.compile(rule.pattern.replace("{current}", subst["current"]).replace(
            "{current_pep440}", subst["current_pep440"]))

        def replace(m: re.Match[str], rule: Rule = rule) -> str:
            if rule.spelling == "digest":
                value = digest(m.group("triple"))
            else:
                value = target.pep440 if rule.spelling == "pep440" else target.semver
            return m.group("pre") + value + m.group("post")

        for path in filter(rule.matches, files):
            edit = edits.get(path)
            text = edit.after if edit else (root / path).read_text()
            new = pattern.sub(replace, text)
            if new != text:
                if edit:
                    edit.after = new
                    if rule.group not in edit.group.split("+"):
                        edit.group += f"+{rule.group}"
                else:
                    edits[path] = Edit(path, rule.group, text, new)
    return [e for _, e in sorted(edits.items())]


# --------------------------------------------------------------------------- release lookups


def http_get(url: str, accept: str = "application/json") -> bytes | None:
    request = urllib.request.Request(url, headers={"Accept": accept, "User-Agent": "iii-workers-pin-iii-sdk"})
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token and url.startswith("https://api.github.com/"):
        request.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.read()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise


def release_digests(tag: str) -> dict[str, str]:
    """sha256 of each asset of the iii GitHub release `tag`, by asset name."""
    out: bytes | None = None
    if shutil.which("gh"):
        proc = subprocess.run(
            ["gh", "api", f"repos/{III_REPO}/releases/tags/{tag}", "--jq", ".assets"],
            capture_output=True,
        )
        out = proc.stdout if proc.returncode == 0 else None
    if out is None:
        body = http_get(f"https://api.github.com/repos/{III_REPO}/releases/tags/{urllib.parse.quote(tag, safe='')}")
        out = json.dumps(json.loads(body)["assets"]).encode() if body else None
    if out is None:
        return {}
    return {
        a["name"]: a["digest"].removeprefix("sha256:")
        for a in json.loads(out)
        if (a.get("digest") or "").startswith("sha256:")
    }


def preflight(target: Target, groups: set[str]) -> list[str]:
    """What the registries do not serve yet; the run stops before writing anything."""
    checks: list[tuple[str, str]] = []
    if "rust" in groups:
        for crate in ("iii-sdk", "iii-helpers", "iii-filter"):
            checks.append((f"crates.io {crate} {target.semver}", f"crate:{crate}"))
    if "node" in groups:
        for package in ("iii-sdk", "iii-browser-sdk", "@iii-dev/helpers"):
            checks.append((f"npm {package}@{target.semver}",
                           f"https://registry.npmjs.org/{package.replace('/', '%2F')}/{target.semver}"))
    if "python" in groups:
        for package in ("iii-sdk", "iii-helpers"):
            checks.append((f"PyPI {package}=={target.pep440}", f"https://pypi.org/pypi/{package}/{target.pep440}/json"))
    if "go" in groups:
        checks.append((f"Go {GO_MODULE}@v{target.semver}",
                       f"https://proxy.golang.org/{GO_MODULE}/@v/v{target.semver}.info"))
    missing = []
    for label, where in checks:
        try:
            if where.startswith("crate:"):
                name = where.removeprefix("crate:")
                body = http_get(f"https://index.crates.io/{name[:2]}/{name[2:4]}/{name}", accept="text/plain")
                ok = bool(body) and any(json.loads(line).get("vers") == target.semver
                                         for line in body.decode().splitlines() if line.strip())
            else:
                ok = http_get(where) is not None
        except (urllib.error.URLError, TimeoutError, ValueError) as e:
            missing.append(f"{label}: {e}")
            continue
        if not ok:
            missing.append(f"{label}: not published")
    return missing


# --------------------------------------------------------------------------- re-lock


@dataclass
class Step:
    group: str
    project: str
    commands: list[list[str]]
    cwd: Path
    env: dict[str, str] = field(default_factory=dict)
    check: Callable[[], str | None] = lambda: None
    # Commands computed from the state the first commands leave (cargo --precise).
    then: Callable[[], list[list[str]]] | None = None
    then_shown: str = ""
    blocker: str = ""  # why the step cannot run here; reported without running it
    status: str = "planned"
    detail: str = ""


def lock_names(lock: Path, names: Iterable[str]) -> dict[str, list[str]]:
    """Versions of each `[[package]]` named in `names` (Cargo.lock and uv.lock share the layout)."""
    wanted = set(names)
    found: dict[str, list[str]] = {}
    for block in re.split(r"(?m)^(?=\[\[package\]\]$)", lock.read_text()):
        name = re.search(r'(?m)^name = "([^"]+)"$', block)
        version = re.search(r'(?m)^version = "([^"]+)"$', block)
        if name and version and name.group(1) in wanted:
            source = re.search(r"(?m)^source = (.*)$", block)
            if source and re.match(r"\{ (?:editable|virtual|directory) = ", source.group(1)):
                continue  # the project itself or a path member, never an SDK release
            found.setdefault(name.group(1), []).append(version.group(1))
    return found


def rust_steps(root: Path, target: Target, cargo_config: list[str]) -> list[Step]:
    steps = []
    for lock_path in git_files(root, "*Cargo.lock"):
        lock = root / lock_path
        manifest = lock.with_name("Cargo.toml")
        crates = lock_names(lock, RUST_CRATES)
        if not crates or not manifest.is_file():
            continue
        base = ["cargo", *cargo_config]
        update = [*base, "update", "--manifest-path", str(manifest.relative_to(root))]
        commands = [[*update, *[a for c in sorted(crates) for a in ("-p", c)]]]

        def precise(lock: Path = lock, update: list[str] = update) -> list[list[str]]:
            # iii-sdk and iii-helpers are exact pins; iii-filter (and any other
            # SDK crate) comes through iii-sdk's caret requirement, so --precise
            # holds it at the release instead of a newer prerelease.
            return [[*update, "-p", c, "--precise", target.semver]
                    for c, versions in sorted(lock_names(lock, RUST_CRATES).items())
                    if versions != [target.semver]]

        def check(lock: Path = lock, manifest: Path = manifest, base: list[str] = base) -> str | None:
            stale = {c: v for c, v in lock_names(lock, RUST_CRATES).items() if v != [target.semver]}
            if stale:
                return f"Cargo.lock still has {stale}"
            proc = subprocess.run([*base, "metadata", "--locked", "--format-version", "1", "--manifest-path",
                                   str(manifest)], capture_output=True, text=True, cwd=root)
            return None if proc.returncode == 0 else "cargo metadata --locked: " + tail(proc.stderr)

        steps.append(Step("rust", str(lock.parent.relative_to(root)), commands, root, check=check, then=precise,
                          then_shown=f"cargo update -p <crate> --precise {target.semver} (each SDK crate not at it)"))
    return steps


def node_steps(root: Path, target: Target) -> list[Step]:
    steps = []
    needles = [f"node_modules/{p}" for p in NODE_PACKAGES] + [f"{p}@" for p in NODE_PACKAGES]
    for lock_path in git_files(root, "*pnpm-lock.yaml", "*package-lock.json"):
        lock = root / lock_path
        text = lock.read_text()
        if not any(n in text for n in needles):
            continue
        project = lock.parent
        if lock.name == "package-lock.json":
            commands = [["npm", "install", "--package-lock-only", "--ignore-scripts", "--no-audit", "--no-fund"]]
        else:
            args = ["install", "--lockfile-only"]
            if not (project / "pnpm-workspace.yaml").is_file() and any(
                (p / "pnpm-workspace.yaml").is_file() for p in project.parents if p.is_relative_to(root)
            ):
                args.append("--ignore-workspace")
            commands = [[*pnpm_command(project), *args]]

        def check(lock: Path = lock) -> str | None:
            stale = {p: v for p, v in node_lock_versions(lock).items() if v != {target.semver}}
            return f"{lock.name} still has {stale}" if stale else None

        steps.append(Step("node", str(project.relative_to(root)), commands, project, check=check))
    return steps


def node_lock_versions(lock: Path) -> dict[str, set[str]]:
    text = lock.read_text()
    versions: dict[str, set[str]] = {}
    if lock.name == "package-lock.json":
        for key, entry in json.loads(text).get("packages", {}).items():
            name = key.rpartition("node_modules/")[2]
            if key and name in NODE_PACKAGES and "version" in entry:
                versions.setdefault(name, set()).add(entry["version"])
    else:
        for m in re.finditer(rf"(?m)^  '?/?({NODE})@({SEMVER})(?:\(|'|:)", text):
            versions.setdefault(m.group(1), set()).add(m.group(2))
    return versions


def pnpm_command(project: Path) -> list[str]:
    """pnpm at the version the project's package.json (or its workspace root's) declares."""
    if os.environ.get("PIN_PNPM"):
        return os.environ["PIN_PNPM"].split()
    for directory in (project, *project.parents):
        manifest = directory / "package.json"
        if manifest.is_file():
            declared = json.loads(manifest.read_text()).get("packageManager", "")
            if declared.startswith("pnpm@"):
                # npx runs the declared pnpm on any Node; the corepack bundled
                # with Node 22 fails on current npm signing keys, Node 25 has none.
                return ["npx", "--yes", declared.split("+", 1)[0]]
        if (directory / "pnpm-workspace.yaml").is_file() and directory != project:
            break
    return ["pnpm"]


def python_steps(root: Path, target: Target) -> list[Step]:
    steps = []
    for lock_path in git_files(root, "*uv.lock"):
        lock = root / lock_path
        packages = lock_names(lock, PYTHON_PACKAGES)
        if not packages:
            continue
        project = lock.parent
        uv = uv_command(project)
        command = [*uv, "lock", "--project", str(project.relative_to(root))]
        for p in sorted(packages):
            command += ["--upgrade-package", f"{p}=={target.pep440}"]

        def check(lock: Path = lock, project: Path = project, uv: list[str] = uv) -> str | None:
            stale = {p: v for p, v in lock_names(lock, PYTHON_PACKAGES).items() if v != [target.pep440]}
            if stale:
                return f"uv.lock still has {stale}"
            proc = subprocess.run([*uv, "lock", "--locked", "--project", str(project)], capture_output=True, text=True)
            return None if proc.returncode == 0 else "uv lock --locked: " + tail(proc.stderr)

        step = Step("python", str(project.relative_to(root)), [command], root, check=check)
        step.blocker = uv_too_old(project, uv)
        steps.append(step)
    return steps


def uv_command(project: Path) -> list[str]:
    return os.environ["PIN_UV"].split() if os.environ.get("PIN_UV") else ["uv"]


def uv_too_old(project: Path, uv: list[str]) -> str:
    """A uv older than the one the project's Dockerfile builds with rewrites its lock in an older format."""
    if os.environ.get("PIN_UV"):
        return ""
    pinned = None
    for dockerfile in project.glob("*Dockerfile*"):
        m = re.search(r"ghcr\.io/astral-sh/uv:(\d+\.\d+\.\d+)", dockerfile.read_text())
        pinned = m.group(1) if m else pinned
    if not pinned or not shutil.which(uv[0]):
        return ""
    out = subprocess.run([*uv, "--version"], capture_output=True, text=True).stdout
    m = re.search(r"(\d+)\.(\d+)\.(\d+)", out)
    if m and tuple(map(int, m.groups())) < tuple(map(int, pinned.split("."))):
        return (f"uv {m.group(0)} is older than uv {pinned}, which {project.name}'s Dockerfile builds with; "
                f"run `uv self update`, or set PIN_UV (e.g. PIN_UV='uvx uv@{pinned}')")
    return ""


def go_steps(root: Path, target: Target) -> list[Step]:
    steps = []
    for mod_path in git_files(root, "*go.mod"):
        mod = root / mod_path
        if not re.search(rf"(?m)^\s*(?:require\s+)?{re.escape(GO_MODULE)}\s+v", mod.read_text()):
            continue

        def check(mod: Path = mod) -> str | None:
            sums = mod.with_name("go.sum")
            text = sums.read_text() if sums.is_file() else ""
            line = f"{GO_MODULE} v{target.semver} h1:"
            return None if line in text else f"go.sum has no `{line}` line"

        steps.append(Step("go", str(mod.parent.relative_to(root)), [["go", "mod", "tidy"]], mod.parent,
                          env={"GOWORK": "off"}, check=check))
    return steps


def tail(text: str, lines: int = 12) -> str:
    return "\n".join(text.strip().splitlines()[-lines:])


def run_step(step: Step) -> None:
    if step.blocker:
        step.status, step.detail = "failed", step.blocker
        return
    env = {**os.environ, **step.env}
    for batch in (lambda: step.commands, step.then or (lambda: [])):
        for command in batch():
            if not shutil.which(command[0]):
                step.status, step.detail = "failed", f"{command[0]} is not on PATH"
                return
            proc = subprocess.run(command, cwd=step.cwd, env=env, capture_output=True, text=True)
            if proc.returncode != 0:
                step.status = "failed"
                step.detail = f"$ {' '.join(command)}\n{tail(proc.stdout + proc.stderr)}"
                return
    problem = step.check()
    step.status, step.detail = ("failed", problem) if problem else ("ok", "")


def show_command(step: Step) -> str:
    shown = [" ".join(c) for c in step.commands]
    if step.then_shown:
        shown.append(step.then_shown)
    prefix = " ".join(f"{k}={v}" for k, v in step.env.items())
    return "; ".join(shown) if not prefix else f"{prefix} " + "; ".join(shown)


# --------------------------------------------------------------------------- pin


def cmd_pin(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    target = Target.parse(args.version)
    groups = set(args.only.split(",")) if args.only else set(GROUPS)
    if groups - set(GROUPS):
        print(f"error: unknown group(s) {sorted(groups - set(GROUPS))}; choose from {', '.join(GROUPS)}",
              file=sys.stderr)
        return 2
    files = git_files(root)

    bad = unsupported_forms(root, files)
    if bad:
        print("error: SDK pins in a form this script does not rewrite:\n  " + "\n  ".join(bad), file=sys.stderr)
        return 1
    pinned = current_versions(root, files)
    # Pins at the target come from an earlier `--only` run; the run moves the rest.
    current = set(pinned) - {target.semver} or set(pinned)
    if len(current) > 1:
        print("error: the repository pins more than one SDK version:", file=sys.stderr)
        for version, where in sorted(pinned.items()):
            print(f"  {version}: {', '.join(sorted(where)[:6])}{' ...' if len(where) > 6 else ''}", file=sys.stderr)
        return 1
    engine_tag = f"iii/v{target.semver}"

    digests: dict[str, str] | None = None
    missing_digests: list[str] = []

    def digest(triple: str) -> str:
        nonlocal digests
        if digests is None:
            digests = release_digests(engine_tag)
        asset = f"iii-{triple}.tar.gz"
        if asset not in digests:
            missing_digests.append(asset)
            return f"<sha256 of {asset} in {engine_tag}>"
        return digests[asset]

    edits = plan_edits(root, files, target, current, groups, digest)
    verb = "would edit" if args.dry_run else "edits"
    print(f"iii SDK pins {', '.join(sorted(current)) or '(none)'} -> {target.semver} "
          f"(PyPI {target.pep440}, Go v{target.semver}, engine {engine_tag})")
    print(f"\n{verb} {len(edits)} files:")
    for edit in edits:
        for line, before, after in edit.lines():
            print(f"  {edit.group:<6} {edit.path}:{line}: {before.strip()}  ->  {after.strip()}")

    steps: list[Step] = []
    if not args.no_relock:
        cargo_config = [a for f in args.cargo_config for a in ("--config", str(Path(f).resolve()))]
        if "rust" in groups:
            steps += rust_steps(root, target, cargo_config)
        if "node" in groups:
            steps += node_steps(root, target)
        if "python" in groups:
            steps += python_steps(root, target)
        if "go" in groups:
            steps += go_steps(root, target)
    print(f"\n{'would re-lock' if args.dry_run else 're-locks'} {len(steps)} projects:")
    for step in steps:
        print(f"  {step.group:<6} {step.project}: {show_command(step)}")

    blocked = [s for s in steps if s.blocker]
    for step in blocked:
        print(f"\n{'note' if args.dry_run else 'error'}: {step.group} {step.project}: {step.blocker}",
              file=sys.stdout if args.dry_run else sys.stderr)

    if args.dry_run:
        if missing_digests:
            print(f"\nnote: release {engine_tag} has no {', '.join(sorted(set(missing_digests)))} yet; "
                  "a real run reads their sha256 from it")
        return 0

    if blocked:
        return 1
    problems = sorted(set(missing_digests))
    if problems:
        print(f"\nerror: release {engine_tag} has no asset digest for {', '.join(problems)}", file=sys.stderr)
        return 1
    if not args.no_preflight:
        missing = preflight(target, groups)
        if missing:
            print("\nerror: the release is not fully published yet (re-run once it is, or pass --no-preflight):",
                  file=sys.stderr)
            for m in missing:
                print(f"  {m}", file=sys.stderr)
            return 1

    for edit in edits:
        (root / edit.path).write_text(edit.after)
    for i, step in enumerate(steps, 1):
        print(f"[{i}/{len(steps)}] {step.group} {step.project} ...", end=" ", flush=True)
        run_step(step)
        print(step.status)
        if step.status != "ok":
            print("    " + step.detail.replace("\n", "\n    "))

    failed = [s for s in steps if s.status != "ok"]
    print("\nsummary")
    counts = {g: sum(g in e.group.split("+") for e in edits) for g in GROUPS}
    print(f"  edited {len(edits)} files ({', '.join(f'{g} {n}' for g, n in counts.items() if n) or 'none'})")
    for group in GROUPS:
        mine = [s for s in steps if s.group == group]
        if mine:
            print(f"  re-locked {group}: {sum(s.status == 'ok' for s in mine)}/{len(mine)}")
    for step in failed:
        print(f"  FAILED {step.group} {step.project}: {step.detail.splitlines()[0] if step.detail else ''}")
    print("\nnext: review `git diff`, commit and push the branch, merge it into main once CI passes, "
          "then publish the worker releases through Release Control.")
    return 1 if failed else 0


# --------------------------------------------------------------------------- kit


def kit_entries(text: str) -> dict[str, re.Match[str]]:
    block = re.search(r"(?ms)^workers:\n(.*?)(?=^\S|\Z)", text)
    if not block:
        return {}
    entries = {}
    for m in re.finditer(
        r"(?m)^(?P<pre>[ \t]+(?P<name>[a-z0-9-]+):[ \t]*(?P<q>[\"']?)(?P<op>\^|~|>=|>|=)?)(?P<ver>[0-9][^\"'\s]*)"
        r"(?P<post>(?P=q)[ \t]*)$",
        block.group(1),
    ):
        entries[m.group("name")] = m
    return entries


def first_release_on(root: Path, worker: str, sdk: str, fetch: bool, remote: str) -> str | None:
    if fetch:
        subprocess.run(["git", "-C", str(root), "fetch", "--quiet", "--no-tags", remote,
                        f"+refs/tags/{worker}/v*:refs/tags/{worker}/v*"], check=False)
    tags = subprocess.check_output(["git", "-C", str(root), "tag", "-l", f"{worker}/v*"], text=True).split()
    versions = []
    for tag in tags:
        version = tag.partition("/v")[2]
        try:
            _lib.parse_release_version(version)
        except ValueError:
            continue
        versions.append(version)
    spec = _lib.read_worker(worker)
    manifest = (spec.path / str(spec.manifest)).relative_to(root)
    for version in sorted(versions, key=_lib.parse_semver):
        proc = subprocess.run(["git", "-C", str(root), "show", f"{worker}/v{version}:{manifest.as_posix()}"],
                              capture_output=True, text=True)
        if proc.returncode == 0 and re.search(rf'(?m)^iii-sdk\s*=\s*"=?{re.escape(sdk)}"', proc.stdout):
            return version
    return None


def cmd_kit(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    pinned = current_versions(root, git_files(root))
    if len(pinned) != 1:
        print(f"error: expected one pinned SDK version, found {sorted(pinned) or 'none'}", file=sys.stderr)
        return 1
    sdk = next(iter(pinned))
    explicit = dict(a.split("=", 1) for a in args.versions)
    path = root / KIT
    text = path.read_text()
    entries = kit_entries(text)
    unknown = set(explicit) - set(entries)
    if unknown:
        print(f"error: {KIT} has no worker {sorted(unknown)}", file=sys.stderr)
        return 2
    new_text, missing = text, []
    print(f"{KIT}: first releases built on iii-sdk {sdk}")
    for name, m in entries.items():
        version = explicit.get(name) or first_release_on(root, name, sdk, not args.no_fetch, args.remote)
        if not version:
            missing.append(name)
            print(f"  {name}: no release pins iii-sdk {sdk} yet (stays {m.group('op') or ''}{m.group('ver')})")
            continue
        line = m.group(0)
        updated = m.group("pre") + version + m.group("post")
        new_text = new_text.replace(line, updated, 1)
        print(f"  {name}: {line.strip()}  ->  {updated.strip()}")
    if new_text != text:
        if args.dry_run:
            print("dry run: nothing written")
        else:
            path.write_text(new_text)
            print(f"wrote {KIT}; review the comments above each changed range")
    return 1 if missing else 0


# --------------------------------------------------------------------------- template


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cmd_template(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    manifest_path = root / SYNC
    manifest = json.loads(manifest_path.read_text())
    template = manifest_path.parents[1]
    local = {name: sha256((template / name).read_bytes()) for name in manifest["files"]}
    stale = sorted(n for n, h in local.items() if manifest["files"][n] != h)
    if stale:
        print(f"error: {SYNC} hashes differ from the files for {stale}", file=sys.stderr)
        return 1
    if args.action == "export":
        destination = Path(args.target).resolve() / manifest["source"]
        for name in manifest["files"]:
            out = destination / name
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(template / name, out)
            print(f"  {manifest['source']}{name}")
        print(f"copied {len(manifest['files'])} files into {destination}; commit and push them in iii-hq/templates, "
              f"then run: scripts/pin-iii-sdk.sh template record <commit>")
        return 0

    commit = args.target
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        print("error: record takes a full 40-character commit", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory() as scratch:
        git = ["git", "-C", scratch]
        subprocess.run(["git", "init", "--quiet", scratch], check=True)
        subprocess.run([*git, "-c", "protocol.ext.allow=never", "fetch", "--quiet", "--depth=1", "--no-tags",
                        args.repo, commit], check=True, env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
        source = manifest["source"]
        listed = subprocess.check_output([*git, "ls-tree", "-r", "--name-only", "FETCH_HEAD", "--",
                                          f"{source}agents", f"{source}skills/harness"], text=True).split()
        upstream = {
            p.removeprefix(source): sha256(subprocess.check_output([*git, "cat-file", "blob", f"FETCH_HEAD:{p}"]))
            for p in listed
        }
    differ = sorted(n for n in set(upstream) | set(local) if upstream.get(n) != local.get(n))
    if differ:
        print(f"error: {commit[:12]} of {args.repo} does not carry these synced files as they are here:",
              file=sys.stderr)
        for name in differ:
            state = "missing upstream" if name not in upstream else "only upstream" if name not in local else "differs"
            print(f"  {source}{name}: {state}", file=sys.stderr)
        return 1
    manifest["commit"] = manifest["ref"] = commit
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"{SYNC} records {commit}")
    return 0


# --------------------------------------------------------------------------- main


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="pin-iii-sdk.sh", description=__doc__.split("\n\n")[0])
    parser.add_argument("--root", default=str(ROOT), help=argparse.SUPPRESS)
    sub = parser.add_subparsers(dest="command", required=True)

    pin = sub.add_parser("pin", help="pin every project to an iii SDK release and re-lock it")
    pin.add_argument("version", help="the iii release, e.g. 0.26.0-rc.1 or 0.26.0")
    pin.add_argument("--dry-run", action="store_true", help="print the edits and re-lock commands only")
    pin.add_argument("--only", help=f"comma-separated groups: {', '.join(GROUPS)}")
    pin.add_argument("--no-relock", action="store_true", help="write the edits without re-locking")
    pin.add_argument("--no-preflight", action="store_true", help="skip the check that the registries serve the release")
    pin.add_argument("--cargo-config", action="append", default=[], metavar="FILE",
                     help="pass --config FILE to every cargo command (a [patch] overlay, for example)")
    pin.set_defaults(func=cmd_pin)

    kit = sub.add_parser("kit", help="set the deepwork kit ranges to the first releases on the pinned SDK")
    kit.add_argument("versions", nargs="*", metavar="WORKER=VERSION", help="use this version instead of the tags")
    kit.add_argument("--dry-run", action="store_true")
    kit.add_argument("--no-fetch", action="store_true", help="use the local tags only")
    kit.add_argument("--remote", default="origin")
    kit.set_defaults(func=cmd_kit)

    template = sub.add_parser("template", help="move the synced template files to iii-hq/templates and record it")
    template.add_argument("action", choices=("export", "record"))
    template.add_argument("target", help="export: a checkout of iii-hq/templates; record: its commit")
    template.add_argument("--repo", default=TEMPLATES_REPO)
    template.set_defaults(func=cmd_template)

    argv = list(sys.argv[1:] if argv is None else argv)
    head, rest = (argv[:2], argv[2:]) if argv[:1] == ["--root"] else ([], argv)
    if rest and rest[0] not in {"pin", "kit", "template", "-h", "--help"}:
        rest.insert(0, "pin")  # `pin-iii-sdk.sh 0.26.0-rc.1` is `pin 0.26.0-rc.1`
    args = parser.parse_args(head + rest)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
