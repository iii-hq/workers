#!/usr/bin/env python3
"""
coder error-frequency instrumentation — counts C2xx errors by (code, function_id).

PURPOSE
-------
Continuous signal that coder 0.3.0 DX fixes hold. The target trend: C210
rate on coder calls approaches zero for post-0.3.0 sessions; C211/C213 are
expected-recoverable and some recurrence is normal.

BASELINE (pre-0.3.0)
--------------------
Session vqrfg31f (2026-06-09): agent's first 3 coder::create-file calls all
returned C210 ("path must be relative to base_path: /tmp/..."). The agent
abandoned the tool entirely and fell back to shell::exec to write files.
Root cause: no absolute-path support and no prescriptive guidance — the error
gave no recovery instruction. Fixed in 0.3.0 (absolute paths now accepted when
inside an allowed root; C215 names all roots; C210 is structured + prescriptive).

ERROR SHAPES HANDLED
--------------------
Structured (0.3.0+):
  entry.error = {"code": "C210", "message": "..."}          # dict already parsed
  entry.error = '{"code":"C210","message":"..."}'            # JSON string

Legacy (pre-0.3.0):
  entry.error = "path must be relative to base_path: ..."   # bare string, no code

SUCCESS METRIC
--------------
  post-0.3.0 C210 rate on coder:: calls → 0
  C211 / C213 expected-recoverable; watch for spikes

USAGE
-----
# Live engine (requires `iii` CLI + running engine):
  python3 scripts/error-frequency.py --live [--sessions N]

# Session export markdown files:
  python3 scripts/error-frequency.py ~/Downloads/iii-session-*.md

# Both:
  python3 scripts/error-frequency.py --live ~/Downloads/iii-session-*.md

# Built-in self-test (no external deps):
  python3 scripts/error-frequency.py --self-test

# Filter to one session via live engine:
  python3 scripts/error-frequency.py --live --session-id console-vqrfg31f4zemq74s5ia

LIMITATIONS
-----------
Live-mode attribution uses the last-seen coder function from the preceding
assistant message — session-tree function_result parts carry no call_id — so
two different coder::* functions dispatched in the same turn may be
misattributed to each other. Counts per code remain correct either way.

SESSION-TREE LIVE-MODE RECIPE (for future automation)
------------------------------------------------------
  iii trigger session-tree::list
  iii trigger session-tree::messages --json '{"session_id":"<id>"}'
Messages are returned as [{entry_id, message: {content: [{text, type}]}}].
The `text` field is a JSON string; parse it to reach .results[].error.
The session-tree worker is "session" in the engine's worker list.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict
from typing import Iterator

# ---------------------------------------------------------------------------
# Error extraction helpers
# ---------------------------------------------------------------------------

_C2XX_RE = re.compile(r'"C2\d{2}"')


def _parse_error(raw_error) -> tuple[str, str]:
    """Return (code, message) from any error shape coder has ever emitted."""
    if raw_error is None:
        return ("?", "")

    # Already a dict — structured 0.3.0 shape
    if isinstance(raw_error, dict):
        return (str(raw_error.get("code", "?")), str(raw_error.get("message", "")))

    if not isinstance(raw_error, str):
        return ("?", str(raw_error))

    s = raw_error.strip()
    if not s:
        return ("?", "")

    # JSON string containing structured error
    if s.startswith("{"):
        try:
            obj = json.loads(s)
            if isinstance(obj, dict) and "code" in obj:
                return (str(obj["code"]), str(obj.get("message", "")))
        except json.JSONDecodeError:
            pass

    # Legacy bare string — classify as LEGACY
    return ("LEGACY", s[:120])


def _extract_from_results(results, function_id: str) -> list[dict]:
    """Walk results[] or files[] arrays; yield {code, message, function_id}."""
    hits = []
    if not isinstance(results, list):
        return hits
    for entry in results:
        if not isinstance(entry, dict):
            continue
        err = entry.get("error") or entry.get("err")
        if not err:
            continue
        code, msg = _parse_error(err)
        if code.startswith("C2") or code == "LEGACY":
            hits.append({"code": code, "message": msg, "function_id": function_id})
    return hits


def _try_parse_text(text: str) -> dict | list | None:
    """Best-effort: parse a possibly-double-encoded JSON text field."""
    if not isinstance(text, str) or not text.strip():
        return None
    t = text.strip()
    try:
        obj = json.loads(t)
        if isinstance(obj, str):
            # Double-encoded — parse once more
            try:
                obj = json.loads(obj)
            except json.JSONDecodeError:
                pass
        return obj
    except json.JSONDecodeError:
        return None


# ---------------------------------------------------------------------------
# Source: live engine via `iii trigger`
# ---------------------------------------------------------------------------

def _iii(*args: str, timeout: int = 30) -> dict | list | None:
    """Run `iii trigger <args>` and return parsed JSON, or None on failure."""
    cmd = ["iii", "trigger", *args]
    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout
        )
    except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
        print(f"[warn] iii CLI unavailable: {exc}", file=sys.stderr)
        return None

    if result.returncode != 0:
        print(f"[warn] iii trigger {args[0]} failed: {result.stderr[:200]}", file=sys.stderr)
        return None

    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        print(f"[warn] unparseable iii output for {args[0]}", file=sys.stderr)
        return None


def _live_sessions(limit: int | None = None) -> list[dict]:
    data = _iii("session-tree::list")
    if not data or not isinstance(data, dict):
        return []
    sessions = data.get("sessions", [])
    if limit:
        sessions = sessions[:limit]
    return sessions


def _live_messages(session_id: str) -> list[dict]:
    data = _iii("session-tree::messages", "--json", json.dumps({"session_id": session_id}))
    if not data or not isinstance(data, dict):
        return []
    return data.get("messages", [])


def _hits_from_live_messages(messages: list[dict], session_id: str) -> Iterator[dict]:
    """Extract C2xx coder errors from a session-tree::messages response.

    Message ordering: assistant messages carry function_call parts that name the
    target function (e.g. coder::create-file). The immediately following
    function_result message carries the output. We build a pending-call map so
    result messages can be attributed to the correct function_id.
    """
    # Map: call_id → target function_id, built from assistant messages
    call_to_fn: dict[str, str] = {}
    # Most recently dispatched coder function (fallback when call_id not in map)
    last_coder_fn: str = "coder::?"

    for m in messages:
        msg = m.get("message", {})
        role = msg.get("role", "")
        content = msg.get("content", [])
        if not isinstance(content, list):
            continue

        if role == "assistant":
            # Record function_call targets for future result attribution
            for part in content:
                if not isinstance(part, dict):
                    continue
                if part.get("type") != "function_call":
                    continue
                call_id = part.get("id", "")
                target = part.get("arguments", {}).get("function", "")
                if call_id:
                    call_to_fn[call_id] = target
                if target.startswith("coder::"):
                    last_coder_fn = target

        elif role == "function_result":
            for part in content:
                if not isinstance(part, dict):
                    continue
                if part.get("type") != "text":
                    continue
                obj = _try_parse_text(part.get("text", ""))
                if not isinstance(obj, dict):
                    continue

                # Infer which coder function produced this result.
                # function_result messages in session-tree don't carry a call_id
                # at the part level; use last_coder_fn as the best attribution.
                fn_id = last_coder_fn

                for key in ("results", "files"):
                    for h in _extract_from_results(obj.get(key, []), fn_id):
                        h["session_id"] = session_id
                        yield h


def scan_live(session_ids: list[str] | None = None, limit: int | None = None) -> list[dict]:
    """Query live engine; return list of {code, function_id, session_id} dicts."""
    if session_ids:
        sessions = [{"session_id": sid} for sid in session_ids]
    else:
        sessions = _live_sessions(limit=limit)

    if not sessions:
        print("[warn] no sessions returned from live engine", file=sys.stderr)
        return []

    all_hits = []
    for s in sessions:
        sid = s.get("session_id", "")
        messages = _live_messages(sid)
        for h in _hits_from_live_messages(messages, sid):
            all_hits.append(h)

    return all_hits


# ---------------------------------------------------------------------------
# Source: session export markdown files
# ---------------------------------------------------------------------------

_TOOL_CALL_RE = re.compile(r"^##\s+Tool call\s+[—–-]\s+([\w::-]+)", re.MULTILINE)
_OUTPUT_BLOCK_RE = re.compile(
    r"\*\*Output:\*\*\s*```(?:json)?\s*([\s\S]*?)```", re.MULTILINE
)


def _parse_markdown(path: str) -> Iterator[dict]:
    """Yield {code, message, function_id, source_file} from a session export .md."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as exc:
        print(f"[warn] cannot read {path}: {exc}", file=sys.stderr)
        return

    # Split into sections by tool call headings; pair each with its output block
    sections = list(_TOOL_CALL_RE.finditer(text))
    for i, match in enumerate(sections):
        fn_id = match.group(1)
        if not fn_id.startswith("coder::"):
            continue

        # Region: from end of this heading to start of next (or EOF)
        start = match.end()
        end = sections[i + 1].start() if i + 1 < len(sections) else len(text)
        section_text = text[start:end]

        for out_match in _OUTPUT_BLOCK_RE.finditer(section_text):
            block = out_match.group(1).strip()
            obj = _try_parse_text(block)
            if not isinstance(obj, dict):
                continue

            # Output block may be the outer {content:[{text:"JSON"}]} envelope
            # or already the inner {results:[]} dict — handle both
            inner = obj
            content = obj.get("content")
            if isinstance(content, list):
                for item in content:
                    if isinstance(item, dict) and item.get("type") == "text":
                        parsed = _try_parse_text(item.get("text", ""))
                        if isinstance(parsed, dict):
                            inner = parsed
                            break

            # Prefer explicit .details over inner (avoids double-counting when
            # the outer envelope has both content[].text and .details keys).
            # Fall back to inner dict if no .details key is present.
            details = obj.get("details")
            if isinstance(details, dict):
                source = details
            else:
                source = inner

            for key in ("results", "files"):
                for h in _extract_from_results(source.get(key, []), fn_id):
                    h["source_file"] = path
                    yield h


