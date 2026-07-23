#!/usr/bin/env python3
"""Discover worker folders that changed between two git refs.

Outputs a JSON object to stdout AND (if $GITHUB_OUTPUT is set) writes keys:
    changed_workers (alias `all`) : all workers with any change
    source_changed                : workers whose change wasn't only metadata
    rust / node / python          : language buckets (subset of changed_workers)
    integration_changed          : bool, did an integration-stack input change
    vscode_changed                : bool, did lsp-vscode/ change
    any                           : bool, any worker or vscode change
"""
from __future__ import annotations

import argparse
import fnmatch
import json
import os
import pathlib
import subprocess
import sys

# Files inside a worker dir that DON'T count as a real source change. If the
# only files a worker touched match these globs, version-bump + tests/ gates
# in pr-checks downgrade to notices.
METADATA_GLOBS = (
    "iii.worker.yaml",
    "README.md",
    "AGENTS.md",
    "AGENTS-*.md",
    "Cargo.lock",
    "Cargo.toml",
)

# Top-level dirs we never treat as workers, regardless of contents.
IGNORE_DIRS = {".git", ".github", "registry", "target", "node_modules"}

# Special non-worker dir tracked separately for the vscode-changed gate.
VSCODE_DIR = "lsp-vscode"

# The integration suite boots this stack directly. Its path gate is separate
# from the per-worker matrices, which only contain directly changed workers.
INTEGRATION_WORKERS = {
    "harness",
    "queue",
    "session-manager",
    "context-manager",
    "iii-directory",
    "console",
}
INTEGRATION_DOC_GLOBS = (
    "README.md",
    "**/README.md",
    "AGENTS.md",
    "AGENTS-*.md",
    "**/AGENTS.md",
    "**/AGENTS-*.md",
)
INTEGRATION_INFRA_PATHS = {
    ".github/scripts/discover_changed_workers.py",
    ".github/workflows/ci.yml",
    ".github/workflows/_harness-integration.yml",
    ".github/workflows/cache-warm.yml",
}


def is_metadata(rel: str) -> bool:
    return any(fnmatch.fnmatch(rel, g) for g in METADATA_GLOBS)


def is_integration_doc(rel: str) -> bool:
    return any(fnmatch.fnmatch(rel, g) for g in INTEGRATION_DOC_GLOBS)


def list_worker_dirs(repo_root: pathlib.Path) -> set[str]:
    return {
        p.name
        for p in repo_root.iterdir()
        if p.is_dir()
        and not p.name.startswith(".")
        and p.name not in IGNORE_DIRS
        and (p / "iii.worker.yaml").exists()
    }


def changed_files(base: str, head: str) -> list[str]:
    try:
        out = subprocess.check_output(
            ["git", "diff", "--name-only", f"{base}...{head}"], text=True
        )
    except subprocess.CalledProcessError:
        out = subprocess.check_output(
            ["git", "diff", "--name-only", "HEAD~1...HEAD"], text=True
        )
    return out.splitlines()


def language_of(worker_dir: pathlib.Path) -> str | None:
    meta = worker_dir / "iii.worker.yaml"
    if not meta.exists():
        return None
    for line in meta.read_text().splitlines():
        s = line.strip()
        if s.startswith("language:"):
            lang = s.split(":", 1)[1].strip()
            if lang == "javascript":
                return "node"
            return lang
    return None


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--base", required=True, help="base git ref")
    p.add_argument("--head", default="HEAD", help="head git ref (default: HEAD)")
    p.add_argument(
        "--force-worker",
        action="append",
        default=[],
        help="include a worker even when it did not change (repeatable)",
    )
    args = p.parse_args(argv)

    repo_root = pathlib.Path(".").resolve()
    workers = list_worker_dirs(repo_root)
    files = changed_files(args.base, args.head)

    touched: dict[str, list[str]] = {}
    vscode_changed = False
    for f in files:
        parts = f.split("/", 1)
        if len(parts) < 2:
            continue
        top, rel = parts[0], parts[1]
        if top == VSCODE_DIR:
            vscode_changed = True
            continue
        if top in workers:
            touched.setdefault(top, []).append(rel)

    unknown_forced = sorted(set(args.force_worker) - workers)
    if unknown_forced:
        p.error(f"unknown --force-worker: {', '.join(unknown_forced)}")

    forced = set(args.force_worker)
    changed = sorted(set(touched) | forced)
    source_changed = sorted(
        w for w, rels in touched.items()
        if any(not is_metadata(rel) for rel in rels)
    )
    integration_changed = bool(forced & INTEGRATION_WORKERS) or any(
        f in INTEGRATION_INFRA_PATHS
        or (
            f.split("/", 1)[0] in INTEGRATION_WORKERS
            and len(f.split("/", 1)) == 2
            and not is_integration_doc(f.split("/", 1)[1])
        )
        for f in files
    )
    by_language: dict[str, list[str]] = {"rust": [], "node": [], "python": []}
    for w in changed:
        lang = language_of(repo_root / w)
        if lang in by_language:
            by_language[lang].append(w)
        else:
            print(f"::warning::{w} has unknown language={lang}", file=sys.stderr)

    payload = {
        "changed_workers": changed,
        "source_changed": source_changed,
        "by_language": by_language,
        "integration_changed": integration_changed,
        "vscode_changed": vscode_changed,
    }
    print(json.dumps(payload))

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        any_change = bool(changed) or vscode_changed
        with open(gh_out, "a") as f:
            f.write(f"changed_workers={json.dumps(changed)}\n")
            # Back-compat alias used by ci.yml downstream matrix jobs.
            f.write(f"all={json.dumps(changed)}\n")
            f.write(f"source_changed={json.dumps(source_changed)}\n")
            for lang in ("rust", "node", "python"):
                f.write(f"{lang}={json.dumps(by_language[lang])}\n")
            f.write(
                f"integration_changed={'true' if integration_changed else 'false'}\n"
            )
            f.write(f"vscode_changed={'true' if vscode_changed else 'false'}\n")
            f.write(f"any={'true' if any_change else 'false'}\n")

    return 0


if __name__ == "__main__":
    sys.exit(main())
