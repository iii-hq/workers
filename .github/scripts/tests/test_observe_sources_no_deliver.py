"""No observe source in this repo sets the removed `deliver` field.

iii 0.26 (`observe.capture`) delivers one shape to every observer target:
the envelope `{ status, input?, output?, caller?, ... }` whose fields are
the observer's `capture`. The raw `deliver: output` / `deliver: input`
modes are gone (an engine that still accepts them only translates them,
with a deprecation warning), so every observe source asks for the fields
it needs with `capture` and its target reads `payload.output` /
`payload.input`. This guard fails on any `deliver` left on a source, in
any language, production or test.
"""
from __future__ import annotations

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

PATTERNS = {
    ".rs": [
        re.compile(r"\.deliver\(\s*(?:iii_sdk::)?Deliver::"),
        re.compile(r"\bDeliver::(?:Output|Input|Envelope)\b"),
        re.compile(r"\bdeliver:\s*'(?:output|input|envelope)'"),
    ],
    ".ts": [re.compile(r"\bdeliver:\s*['\"](?:output|input|envelope)['\"]")],
    ".go": [re.compile(r"\bDeliver:\s+(?:iii\.Deliver\w+|\"(?:output|input|envelope)\")")],
    ".py": [re.compile(r"[\"']deliver[\"']\s*:\s*[\"'](?:output|input)[\"']")],
}
PATTERNS[".tsx"] = PATTERNS[".mjs"] = PATTERNS[".js"] = PATTERNS[".ts"]

# Files that mention `deliver` without creating an observer: readers of
# observer listings / views from engines that still report the legacy
# field, and validators of what a client sends. Each entry says why.
ALLOWED: dict[str, str] = {
    "code-runner/src/node_bus.rs": (
        "forwards a guest JS observe `deliver` (deprecated) to the engine as the "
        "legacy source field, which the 0.26 engine translates, with a warning "
        "naming `capture`; it creates no `deliver` observer of its own"
    ),
    "crates/node-core/src/engine.rs": (
        "defines and parses GuestDeliver, the deprecated guest JS `deliver` that "
        "code-runner forwards as the legacy source field"
    ),
    "rbac-proxy/src/interceptor.rs": (
        "reads `source.deliver` from client observe frames it proxies; it is a "
        "validator of what a client sends, not an observe source"
    ),
}

COMMENT = re.compile(r"^\s*(?://|\*|/\*|#)")


def tracked_files() -> list[str]:
    out = subprocess.check_output(["git", "ls-files"], cwd=ROOT, text=True)
    return [
        f
        for f in out.splitlines()
        if Path(f).suffix in PATTERNS
        and "/node_modules/" not in f
        and "/vendor/" not in f
        and not f.startswith(".github/scripts/tests/test_observe_sources_no_deliver")
    ]


def offending_lines() -> list[str]:
    hits: list[str] = []
    for rel in tracked_files():
        if rel in ALLOWED:
            continue
        path = ROOT / rel
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError):
            continue
        for number, line in enumerate(text.splitlines(), 1):
            if COMMENT.match(line):
                continue
            if any(p.search(line) for p in PATTERNS[path.suffix]):
                hits.append(f"{rel}:{number}: {line.strip()}")
    return hits


def test_no_observe_source_sets_deliver() -> None:
    hits = offending_lines()
    assert not hits, (
        f"{len(hits)} observe source(s) still set the removed `deliver`; "
        "use `capture` and read payload.output / payload.input instead:\n"
        + "\n".join(hits)
    )


def test_allowlist_entries_exist() -> None:
    missing = [rel for rel in ALLOWED if not (ROOT / rel).exists()]
    assert not missing, f"stale allowlist entries: {missing}"


# Markdown: an observe example inside a fenced code block teaches readers to
# set `deliver`, so the same per-language patterns apply to fence bodies.
# Prose outside fences is not scanned (it may describe the legacy field).
MD_PATTERNS = [p for suffix in (".rs", ".ts", ".go", ".py") for p in PATTERNS[suffix]]
MD_FENCE = re.compile(r"^\s*(`{3,}|~{3,})")

# Markdown trees whose fences may still show `deliver`. Each entry says why.
MD_ALLOWED: dict[str, str] = {
    "tech-specs/": "dated design records of the observe API as it was specified",
    "template/skills/": (
        "vendored from the upstream template by template/sync.sh and pinned by "
        "hash in template/upstream/sync.json; fixed upstream, then re-synced"
    ),
}


def markdown_offending_lines() -> list[str]:
    out = subprocess.check_output(["git", "ls-files", "*.md"], cwd=ROOT, text=True)
    hits: list[str] = []
    for rel in out.splitlines():
        if "/node_modules/" in rel or any(rel.startswith(p) for p in MD_ALLOWED):
            continue
        try:
            text = (ROOT / rel).read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError):
            continue
        fence: str | None = None
        for number, line in enumerate(text.splitlines(), 1):
            opened = MD_FENCE.match(line)
            if opened:
                marker = opened.group(1)
                if fence is None:
                    fence = marker
                    continue
                closing = line.strip()
                if closing == closing[0] * len(closing) and closing[0] == fence[0] and len(closing) >= len(fence):
                    fence = None
                    continue
            if fence and any(p.search(line) for p in MD_PATTERNS):
                hits.append(f"{rel}:{number}: {line.strip()}")
    return hits


def test_no_markdown_observe_example_sets_deliver() -> None:
    hits = markdown_offending_lines()
    assert not hits, (
        f"{len(hits)} markdown code example(s) still set the removed `deliver`; "
        "show `capture` and a target reading payload.output / payload.input:\n"
        + "\n".join(hits)
    )


def test_markdown_allowlist_entries_exist() -> None:
    missing = [prefix for prefix in MD_ALLOWED if not (ROOT / prefix).is_dir()]
    assert not missing, f"stale markdown allowlist entries: {missing}"
