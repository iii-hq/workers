"""Judge prompt rows from session-manager corpora: python3 -I extract.py <out_dir> <dir>..."""

import hashlib
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

NOTIFICATION = "[notification]"


def text(message: dict) -> str:
    return "".join(b.get("text", "") for b in message.get("content") or [] if b.get("type") == "text")


def messages(lines: list[dict]) -> list[dict]:
    """The session's final messages in first-append order: the highest revision of each entry wins."""
    latest = {}
    for line in lines:
        e = line.get("entry")
        if e and (e["id"] not in latest or e["revision"] > latest[e["id"]]["revision"]):
            latest[e["id"]] = e
    return [
        {**e["message"], "id": e["id"], "timestamp": e["timestamp"]} for e in latest.values() if e["kind"] == "message"
    ]


def is_turn_start(message: dict) -> bool:
    return message.get("role") == "user" and bool(text(message).strip())


def session_rows(path: Path, source: str) -> list[dict]:
    lines = common.read_jsonl(path)
    meta = next((line["meta"] for line in reversed(lines) if "meta" in line), {})
    metadata = meta.get("metadata") or {}
    sid = meta.get("session_id", path.stem)
    child = bool(metadata.get("parent_session_id"))
    turns = []  # (start message, its assistant messages, context, gap_ms)
    prev_user, prev_reply, prev_ts = "", "", None
    for m in messages(lines):
        if is_turn_start(m):
            parts = (("user", prev_user[-600:]), ("assistant", prev_reply[-1500:]))
            context = "\n".join(f"{role}: {part}" for role, part in parts if part)
            turns.append((m, [], context, None if prev_ts is None else m["timestamp"] - prev_ts))
            prev_user = text(m)
        elif m.get("role") == "assistant":
            if turns:
                turns[-1][1].append(m)
            prev_ts = m["timestamp"]
            if text(m).strip():
                prev_reply = text(m)
    human = [turn for turn in turns if not text(turn[0]).startswith(NOTIFICATION)]
    rows = []
    for seq, (m, steps, context, gap_ms) in enumerate(human[:1] if child else human):
        first = (steps[0].get("usage") or {}) if steps else {}
        rows.append(
            {
                "id": hashlib.sha1((sid + m["id"]).encode()).hexdigest()[:12],
                "source": source,
                "slice": "child" if child else "top",
                "session_id": sid,
                "seq": seq,
                "message": text(m)[:4000],
                "context": context,
                "model": (steps[0].get("model") if steps else None) or metadata.get("model"),
                "level": metadata.get("thinking_level"),
                "gap_ms": gap_ms,
                "cost_units": sum((common.units(step.get("usage")) for step in steps), 0.0),
                "steps": len(steps),
                "first_read": first.get("cache_read") or 0,
            }
        )
    return rows


def prompts(dirs: list[Path]) -> list[dict]:
    rows, seen = [], set()
    for d in map(Path, dirs):
        source = d.absolute().parent.parent.name
        for path in sorted(d.glob("*.jsonl")):
            for row in session_rows(path, source):
                key = (row["slice"], row["message"])
                if key not in seen:
                    seen.add(key)
                    rows.append(row)
    return rows


def main(argv: list[str]) -> None:
    if len(argv) < 2:
        sys.exit("usage: python3 -I extract.py <out_dir> <dir>...")
    rows = prompts([Path(d) for d in argv[1:]])
    common.write_jsonl(Path(argv[0]) / "prompts.jsonl", rows)
    for (slice_, source), n in sorted(Counter((r["slice"], r["source"]) for r in rows).items()):
        print(f"{slice_}\t{source}\t{n}")
    for slice_, n in sorted(Counter(r["slice"] for r in rows).items()):
        print(f"{slice_}\ttotal\t{n}")


if __name__ == "__main__":
    main(sys.argv[1:])