def scan_files(paths: list[str]) -> list[dict]:
    hits = []
    for p in paths:
        for h in _parse_markdown(p):
            hits.append(h)
    return hits


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------

def _tabulate(hits: list[dict], source_label: str) -> None:
    if not hits:
        print(f"\n[{source_label}] no C2xx errors found\n")
        return

    counts: dict[tuple[str, str], int] = defaultdict(int)
    for h in hits:
        counts[(h["code"], h["function_id"])] += 1

    total = sum(counts.values())
    print(f"\n[{source_label}] {total} C2xx error(s) across {len(hits)} entries")
    print(f"  {'CODE':<8}  {'FUNCTION':<30}  COUNT")
    print(f"  {'-'*8}  {'-'*30}  -----")
    for (code, fn), cnt in sorted(counts.items(), key=lambda x: (-x[1], x[0])):
        print(f"  {code:<8}  {fn:<30}  {cnt}")
    print()


# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------

SELF_TEST_MD = """\
# Session: self-test

- ID: `selftest`

---
## Tool call — coder::create-file
**Input:**
```json
{"files": [{"path": "/tmp/x", "content": "hi"}]}
```
**Output:**
```json
{
  "content": [
    {
      "text": "{\\"results\\":[{\\"bytes_written\\":0,\\"error\\":\\"{\\\\\\"code\\\\\\":\\\\\\"C210\\\\\\",\\\\\\"message\\\\\\":\\\\\\"path must be relative to base_path: /tmp/x\\\\\\"}\\",\\"path\\":\\"/tmp/x\\",\\"success\\":false},{\\"bytes_written\\":0,\\"error\\":\\"{\\\\\\"code\\\\\\":\\\\\\"C210\\\\\\",\\\\\\"message\\\\\\":\\\\\\"path must be relative to base_path: /tmp/y\\\\\\"}\\",\\"path\\":\\"/tmp/y\\",\\"success\\":false},{\\"bytes_written\\":0,\\"error\\":\\"{\\\\\\"code\\\\\\":\\\\\\"C210\\\\\\",\\\\\\"message\\\\\\":\\\\\\"path must be relative to base_path: /tmp/z\\\\\\"}\\",\\"path\\":\\"/tmp/z\\",\\"success\\":false}]}",
      "type": "text"
    }
  ],
  "details": {
    "results": [
      {"bytes_written": 0, "error": "{\\"code\\":\\"C210\\",\\"message\\":\\"path must be relative to base_path: /tmp/x\\"}", "path": "/tmp/x", "success": false},
      {"bytes_written": 0, "error": "{\\"code\\":\\"C210\\",\\"message\\":\\"path must be relative to base_path: /tmp/y\\"}", "path": "/tmp/y", "success": false},
      {"bytes_written": 0, "error": "{\\"code\\":\\"C210\\",\\"message\\":\\"path must be relative to base_path: /tmp/z\\"}", "path": "/tmp/z", "success": false}
    ]
  },
  "terminate": false
}
```

## Tool call — coder::read-file
**Input:**
```json
{"path": "secret.pem"}
```
**Output:**
```json
{
  "details": {
    "results": [
      {"error": {"code": "C211", "message": "non_accessible: secret.pem"}, "path": "secret.pem", "success": false}
    ]
  }
}
```
"""


