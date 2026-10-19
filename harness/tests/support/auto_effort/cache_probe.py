"""Cache retention across a level flip: python3 -I cache_probe.py <out_dir> --data <dir> [--with-openai] [--run]

Each (model, pair) is four turns on a new e2e session with no functions: the history at medium, a control at medium,
a flip to low and a flip back to medium. The history pastes judge.rs and send.rs, cut at the line where the payload
would reach Linux's cap on one exec argument, since iii trigger takes the payload as one. A turn is sent as soon as the
one before ends, so the cache stays warm. retention is the flip's first-step cache_read over what the control's first
step left in the cache, its cache_read plus cache_write, read from the session file under --data. Without --run it
prints the turn count and an estimated cost. A --run skips the pairs cache.jsonl holds as valid and rewrites it after
each pair; a pair whose turn fails, times out or shows no usage gets `invalid` = why, and 3 invalid pairs in a row
stop the run.
"""

import argparse
import json
import sys
import time
from collections.abc import Callable
from functools import partial
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import extract  # noqa: E402

REPO = Path(__file__).resolve().parents[4]
FILES = ("harness/src/judge.rs", "harness/src/functions/send.rs")
FOLLOW_UPS = (
    ("medium", "List the public functions in judge.rs above."),  # the control
    ("low", "List the public functions in send.rs above."),  # the flip
    ("medium", "Which of those functions touch thinking_level?"),  # the flip back
)
MODELS = {"claude-opus-5-5": "anthropic", "claude-sonnet-5-5": "anthropic", "claude-fable-5-1": "anthropic"}
OPENAI = {"gpt-6.1-sol": "openai"}  # the provider id the corpus's assistant messages record for it
PAIRS = 3
ARG_LIMIT = 131_072  # Linux refuses an exec argument this long (MAX_ARG_STRLEN); iii trigger takes the payload as one
OUTPUT_TOKENS = 2_000  # thinking and reply of one turn, for the estimate


def first_step_usage(lines: list[dict], user_seq: int) -> dict:
    """The final usage of the first assistant message after the user_seq-th turn start; {} until it is written."""
    turn = -1
    for message in extract.messages(lines):
        if extract.is_turn_start(message):
            turn += 1
        elif turn == user_seq and message.get("role") == "assistant":
            return message.get("usage") or {}
    return {}


def retention(cached: int, read: int) -> float:
    """The share of `cached`, what the control's first step left in the cache (its cache_read plus cache_write), that
    a later first step reads. The history turn gives the message cache no anchor (provider-anthropic anchors it on a
    tool_result or a stable assistant turn), so the control reads only the system prefix and writes the history."""
    return read / cached if cached else 0.0


def history() -> str:
    files = (f"### {path}\n{(REPO / path).read_text(encoding='utf-8')}" for path in FILES)
    return "\n\n".join(["Summarise each file below in three lines.", *files])


def fit(text: str, limit: int) -> str:
    """text, cut at a line end where it would pass `limit` chars as a JSON string."""
    kept, size = [], 0
    for line in text.splitlines(keepends=True):
        size += len(json.dumps(line)) - 2
        if size > limit:
            break
        kept.append(line)
    return "".join(kept)


def poll(check: Callable[[], dict | None], every: float, within: float) -> dict | None:
    """check()'s first truthy result, tried every `every` s for at most `within` s; None when none came."""
    deadline = time.monotonic() + within
    while not (result := check()):
        if time.monotonic() >= deadline:
            return None
        time.sleep(every)
    return result


def wait(session_id: str, turn_id: str, within: float = 300) -> str | None:
    """None once the turn completes; else why it did not. A status call that fails is retried; the third failure in
    a row raises its TriggerError."""
    failed = 0

    def ended() -> dict | None:
        nonlocal failed
        try:
            status = common.trigger("harness::status", {"session_id": session_id}, timeout_s=10)
        except common.TriggerError:
            failed += 1
            if failed == 3:
                raise
            return None
        failed = 0
        done = status.get("turn_id") == turn_id and status.get("status") in ("completed", "cancelled", "failed")
        return status if done else None

    status = poll(ended, 2, within)
    if status is None:
        return f"not ended after {within:g} s"
    if status["status"] == "completed":
        return None
    return status["status"] + (f" ({status['result_error']})" if status.get("result_error") else "")


def read_usage(path: Path, user_seq: int) -> dict:
    try:
        return first_step_usage(common.read_jsonl(path), user_seq)
    except (OSError, ValueError):  # not written yet, or a line caught mid-append
        return {}


