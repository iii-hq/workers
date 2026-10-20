"""Fresh G1 prompts from Claude Code transcripts (phase 0b):
python3 -I cc_extract.py <out_dir> <transcript.jsonl>... [--n 300] [--per-session 5] [--seed 20261019] [--skip <session>]

A row is a typed human prompt; its level and model are those of the assistant reply that answered it (Claude Code
records `effort` on every assistant line; max reads as xhigh). Prompts that look like secrets are dropped. The sample
takes at most --per-session prompts per session, then --n in all, in an order fixed by --seed. Writes
<out_dir>/prompts.jsonl with slice "cc", in the shape extract.py writes, so label.py, judge.py and analyze.py
(--g1 cc) read it unchanged.
"""

import argparse
import hashlib
import json
import random
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

SECRET = re.compile(
    r"sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-|AKIA[0-9A-Z]{16}|-----BEGIN [A-Z ]*PRIVATE KEY"
    r"|eyJ[A-Za-z0-9_-]{20,}\.|(?i:api[_-]?key|token|secret|password)\s*[:=]\s*\S{8,}"
)
INJECTED = ("<", "Caveat:", "[Request interrupted")


def text_of(content) -> str | None:
    """A message's text, or None when it holds anything but text blocks (tool results, images)."""
    if isinstance(content, str):
        return content
    if isinstance(content, list) and all(isinstance(b, dict) and b.get("type") == "text" for b in content):
        return "\n".join(b.get("text", "") for b in content)
    return None


def is_prompt(line: dict) -> str | None:
    """The text of a typed human prompt, or None."""
    if line.get("type") != "user" or line.get("isSidechain") or line.get("isMeta"):
        return None
    if line.get("isCompactSummary") or line.get("isVisibleInTranscriptOnly"):
        return None
    if (line.get("origin") or {}).get("kind") not in (None, "human"):
        return None
    text = text_of((line.get("message") or {}).get("content"))
    if not text or not text.strip() or text.lstrip().startswith(INJECTED) or SECRET.search(text):
        return None
    return text


def level_of(effort) -> str | None:
    effort = "xhigh" if effort == "max" else effort
    return effort if effort in common.LEVELS else None


def session_rows(lines: list[dict]) -> list[dict]:
    """Every prompt row of one transcript, in order."""
    rows, last_user, last_reply, pending = [], "", "", None
    for line in lines:
        prompt = is_prompt(line)
        if prompt is not None:
            context = f"user: {last_user[-600:]}\nassistant: {last_reply[-1500:]}" if last_user or last_reply else ""
            sid = line.get("sessionId") or line.get("session_id") or ""
            pending = {
                "id": hashlib.sha1(f"{sid}{line.get('uuid')}".encode()).hexdigest()[:12],
                "source": "claude-code",
                "slice": "cc",
                "session_id": sid,
                "seq": len(rows),
                "message": prompt[:4000],
                "context": context,
                "model": None,
                "level": None,
                "level_source": None,
                "gap_ms": None,
                "cost_units": 0.0,
                "steps": 0,
                "first_read": 0,
            }
            rows.append(pending)
            last_user = prompt
            continue
        if line.get("type") != "assistant":
            continue
        message = line.get("message") or {}
        if pending is not None and pending["level"] is None and "effort" in line:
            pending["level"] = level_of(line["effort"])
            pending["level_source"] = "assistant_effort" if pending["level"] else None
            pending["model"] = message.get("model")
        reply = text_of([b for b in message.get("content") or [] if isinstance(b, dict) and b.get("type") == "text"])
        if reply:
            last_reply = reply
    return rows


def sample(rows: list[dict], n: int, per_session: int, seed: int) -> list[dict]:
    """Up to per_session distinct prompts from each session, then n in all; the same inputs give the same sample."""
    rng = random.Random(seed)
    seen, by_session = set(), {}
    for row in rows:
        if row["message"] not in seen:
            seen.add(row["message"])
            by_session.setdefault(row["session_id"], []).append(row)
    pool = []
    for sid in sorted(by_session):
        group = by_session[sid]
        rng.shuffle(group)
        pool += group[:per_session]
    rng.shuffle(pool)
    return pool[:n]


def main(argv: list[str]) -> None:
    parser = argparse.ArgumentParser(prog="cc_extract.py")
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("transcripts", type=Path, nargs="+")
    parser.add_argument("--n", type=int, default=300)
    parser.add_argument("--per-session", type=int, default=5)
    parser.add_argument("--seed", type=int, default=20261019)
    parser.add_argument("--skip", action="append", default=[], help="a session id to leave out")
    args = parser.parse_args(argv)
    rows = []
    for path in sorted(args.transcripts):
        if path.stem in args.skip:
            continue
        lines = []
        for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
            try:
                lines.append(json.loads(raw))
            except ValueError:
                continue
        rows += [r for r in session_rows(lines) if r["session_id"] not in args.skip]
    picked = sample(rows, args.n, args.per_session, args.seed)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    common.write_jsonl(args.out_dir / "prompts.jsonl", picked)
    sessions = len({r["session_id"] for r in picked})
    levels = {lv: sum(r["level"] == lv for r in picked) for lv in [*common.LEVELS, None]}
    print(f"{len(rows)} prompts in {len(args.transcripts)} transcripts; sampled {len(picked)} from {sessions} sessions")
    print("levels:", ", ".join(f"{lv}: {n}" for lv, n in levels.items()))


if __name__ == "__main__":
    main(sys.argv[1:])
