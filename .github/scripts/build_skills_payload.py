#!/usr/bin/env python3
"""Build the POST /w/<slug>/skills payload from a worker directory.

Walks the canonical ``<worker>/skills/SKILL.md`` entrypoint plus
``<worker>/skills/**/*.md`` and produces the JSON body expected by the
workers-registry endpoint. Skill paths map to keys as:

    <worker>/skills/SKILL.md      -> "SKILL.md"
    <worker>/skills/<rel>.md      -> "skills/<rel>.md"  (except SKILL.md)

If no non-empty markdown is found the script writes ``skip=true`` to
``$GITHUB_OUTPUT`` (so the calling workflow can gate the POST step off) and
exits cleanly; the API rejects payloads that omit both ``skills`` and
``prompts``, and ``skills: {}`` would be a destructive "clear all" call which is
wrong on a fresh publish.
"""
import argparse
import json
import os
import pathlib
import re
import sys


KEY_RE = re.compile(r"^[a-z0-9][a-z0-9._/\-]*\.md$", re.IGNORECASE)

TOP_SKILL_KEY = "SKILL.md"


def _read_nonempty(path: pathlib.Path) -> str | None:
    body = path.read_text(encoding="utf-8")
    return body if body.strip() else None


def collect_skills(worker_root: pathlib.Path) -> dict[str, str]:
    """Return a ``{payload-key: markdown-body}`` map for one worker directory.

    The worker overview is always published as registry key ``SKILL.md``,
    sourced from ``skills/SKILL.md`` when present.  Empty bodies are skipped
    silently so blank placeholder files don't end up in the registry.
    """
    skills: dict[str, str] = {}

    leaves_dir = worker_root / "skills"
    skills_skill = leaves_dir / "SKILL.md"
    markdown = sorted(leaves_dir.rglob("*.md")) if leaves_dir.is_dir() else []
    if markdown and not skills_skill.is_file():
        raise ValueError(f"{worker_root.name}/skills/SKILL.md is required when skill documents are present")

    top_body = _read_nonempty(skills_skill) if skills_skill.is_file() else None
    if top_body is not None:
        skills[TOP_SKILL_KEY] = top_body

    if markdown:
        for path in markdown:
            if path == skills_skill:
                continue
            rel = path.relative_to(worker_root).as_posix()
            if not KEY_RE.match(rel):
                raise ValueError(
                    f"skill path rejected by server regex: {rel} "
                    "(must match /^[a-z0-9][a-z0-9._/\\-]*\\.md$/i)"
                )
            body = path.read_text(encoding="utf-8")
            if not body.strip():
                continue
            skills[rel] = body

    return skills


def _signal_skip(worker: str) -> None:
    gha_out = os.environ.get("GITHUB_OUTPUT")
    if gha_out:
        with open(gha_out, "a", encoding="utf-8") as f:
            f.write("skip=true\n")
    print(f"::notice::no skills found for {worker}; skipping POST /w/.../skills")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True, help="Worker folder name in the repo root")
    parser.add_argument("--version", required=True, help="Exact semver to attach this snapshot to")
    parser.add_argument("--out", default="skills-payload.json")
    parser.add_argument(
        "--repo-root",
        default=".",
        help="Repo root containing the worker folder (default: cwd).",
    )
    args = parser.parse_args()

    worker_root = pathlib.Path(args.repo_root) / args.worker
    if not worker_root.is_dir():
        print(f"::error::worker directory not found: {worker_root}", file=sys.stderr)
        return 1

    try:
        skills = collect_skills(worker_root)
    except ValueError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1

    out_path = pathlib.Path(args.out)
    if not skills:
        out_path.write_text("{}\n", encoding="utf-8")
        _signal_skip(args.worker)
        return 0

    payload = {"version": args.version, "skills": skills}
    out_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"::notice::collected {len(skills)} skill file(s) for {args.worker}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