def probe(model: str, provider: str, turns: list[tuple[str, str]], sessions: Path) -> tuple[list, str | None]:
    """One sequence on a new session: the first-step usage of turns 2-4 (None for a turn it did not reach), and why
    the pair is invalid, if it is."""
    usages, session_id = [None, None, None], None
    for seq, (level, message) in enumerate(turns):
        payload = {
            "message": message,
            "model": model,
            "provider": provider,
            "session": {"kind": "e2e"},
            "options": {"thinking_level": level},
        }
        if session_id:
            payload["session_id"] = session_id
        try:
            sent = common.trigger("harness::send", payload, timeout_s=30)
            session_id = sent["session_id"]
            why = wait(session_id, sent["turn_id"])
        except common.TriggerError as err:
            why = str(err)
        if seq and not why:
            usage = poll(partial(read_usage, sessions / f"{session_id}.jsonl", seq), 0.5, 10)
            if usage:
                usages[seq - 1] = usage
            else:
                why = "no usage after 10 s"
        if why:
            return usages, f"turn {seq + 1}: {why}"
    return usages, None


def cache(usage: dict | None, key: str) -> int | None:
    """A cache figure of a measured first step; None for a turn the pair did not reach."""
    return None if usage is None else usage.get(key) or 0


def main(argv: list[str]) -> None:
    parser = argparse.ArgumentParser(prog="cache_probe.py")
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("--data", type=Path, required=True, help="the stack's data dir, which holds session-manager/")
    parser.add_argument("--with-openai", action="store_true", help="also probe " + ", ".join(OPENAI))
    parser.add_argument("--run", action="store_true", help="drive the stack and write cache.jsonl")
    args = parser.parse_args(argv)
    sessions = args.data / "session-manager"
    if not sessions.is_dir():
        parser.error(f"{sessions} is not a directory")
    models = MODELS | (OPENAI if args.with_openai else {})
    full = history()
    turns = [("medium", fit(full, ARG_LIMIT - 1024)), *FOLLOW_UPS]  # 1 KiB left for the rest of the payload
    path = args.out_dir / "cache.jsonl"
    rows = common.read_jsonl(path) if path.exists() else []
    done = {(row["model"], row["pair"]) for row in rows if not row["invalid"]}
    todo = [(model, pair) for model in models for pair in range(PAIRS) if (model, pair) not in done]
    if not args.run:
        chars = len(turns[0][1])
        tokens = chars // 4  # ponytail: chars/4, as in label.py; router::count_tokens when it must be exact
        usages = ({"cache_write": tokens}, {"cache_read": tokens}, {"cache_write": tokens}, {"cache_write": tokens})
        per_pair = sum(common.units(dict(usage, output=OUTPUT_TOKENS)) for usage in usages)
        print(
            f"{len(turns)} turns × {PAIRS} pairs × {len(models)} models = {len(turns) * PAIRS * len(models)} turns,"
            f" {len(todo)} pairs still to run; the history is about {tokens:,} tokens ({chars:,} of {len(full):,}"
            " chars fit one iii trigger argument)"
        )
        for provider in dict.fromkeys(models.values()):  # Anthropic and OpenAI apart, never summed
            pairs = sum(models[model] == provider for model, _ in todo)
            print(
                f"{provider}: {pairs} pairs, about {pairs * per_pair:,.0f} units if every flip rewrites the cache"
                f" and a turn outputs {OUTPUT_TOKENS:,} tokens"
            )
        return
    invalid_in_a_row = 0  # a broken setup (a wrong --data, a model the stack lacks) must not pay for every pair
    for model, pair in todo:
        (control, flip, back), invalid = probe(model, models[model], turns, sessions)
        row = {
            "model": model,
            "pair": pair,
            "control_read": cache(control, "cache_read"),
            "control_write": cache(control, "cache_write"),
            "flip_read": cache(flip, "cache_read"),
            "back_read": cache(back, "cache_read"),
            "retention": None,
            "retention_back": None,
            "invalid": invalid,
        }
        if not invalid:  # every turn measured
            cached = row["control_read"] + row["control_write"]
            row["retention"] = retention(cached, row["flip_read"])
            row["retention_back"] = retention(cached, row["back_read"])
        rows = [old for old in rows if (old["model"], old["pair"]) != (model, pair)] + [row]  # a retry replaces it
        common.write_jsonl(path, rows)  # after each pair, so a stopped run keeps the pairs it measured
        if not invalid:
            invalid_in_a_row = 0
            print(f"{model} pair {pair}: retention {row['retention']:.2f}, back {row['retention_back']:.2f}")
            continue
        print(f"{model} pair {pair}: invalid, {invalid}")
        invalid_in_a_row += 1
        if invalid_in_a_row == 3:
            sys.exit(f"stopped: 3 pairs in a row invalid ({invalid}); {path} keeps the pairs measured so far")
    print(f"wrote {path}")


if __name__ == "__main__":
    main(sys.argv[1:])
