"""Judge prompt rows with each arm: python3 -I judge.py <out_dir> --arm <name>... [--rows <file>] [--run]

Arms: typesafe (judge::evaluate), haiku (router::complete) and the baselines always-low, always-medium, always-high and
keep. Without --run it prints each arm's call count and estimated input tokens. Rows default to prompts.jsonl in
out_dir plus the effortless cases. A --run upserts verdicts.jsonl by (id, arm), and writes it even when stopped. It
does not ask a judge again for a row verdicts.jsonl answers, judge-side errors included, which count as kept; a
transport error, a call that never reached the judge, is asked again, and 5 of them in a row stop the run.
effort_question.json and haiku_system.md adapt effortless's JEV_TASK, JEV_EFFORTS and JUDGE_SYSTEM (MIT, see
cases/LICENSE-effortless).
"""

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import extract  # noqa: E402
import label  # noqa: E402

HERE = Path(__file__).resolve().parent
QUESTION = HERE / "effort_question.json"
HAIKU_SYSTEM = HERE / "haiku_system.md"
ARMS = ("typesafe", "haiku", "always-low", "always-medium", "always-high", "keep")


def keeps_effort(message: str, context: str) -> bool:
    """A short follow-up ("go", "ok") keeps the effort, unless it answers a question at the end of the last reply."""
    text = message.strip()
    follow_up = 0 < len(text) <= 12 and len(text.split()) <= 2
    reply = context.split("\nassistant: ")[-1]
    return follow_up and not ("assistant: " in context and "?" in reply[-400:])


def calls(arm: str, row: dict) -> bool:
    """Whether the arm asks the stack about this row: only the judges do, and never for a follow-up that keeps."""
    return arm in ("typesafe", "haiku") and not keeps_effort(row["message"], row["context"])


def evaluation(row: dict, question: dict) -> dict:
    """The judge::evaluate item. The state is a JSON string: the bus sorts object keys, and Jev reads them in order."""
    state = {
        "task": question["task"],
        "current_model": row.get("model"),
        "current_effort": row.get("level"),
        "recent_conversation": row["context"],
        "next_message": row["message"][:4000],
    }
    effort = {"type": "choice", "instructions": question["instructions"], "criteria": question["criteria"]}
    return {"id": row["id"], "state": json.dumps(state, ensure_ascii=False), "questions": {"effort": effort}}


def verdict(answer: dict | None, error: str | None, current: str | None) -> dict:
    """An error, no answer, a choice outside LEVELS or confidence below 0.5 keeps the current level, medium when
    unknown."""
    confidence = (answer or {}).get("confidence")
    if answer and not error and answer.get("choice") not in common.LEVELS:
        error = "invalid_choice"
    if error or not answer or (confidence is not None and confidence < 0.5):
        return {"level": current or "medium", "confidence": confidence, "kept": True, "error": error}
    return {"level": answer["choice"], "confidence": confidence, "kept": False, "error": None}


def ask_typesafe(row: dict, question: dict) -> tuple[dict | None, str | None, int]:
    payload = {"timeout_ms": 3000, "provider": "typesafe", "evaluations": [evaluation(row, question)]}
    try:
        reply = common.trigger("judge::evaluate", payload, timeout_s=4)  # the harness waits timeout_ms + 1 s
    except common.TriggerError as err:
        return None, err.code, 0
    if reply.get("status") != "ok":
        return None, reply.get("code", "error"), 0
    result = reply["results"][row["id"]]
    usage = result.get("usage") or {}
    return result["answers"]["effort"], None, (usage.get("input_tokens") or 0) + (usage.get("output_tokens") or 0)


def parse_haiku(text: str) -> dict | None:
    """A {"effort", "sure"} reply as a judge answer; None without a known level."""
    try:
        raw = json.JSONDecoder().raw_decode(text, text.find("{"))[0]
    except ValueError:
        return None
    if not isinstance(raw, dict) or raw.get("effort") not in common.LEVELS:
        return None
    sure = raw.get("sure")
    return {"choice": raw["effort"], "confidence": sure if type(sure) in (int, float) and 0 <= sure <= 1 else None}


