"""Drive context-clm bench episodes through the isolated iii stack (boot.sh), grade them, estimate their cost.

  python run.py grid regression|long-pilot|long > episodes.json
  python run.py estimate episodes.json [--prior RUN ...]
  python run.py run RUN episodes.json --yes [--workers 6] [--prior RUN ...]

Episode: {arm: clm|sum|base|default, task, level, seed, model, budget?, effort?, max_cost_usd?, tag?}.
Tasks: kv|log|needle (ContextBench-style; level = size), fields (level 24 = ide functions, 40 = ide + harness),
http-brace|ide-update-op (long coding tasks). A strategy arm opts its session in through session metadata
{context_strategy, context_budget_tokens}; `default` sets neither. Results: RUN/results/<sid>.json.
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from ctxbench import CLAUDE_PER_O200K, END, GEN, LIMIT, answer_re, gen
from skills import skill

HERE = Path(__file__).resolve().parent
REPO = Path(os.environ.get("BENCH_REPO") or HERE.parents[1])
ENGINE = os.environ.get("BENCH_ENGINE", "ws://127.0.0.1:49250")
III = [os.environ.get("III_BIN", str(Path.home() / ".local/bin/iii")), "trigger", "--engine", ENGINE]
S55, O55 = "claude-sonnet-5-5", "claude-opus-5-5"
MTAG = {O55: "o55", S55: "s55"}
SYSTEM = "You are an assistant working in a Linux shell through tools."
DROP = "anthropic dropped replayed thinking blocks"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
TRUTH = [json.loads(line) for line in (HERE / "truth-fields.jsonl").read_text().splitlines()]
FIELDS_SNAPSHOT = "d3f9bcb8a"
FIELDS_TASK = (
    "Your working directory contains {dirs}, the Rust source of {what}. For each of the {n} bus functions listed "
    "below, find the request type its handler deserializes and list the TOP-LEVEL JSON field names a caller can send: "
    "apply serde renames and rename_all, skip fields marked #[serde(skip)] or #[schemars(skip)], and for "
    "#[serde(flatten)] include the flattened struct's fields. Work only from the source code.\n\nWhen you are done, "
    'reply with one JSON object {{"<function id>": ["field", ...], ...}} (each array sorted) as your final message; '
    "that message is what is graded.\n\nFunctions: {fns}"
)
SHELL_HINT = (
    "shell::exec runs no shell and starts in your working directory: pass cwd for a subdirectory, and for pipes, "
    'redirection or heredocs run {"command":"bash","args":["-c","<command>"]}.'
)
# Mean cost per episode in the spike's run-03 report, keyed arm|task|level|model.
PRIORS = {
    f"{arm}|{task}|{level}|{model}": cost
    for model, rows in {
        S55: [
            ("kv", 1000, 0.33, 0.55, 2.28),
            ("kv", 4000, 0.33, 1.46, 6.02),
            ("log", 1600, 0.37, 0.98, 2.57),
            ("log", 3200, 0.38, 1.65, 4.83),
            ("needle", 24, 0.28, 0.88, 1.40),
            ("needle", 60, 0.28, 2.43, 4.51),
            ("fields", 24, 0.05, 0.23, 0.21),
        ],
        O55: [
            ("kv", 4000, 0.61, 2.49, 20.91),
            ("log", 3200, 0.57, 2.03, 9.44),
            ("needle", 60, 0.59, 5.55, 9.12),
            ("fields", 24, 0.10, 0.30, 0.31),
        ],
    }.items()
    for task, level, *costs in rows
    for arm, cost in zip(("base", "clm", "sum"), costs, strict=True)
}


def kind(task):
    return "ctx" if task in GEN else "fields" if task == "fields" else "code"


def cap_of(ep):
    return ep.get("max_cost_usd") or {"ctx": 15, "fields": 5, "code": 10}[kind(ep["task"])]


def ep_key(ep):
    return f"{ep['arm']}|{ep['task']}|{ep.get('level', 0)}|{ep['model']}"


def sid_of(ep):
    return f"{ep['arm']}-{ep['task']}{ep.get('level', 0)}-s{ep['seed']}-{MTAG[ep['model']]}{ep.get('tag', '')}"


def grid(name):
    eps = []
    if name == "regression":
        for seed in (0, 1):
            for task, level in (
                ("kv", 1000),
                ("kv", 4000),
                ("log", 1600),
                ("log", 3200),
                ("needle", 24),
                ("needle", 60),
            ):
                arms = ["clm", "sum", "base"] + (["default"] if level in (1000, 1600) else [])
                eps += [{"arm": a, "task": task, "level": level, "seed": seed, "model": S55} for a in arms]
        for task, level in (("kv", 4000), ("log", 3200), ("needle", 60)):
            eps += [
                {"arm": a, "task": task, "level": level, "seed": 0, "model": O55, "max_cost_usd": 30}
                for a in ("clm", "sum", "base")
            ]
        for model in (S55, O55):
            for seed in (0, 1):
                eps += [
                    {"arm": a, "task": "fields", "level": 24, "seed": seed, "model": model, "budget": 16384}
                    for a in ("clm", "sum", "base", "default")
                ]
    elif name in ("long", "long-pilot"):
        pilot = name == "long-pilot"
        for model in (S55,) if pilot else (S55, O55):
            for seed in (0,) if pilot else (0, 1, 2):
                for task, level in (("http-brace", 0), ("ide-update-op", 0), ("fields", 40)):
                    eps.append(
                        {"arm": "clm", "task": task, "level": level, "seed": seed, "model": model, "budget": 65536}
                    )
                    eps.append({"arm": "default", "task": task, "level": level, "seed": seed, "model": model})
    else:
        raise SystemExit(f"unknown grid {name!r}: regression | long-pilot | long")
    return eps


def load_priors(dirs):
    priors, seen = dict(PRIORS), {}
    for d in dirs:
        for p in Path(d, "results").glob("*.json"):
            r = json.loads(p.read_text())
            if "cost_usd" in r:
                seen.setdefault(ep_key(r), []).append(r["cost_usd"])
    priors.update({k: statistics.mean(v) for k, v in seen.items()})
    return priors


def estimate(eps, priors):
    """Print expected cost (prior mean, or the cap when no prior) and the cap total; return (expected, cap)."""
    groups = {}
    for ep in eps:
        g = groups.setdefault(ep_key(ep), {"n": 0, "cap": 0.0, "prior": priors.get(ep_key(ep))})
        g["n"] += 1
        g["cap"] += cap_of(ep)
    expected = sum(g["n"] * g["prior"] if g["prior"] is not None else g["cap"] for g in groups.values())
    cap = sum(g["cap"] for g in groups.values())
    print("| group | n | prior $/ep | cap $ |\n|---|---|---|---|")
    for k, g in sorted(groups.items()):
        prior = "none (cap used)" if g["prior"] is None else f"{g['prior']:.2f}"
        print(f"| {k} | {g['n']} | {prior} | {g['cap']:.0f} |")
    print(f"\nepisodes: {len(eps)}; expected ~${expected:.2f}; harness caps total ${cap:.0f}")
    print("The sum arm's summariser calls are outside the harness cap (its priors include them).")
    return expected, cap


def call(fn, payload, timeout=60):
    out = subprocess.run([*III, fn, "--json", json.dumps(payload)], capture_output=True, text=True, timeout=timeout)
    if out.returncode:
        raise RuntimeError(f"{fn}: {out.stderr.strip()[:500]}")
    return json.loads(out.stdout or "null")


def send(sid, message, options, first=None):
    return call("harness::send", {"session_id": sid, "message": message, "options": options, **(first or {})})


def wait(sid, turn_id, deadline_s=1800):
    end = time.time() + deadline_s
    while time.time() < end:
        st = call("harness::status", {"session_id": sid, "verbose": True})
        done = st.get("status") in ("completed", "failed", "cancelled") and not st.get("expects_wake")
        if st.get("turn_id") == turn_id and done:
            return st
        time.sleep(2)
    raise TimeoutError(f"{sid} turn {turn_id} did not finish")


def messages(sid):
    out, cursor = [], None
    while True:
        r = call("session::messages", {"session_id": sid, "limit": 500, **({"cursor": cursor} if cursor else {})})
        out += r.get("messages", [])
        cursor = r.get("next_cursor")
        if not cursor:
            return out


def strings(x):
    if isinstance(x, str):
        try:
            j = json.loads(x)
        except ValueError:
            j = None
        if isinstance(j, (dict, list)):
            yield from strings(j)
        else:
            yield x
    elif isinstance(x, dict):
        for v in x.values():
            yield from strings(v)
    elif isinstance(x, list):
        for v in x:
            yield from strings(v)


def turn_text(entries):
    """Everything the agent added during a turn: assistant text, call arguments, result text."""
    parts = []
    for e in entries:
        m = e.get("message") or {}
        if m.get("role") not in ("assistant", "function_result"):
            continue
        for b in m.get("content") or []:
            if b.get("type") == "text":
                parts += list(strings(b.get("text", "")))
            elif b.get("type") == "function_call":
                parts += list(strings(b.get("arguments")))
    return "\n".join(parts)


def last_json(text):
    """The last top-level JSON object in a reply (a model may print a draft, then the corrected answer)."""
    dec, found, i = json.JSONDecoder(), {}, 0
    while (i := text.find("{", i)) >= 0:
        try:
            obj, end = dec.raw_decode(text, i)
            if isinstance(obj, dict):
                found = obj
            i = end
        except ValueError:
            i += 1
    return {k: set(v) for k, v in found.items() if isinstance(v, list)}


def score_fields(final, truth):
    ans = last_json(final)
    tp = sum(len(set(t["fields"]) & ans.get(t["id"], set())) for t in truth)
    fp = sum(len(ans.get(t["id"], set()) - set(t["fields"])) for t in truth)
    fn = sum(len(set(t["fields"]) - ans.get(t["id"], set())) for t in truth)
    return {
        "score": round(sum(ans.get(t["id"]) == set(t["fields"]) for t in truth) / len(truth), 4),
        "precision": round(tp / max(tp + fp, 1), 3),
        "recall": round(tp / max(tp + fn, 1), 3),
    }


def answer_score(by_turn, op_turn, first_query_op, pairs, attr):
    ok = 0
    for q, (key, val) in enumerate(pairs):
        text = turn_text(by_turn.get(op_turn(first_query_op + q), []))
        got = re.findall(answer_re(attr, key), text, flags=re.S)
        ok += any(x.strip() == val for x in got)
    return ok / len(pairs)


def needle_score(window, needles):
    lines = {line.rstrip() for line in window.splitlines()}
    return sum(n in lines for n in needles) / len(needles)


def drop_events(run, sid):
    log = run / "logs" / "provider-anthropic.log"
    if not log.exists():
        return None
    mark = f"session_id={sid} "
    with log.open(errors="replace") as f:
        lines = (ANSI.sub("", line) for line in f)  # a coloured log splits "session_id=" with escapes
        return sum(1 for line in lines if DROP in line and mark in line)


def strategy_status(arm, sid, budget):
    """context-<arm>::status in the shape report.py reads; raises on anything else instead of grading blanks."""
    st = call(f"context-{arm}::status", {"session_id": sid})
    stats = st.get("stats")
    if not (st.get("found") and st.get("budget_tokens") == budget and isinstance(stats, dict) and "peak_tc" in stats):
        raise RuntimeError(f"{sid}: context-{arm}::status is not {{found, budget_tokens: {budget}, stats}}: {st}")
    return st


def git(*args):
    return subprocess.run(["git", "-C", str(REPO), *args], check=True, capture_output=True).stdout


def untar(blob, root, exclude=()):
    subprocess.run(["tar", "-x", "-C", str(root), *[f"--exclude={x}" for x in exclude]], input=blob, check=True)


def episode(run, ep):
    arm, task, model, level = ep["arm"], ep["task"], ep["model"], ep.get("level", 0)
    sid = sid_of(ep)
    res_path = run / "results" / f"{sid}.json"
    if res_path.exists():
        return json.loads(res_path.read_text())
    root = run / "workspace" / sid
    if root.exists():
        raise RuntimeError(f"{sid}: stale attempt (workspace exists, no result); use a new tag")
    root.mkdir(parents=True)
    k = kind(task)
    budget = ep.get("budget") or (round(LIMIT * CLAUDE_PER_O200K[task]) if k == "ctx" else 65536)
    meta = {} if arm == "default" else {"context_strategy": arm, "context_budget_tokens": budget}
    mirror = f".clm/{sid}.ctx"
    first = {"model": model, "provider": "anthropic", "session": {"title": sid, "kind": "automation", "metadata": meta}}
    per = {"max_turns": {"ctx": 40, "fields": 80, "code": 200}[k], "max_output_tokens": 16000}
    per["metadata"] = {"fs_scope": {"root": str(root)}}
    allow = ["shell::exec"] + {
        "ctx": [],
        "fields": ["coder::read-file"],
        "code": ["coder::read-file", "coder::search", "coder::update-file", "coder::create-file"],
    }[k]
    opts = {
        **per,
        "system_prompt": SYSTEM,
        "system_prompt_strategy": "override",
        "thinking_level": ep.get("effort", "medium"),
        "max_cost_usd": cap_of(ep),
        "functions": {"allow": allow, "expose": "native"},
    }
    t0 = time.time()
    if k == "ctx":
        g = gen(task, level, ep["seed"])
        msgs = [g["instruction"] + "\n\n" + skill(arm, task, mirror, budget), *g["ops"], END]
    elif k == "fields":
        truth = [t for t in TRUTH if level == 40 or t["scope"] == "ide"]
        dirs = ["ide/src", "ide/Cargo.toml"] + (["harness/src", "harness/Cargo.toml"] if level == 40 else [])
        untar(git("archive", FIELDS_SNAPSHOT, *dirs), root)
        what = "two iii workers (ide and harness)" if level == 40 else "an iii worker (ide)"
        top = "`ide/` and `harness/`" if level == 40 else "`ide/`"
        fns = " ".join(sorted(t["id"] for t in truth))
        msgs = [FIELDS_TASK.format(dirs=top, what=what, n=len(truth), fns=fns) + "\n\n" + SHELL_HINT]
    else:
        raise ValueError(f"{sid}: unknown task {task!r}")

    turns, stops, status, err = [], [], "completed", None
    for i, m in enumerate(msgs):
        r = send(sid, m, opts if i == 0 else per, first if i == 0 else None)
        st = wait(sid, r["turn_id"], deadline_s=1800 if k == "ctx" else 5400)
        if i == 0:
            cs = st.get("context_strategy") or {}
            if arm != "default" and not cs.get("effective"):
                raise RuntimeError(f"{sid}: strategy not effective: {cs}")
            if arm == "clm" and not (root / mirror).exists():
                raise RuntimeError(f"{sid}: no mirror at {mirror} (does safe_id change this session id?)")
            if arm != "default":
                strategy_status(arm, sid, budget)
        turns.append(r["turn_id"])
        stops.append(st.get("stop_reason"))
        if st.get("status") != "completed":
            status, err = st.get("status"), st.get("result_error")
            break

    entries = messages(sid)
    out = {"sid": sid, **{x: ep.get(x) for x in ("arm", "task", "level", "seed", "model")}, "tag": ep.get("tag", "")}
    out.update({"status": status, "error": err, "budget": budget if arm != "default" else None})
    if k == "ctx":
        by_turn = {}
        for e in entries:
            by_turn.setdefault((e.get("origin") or {}).get("turn_id"), []).append(e)

        def op_turn(op):
            return turns[op + 1] if op + 1 < len(turns) else None

        t = g["truth"]
        if t["kind"] == "kv":
            score = answer_score(by_turn, op_turn, t["first_query_op"], t["gets"], "key")
        elif t["kind"] == "log":
            score = answer_score(by_turn, op_turn, t["first_query_op"], t["answers"], "qid")
        else:
            window = root / mirror if arm == "clm" else run / "arms" / "window" / f"{sid}.txt"
            score = needle_score(window.read_text(errors="replace"), t["needles"])
        out.update({"score": round(score, 4), "ops_total": len(g["ops"]), "end_sent": len(turns) == len(msgs)})
        out["ops_delivered"] = min(max(0, len(turns) - 1), len(g["ops"]))
        out["max_turns_hits"] = sum(1 for x in stops if x == "max_turns")
    elif k == "fields":
        final = ""
        for e in entries:
            m = e.get("message") or {}
            if m.get("role") == "assistant":
                text = "".join(b.get("text", "") for b in m.get("content") or [] if b.get("type") == "text").strip()
                final = text or final
        out.update(score_fields(final, truth))

    usage = [(e.get("message") or {}).get("usage") or {} for e in entries]
    usage = [u for u in usage if u]
    strategy = {} if arm == "default" else strategy_status(arm, sid, budget)
    summary_cost = (strategy.get("stats") or {}).get("summary_cost_usd") or 0
    out.update(
        {
            "cost_usd": round(sum(u.get("cost_usd") or 0 for u in usage) + summary_cost, 4),
            "llm_calls": len(usage),
            "input": sum(u.get("input") or 0 for u in usage),
            "cache_read": sum(u.get("cache_read") or 0 for u in usage),
            "cache_write": sum(u.get("cache_write") or 0 for u in usage),
            "output": sum(u.get("output") or 0 for u in usage),
            "peak_prompt": max(
                ((u.get("input") or 0) + (u.get("cache_read") or 0) + (u.get("cache_write") or 0) for u in usage),
                default=0,
            ),
            "strategy": strategy,
            "drops": drop_events(run, sid),
            "wall_s": round(time.time() - t0),
        }
    )
    res_path.write_text(json.dumps(out, indent=1))
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("grid").add_argument("name")
    e = sub.add_parser("estimate")
    e.add_argument("episodes")
    e.add_argument("--prior", nargs="*", default=[])
    r = sub.add_parser("run")
    r.add_argument("run")
    r.add_argument("episodes")
    r.add_argument("--workers", type=int, default=6)
    r.add_argument("--prior", nargs="*", default=[])
    r.add_argument("--yes", action="store_true", help="spend money: run after printing the estimate")
    a = ap.parse_args(argv)
    if a.cmd == "grid":
        print(json.dumps(grid(a.name), indent=1))
        return 0
    eps = json.loads(Path(a.episodes).read_text())
    if a.cmd == "run":  # episodes with a result are skipped, so they are not priced either
        done = {p.stem for p in Path(a.run, "results").glob("*.json")}
        eps = [ep for ep in eps if sid_of(ep) not in done]
    _, cap = estimate(eps, load_priors(a.prior))
    if a.cmd == "estimate":
        return 0
    if not a.yes:
        print(f"Not started. Re-run with --yes to run {len(eps)} episodes (harness caps total ${cap:.0f}).")
        return 2
    run = Path(a.run).resolve()
    (run / "results").mkdir(parents=True, exist_ok=True)

    def safe(ep):
        try:
            return episode(run, ep)
        except Exception as ex:  # one broken episode must not stop the batch
            return {"episode": ep, "crash": repr(ex)[:500]}

    with ThreadPoolExecutor(a.workers) as pool, (run / "episodes.out").open("a") as log:
        for res in pool.map(safe, eps):
            line = json.dumps(res)
            print(line, flush=True)
            log.write(line + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
