"""Label harness prompts with the efforts that would do the job: python3 -I label.py <out_dir> [--run]

Without --run it prints the call count, estimated tokens and their cost. Prompts already in labels.jsonl are skipped,
so a re-run labels only the rest; delete labels.jsonl to label again from scratch. A --run logs every reply and
prompt that yields no label to stderr, and stops once 5 prompts in a row got none.
"""

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import extract  # noqa: E402

HERE = Path(__file__).resolve().parent
RUBRIC = HERE / "rubric.md"
CASES = HERE / "cases" / "effortless-judge-cases.json"
MODELS = {"sonnet": "claude-sonnet-5-5", "opus": "claude-opus-5-5"}
LABELLER = "claude-opus-5-5"
OUTPUT_TOKENS = 800  # a labeller call's output, thinking included, for the estimate


def parse_label(text: str) -> list[str] | None:
    """The `ok` levels of a {"ok": [...]} reply, ordered as in LEVELS; None when empty or unknown.

    The whole reply is tried as JSON first, then the value at each `{` in turn: the first object holding "ok" counts,
    whatever prose or braces surround it.
    """
    decoder = json.JSONDecoder()
    for at in (None, *(i for i, char in enumerate(text) if char == "{")):
        try:
            found = json.loads(text) if at is None else decoder.raw_decode(text, at)[0]
        except ValueError:
            continue
        if isinstance(found, dict) and "ok" in found:
            ok = found["ok"]
            if isinstance(ok, list) and ok and all(level in common.LEVELS for level in ok):
                return sorted(set(ok), key=common.LEVELS.index)
            return None
    return None


def needs_adjudication(a: list[str], b: list[str]) -> bool:
    return a[0] != b[0]


def effortless_rows() -> list[dict]:
    cases = json.loads(CASES.read_text(encoding="utf-8"))["cases"]
    return [
        {
            "id": "eff-" + case["id"],
            "slice": "effortless",
            "holdout": bool(case.get("holdout")),
            "model": MODELS[case["current"]["model"]],
            "level": case["current"]["effort"],
            "context": case.get("context") or "",
            "message": case["message"],
            "ok": sorted(case["ok"], key=common.LEVELS.index),  # some cases list a preferred level first
        }
        for case in cases
    ]


def labeller_input(row: dict, **extra) -> str:
    fields = {"model": row["model"], "context": row["context"], "message": row["message"], **extra}
    return json.dumps(fields, ensure_ascii=False)


def ask(rubric: str, text: str) -> list[str] | None:
    message = {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": int(time.time() * 1000)}
    payload = {
        "model": LABELLER,
        "provider": "anthropic",
        "thinking_level": "medium",
        "system_prompt": rubric,
        "messages": [message],
    }
    reply = common.trigger("router::complete", payload, timeout_s=300).get("message") or {}
    answer = extract.text(reply)
    ok = parse_label(answer)
    if ok is None:  # a provider failure comes back as a reply with stop_reason "error", not as a TriggerError
        cause = reply.get("error_message") or reply.get("stop_reason")
        print(f"no label in reply ({cause}): {answer[:200]!r}", file=sys.stderr)
    return ok


def label_row(row: dict, rubric: str) -> dict | None:
    """Two labellers; a third call settles a disagreement on the cheapest level. None when no `ok` came back."""
    text = labeller_input(row)
    a, b = ask(rubric, text), ask(rubric, text)
    if a and b and not needs_adjudication(a, b):
        return {"id": row["id"], "ok": sorted(set(a) | set(b), key=common.LEVELS.index), "agreed": True}
    ok = ask(rubric, labeller_input(row, answers=[a, b]))
    return {"id": row["id"], "ok": ok, "agreed": False} if ok else None


def main(argv: list[str]) -> None:
    args = [a for a in argv if a != "--run"]
    if len(args) != 1:
        sys.exit("usage: python3 -I label.py <out_dir> [--run]")
    out = Path(args[0])
    rows = common.read_jsonl(out / "prompts.jsonl")
    path = out / "labels.jsonl"
    labels = {r["id"]: r for r in common.read_jsonl(path)} if path.exists() else {}
    todo = [row for row in rows if row["id"] not in labels]
    rubric = RUBRIC.read_text(encoding="utf-8")
    if "--run" not in argv:
        n = len(todo)
        # ponytail: chars/4 estimate; router::count_tokens when it must be exact
        tokens = sum(len(rubric) + len(labeller_input(row)) for row in todo) // 4
        usd = common.INPUT_USD[LABELLER] / 1e6  # per input token; an output token costs 5 times as much
        low, high = ((k * tokens + 5 * k * n * OUTPUT_TOKENS) * usd for k in (2, 3))
        print(
            f"{n} prompts × 2–3 calls ({2 * n}–{3 * n} calls), about {2 * tokens:,}–{3 * tokens:,} input tokens and"
            f" {2 * n * OUTPUT_TOKENS:,}–{3 * n * OUTPUT_TOKENS:,} output tokens, about ${low:,.2f}–${high:,.2f}"
        )
        return
    misses = 0  # prompts in a row without a label: a run that keeps failing stops before it pays for the rest
    with open(path, "a", encoding="utf-8") as f:
        for row in todo:
            try:
                result, why = label_row(row, rubric), "no label"
            except common.TriggerError as err:
                result, why = None, err
            if result is None:
                print(f"{row['id']}: {why}", file=sys.stderr)
                misses += 1
                if misses == 5:
                    sys.exit(f"stopped: {misses} prompts in a row got no label")
                continue
            misses = 0
            labels[row["id"]] = result
            f.write(json.dumps(result) + "\n")
            f.flush()
    labelled = [labels[row["id"]] for row in rows if row["id"] in labels]
    agreed = sum(r["agreed"] for r in labelled)
    print(f"{len(labelled)} of {len(rows)} prompts labelled, {agreed} agreed ({agreed / max(len(labelled), 1):.0%})")


if __name__ == "__main__":
    main(sys.argv[1:])