def ask_haiku(row: dict, question: dict, system: str) -> tuple[dict | None, str | None, int]:
    state = evaluation(row, question)["state"]
    message = {"role": "user", "content": [{"type": "text", "text": state}], "timestamp": int(time.time() * 1000)}
    payload = {
        "model": "claude-haiku-5-5",
        "provider": "anthropic",
        "system_prompt": system,
        "max_output_tokens": 160,
        "messages": [message],
    }
    try:
        reply = common.trigger("router::complete", payload, timeout_s=6)  # effortless gives Haiku 6 s
    except common.TriggerError as err:
        return None, err.code, 0
    usage = reply.get("usage") or {}
    tokens = sum(usage.get(key) or 0 for key in ("input", "cache_read", "cache_write", "output"))
    message = reply.get("message") or {}
    answer = parse_haiku(extract.text(message))
    return answer, None if answer else (message.get("error_kind") or "invalid_response"), tokens


def judged(arm: str, row: dict, question: dict, system: str) -> dict:
    """One verdicts.jsonl row."""
    answer, error, tokens, ms = None, None, 0, 0
    if arm.startswith("always-"):
        answer = {"choice": arm.removeprefix("always-")}
    elif calls(arm, row):
        start = time.monotonic()
        if arm == "typesafe":
            answer, error, tokens = ask_typesafe(row, question)
        else:
            answer, error, tokens = ask_haiku(row, question, system)
        ms = round((time.monotonic() - start) * 1000)
    return {"id": row["id"], "arm": arm, **verdict(answer, error, row.get("level")), "ms": ms, "tokens": tokens}


def main(argv: list[str]) -> None:
    parser = argparse.ArgumentParser(prog="judge.py")
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("--arm", nargs="+", choices=ARMS, required=True)
    parser.add_argument("--rows", type=Path, help="default: prompts.jsonl in out_dir plus the effortless cases")
    parser.add_argument("--run", action="store_true", help="call the stack and write verdicts.jsonl")
    args = parser.parse_args(argv)
    if args.rows:
        rows = common.read_jsonl(args.rows)
    else:
        rows = common.read_jsonl(args.out_dir / "prompts.jsonl") + label.effortless_rows()
    question = json.loads(QUESTION.read_text(encoding="utf-8"))
    system = HAIKU_SYSTEM.read_text(encoding="utf-8")
    path = args.out_dir / "verdicts.jsonl"
    verdicts = {(v["id"], v["arm"]): v for v in common.read_jsonl(path)} if path.exists() else {}  # R5: by (id, arm)

    def answered(arm: str, row: dict) -> bool:
        """A paid verdict to keep: an answer, or a judge-side error, which counts as kept (Review Focus 2). Only a
        transport error, a stack that never answered, is asked again."""
        old = verdicts.get((row["id"], arm))
        return calls(arm, row) and old is not None and old.get("error") != "transport"

    if not args.run:
        for arm in args.arm:
            asked = [evaluation(row, question) for row in rows if calls(arm, row) and not answered(arm, row)]
            # ponytail: chars/4 estimate, as in label.py; router::count_tokens when it must be exact
            sizes = [
                len(system) + len(e["state"]) if arm == "haiku" else len(json.dumps(e, ensure_ascii=False))
                for e in asked
            ]
            print(f"{arm}: {len(rows)} rows, {len(asked)} calls, about {sum(sizes) // 4:,} input tokens")
        return
    for arm in args.arm:
        out, failed = [], 0  # calls in a row that never reached the judge: a down stack stops the run
        try:
            for row in rows:
                if answered(arm, row):
                    continue
                v = verdicts[row["id"], arm] = judged(arm, row, question, system)
                out.append(v)
                if v["error"] == "transport":
                    failed += 1
                    if failed == 5:
                        sys.exit(f"stopped: {arm} could not reach the stack 5 times in a row")
                elif calls(arm, row):
                    failed = 0
        finally:  # an upsert (R5) of every verdict so far, through a stop, a Ctrl-C or a crash
            common.write_jsonl(path, list(verdicts.values()))
        kept, errors = sum(v["kept"] for v in out), sum(v["error"] is not None for v in out)
        print(f"{arm}: {len(out)} verdicts, {kept} kept, {errors} errors")


if __name__ == "__main__":
    main(sys.argv[1:])
