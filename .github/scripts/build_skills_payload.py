#!/usr/bin/env python3
"""Build the POST /w/<slug>/skills payload from a worker directory.

Walks the optional ``<worker>/skills/SKILL.md`` entrypoint plus every other
``<worker>/skills/**/*.md`` document and produces the JSON body expected by
the workers-registry endpoint. Skill paths map to keys as:

    <worker>/skills/SKILL.md      -> "SKILL.md"
    <worker>/skills/<rel>.md      -> "skills/<rel>.md"  (except SKILL.md)

The payload always carries the complete skills snapshot, including
``skills: {}`` when no non-empty markdown exists. Publishing that explicit
empty snapshot is idempotent and lets retries prove that the exact version has
no attached skills before a mutable Registry channel is assigned.
"""
import argparse
import json
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

    The optional worker overview is published as registry key ``SKILL.md``,
    sourced from ``skills/SKILL.md`` when present. Other markdown documents do
    not require that overview. Empty bodies are skipped silently so blank
    placeholder files don't end up in the registry.
    """
    skills: dict[str, str] = {}

    leaves_dir = worker_root / "skills"
    skills_skill = leaves_dir / "SKILL.md"
    markdown = sorted(leaves_dir.rglob("*.md")) if leaves_dir.is_dir() else []

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

    payload = {"version": args.version, "skills": skills}
    out_path = pathlib.Path(args.out)
    out_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"::notice::collected {len(skills)} skill file(s) for {args.worker}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