def self_test() -> int:
    import tempfile, os

    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".md", delete=False, encoding="utf-8"
    ) as f:
        f.write(SELF_TEST_MD)
        tmp_path = f.name

    try:
        hits = scan_files([tmp_path])
    finally:
        os.unlink(tmp_path)

    c210 = [h for h in hits if h["code"] == "C210" and h["function_id"] == "coder::create-file"]
    c211 = [h for h in hits if h["code"] == "C211" and h["function_id"] == "coder::read-file"]

    ok = True
    if len(c210) != 3:
        print(f"FAIL: expected 3 C210 on coder::create-file, got {len(c210)}")
        print("  hits:", json.dumps(hits, indent=2))
        ok = False
    if len(c211) != 1:
        print(f"FAIL: expected 1 C211 on coder::read-file, got {len(c211)}")
        ok = False

    if ok:
        print("PASS: self-test — 3x C210 coder::create-file + 1x C211 coder::read-file detected")
        _tabulate(hits, "self-test")
    return 0 if ok else 1


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="Count coder C2xx errors by (code, function_id) from session exports or live engine.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("USAGE")[1].split("SESSION-TREE")[0] if "USAGE" in __doc__ else "",
    )
    parser.add_argument(
        "files",
        nargs="*",
        metavar="FILE",
        help="Session export markdown file(s) (.md) — output of iii session export",
    )
    parser.add_argument(
        "--live",
        action="store_true",
        help="Query live engine via `iii trigger session-tree::*`",
    )
    parser.add_argument(
        "--sessions",
        type=int,
        default=20,
        metavar="N",
        help="Max sessions to scan from live engine (default: 20)",
    )
    parser.add_argument(
        "--session-id",
        action="append",
        dest="session_ids",
        metavar="ID",
        help="Scan a specific session id (repeatable; implies --live)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run built-in fixture test and exit (no external deps required)",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    all_hits: list[dict] = []

    if args.files:
        file_hits = scan_files(args.files)
        all_hits.extend(file_hits)
        label = f"files: {', '.join(args.files)}"
        _tabulate(file_hits, label)

    if args.live or args.session_ids:
        live_hits = scan_live(
            session_ids=args.session_ids,
            limit=args.sessions if not args.session_ids else None,
        )
        all_hits.extend(live_hits)
        _tabulate(live_hits, "live engine")

    if not args.files and not args.live and not args.session_ids:
        parser.print_help()
        return 1

    if args.files and (args.live or args.session_ids):
        _tabulate(all_hits, "TOTAL (files + live)")

    return 0


if __name__ == "__main__":
    sys.exit(main())
