"""Aggregate RUN/results/*.json into markdown tables and the spec §7 pass checks.

  python report.py RUN [RUN ...] [--check]

--check exits 1 when a pass check fails or nothing was graded.
"""

import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

S55, O55 = "claude-sonnet-5-5", "claude-opus-5-5"
NAME = {S55: "Sonnet 5.5", O55: "Opus 5.5"}
# CLM score per group in the spike's run-03 (spec §7.1: the product must match or beat it).
SPIKE_CLM = {
    ("kv", 1000, S55): 1.0,
    ("kv", 4000, S55): 1.0,
    ("log", 1600, S55): 1.0,
    ("log", 3200, S55): 1.0,
    ("needle", 24, S55): 1.0,
    ("needle", 60, S55): 0.95,
    ("kv", 4000, O55): 1.0,
    ("log", 3200, O55): 0.96,
    ("needle", 60, O55): 1.0,
    ("fields", 24, S55): 0.92,
    ("fields", 24, O55): 1.0,
}
CODE = ("http-brace", "ide-update-op")


def mean(xs):
    xs = [x for x in xs if x is not None]
    return statistics.mean(xs) if xs else None


def fmt(x, d=2):
    return "–" if x is None else f"{x:.{d}f}"


def load(runs):
    return [json.loads(p.read_text()) for r in runs for p in sorted(Path(r, "results").glob("*.json"))]


def table(title, keyf, rows, cols):
    groups = defaultdict(list)
    for r in rows:
        groups[keyf(r)].append(r)
    out = [f"\n### {title}\n", "| " + " | ".join(["group", "n"] + [c[0] for c in cols]) + " |"]
    out.append("|" + "---|" * (2 + len(cols)))
    for k in sorted(groups):
        xs = groups[k]
        out.append("| " + " | ".join([k, str(len(xs))] + [c[1](xs) for c in cols]) + " |")
    return out


def st(x, k):
    """One counter of the strategy's status (`context-<arm>::status`), which keeps its counters under `stats`."""
    return ((x.get("strategy") or {}).get("stats") or {}).get(k)


COMMON = [
    ("score", lambda xs: fmt(mean([x["score"] for x in xs]))),
    ("cost $", lambda xs: fmt(mean([x["cost_usd"] for x in xs]))),
    (
        "$/success",
        lambda xs: fmt(sum(x["cost_usd"] for x in xs) / s) if (s := sum(x["score"] for x in xs)) else "∞",
    ),
    ("peak prompt", lambda xs: fmt(mean([x["peak_prompt"] for x in xs]), 0)),
    ("peak tc", lambda xs: fmt(mean([st(x, "peak_tc") for x in xs]), 0)),
    ("ended at wall", lambda xs: f"{sum(1 for x in xs if x['status'] != 'completed')}/{len(xs)}"),
    ("edits", lambda xs: fmt(mean([st(x, "edits_applied") for x in xs]), 1)),
    ("rollbacks", lambda xs: fmt(mean([st(x, "rollbacks") for x in xs]), 1)),
    ("summaries", lambda xs: fmt(mean([st(x, "summaries") for x in xs]), 1)),
    ("summary fails", lambda xs: fmt(mean([st(x, "summary_failures") for x in xs]), 1)),
    ("drops", lambda xs: str(sum(x.get("drops") or 0 for x in xs))),
]


def tables(rows):
    out = []
    ctx = [r for r in rows if r["task"] in ("kv", "log", "needle")]
    ops = ("ops done", lambda xs: fmt(mean([x["ops_delivered"] / x["ops_total"] for x in xs])))
    for model in NAME:
        sel = [r for r in ctx if r["model"] == model]
        out += table(
            f"ContextBench-style — {NAME[model]}", lambda r: f"{r['task']} {r['level']} {r['arm']}", sel, [*COMMON, ops]
        )
    fields = [r for r in rows if r["task"] == "fields"]
    recall = ("recall", lambda xs: fmt(mean([x.get("recall") for x in xs])))
    out += table(
        "Code reading (exact answer)",
        lambda r: f"fields{r['level']} {NAME[r['model']]} {r['arm']}",
        fields,
        [*COMMON, recall],
    )
    code = [r for r in rows if r["task"] in CODE]
    calls = ("llm calls", lambda xs: fmt(mean([x["llm_calls"] for x in xs]), 0))
    out += table(
        "Long coding tasks (cargo test)", lambda r: f"{r['task']} {NAME[r['model']]} {r['arm']}", code, [*COMMON, calls]
    )
    return out


def checks(rows):
    """[(name, ok, detail)] for spec §7.1 (regression) and §7.2 (long tasks), over whatever rows are present."""
    out = []
    clm = [r for r in rows if r["arm"] == "clm"]
    for (task, level, model), spike in SPIKE_CLM.items():
        xs = [r["score"] for r in clm if (r["task"], r["level"], r["model"]) == (task, level, model)]
        if xs:
            got = statistics.mean(xs)
            out.append(
                (f"regression {task} {level} {NAME[model]}", got >= spike, f"clm {got:.2f} vs spike {spike:.2f}")
            )
    drops = sum(r.get("drops") or 0 for r in clm)
    if clm:
        out.append(("zero thinking-drop events on clm", drops == 0, f"{drops} events"))
    long = [r for r in rows if r["task"] in CODE or (r["task"] == "fields" and r["level"] == 40)]
    for model in NAME:
        by = defaultdict(list)
        for r in long:
            if r["model"] == model:
                by[(r["arm"], r["task"])].append(r)
        if not any(a == "clm" for a, _ in by) or not any(a == "default" for a, _ in by):
            continue

        def solved(arm, by=by):
            return sum(1 for (a, _), xs in by.items() if a == arm and sum(x["score"] == 1.0 for x in xs) * 2 > len(xs))

        def avg(arm, key, by=by):
            return statistics.mean(x[key] for (a, _), xs in by.items() if a == arm for x in xs)

        c, d = solved("clm"), solved("default")
        out.append((f"long {NAME[model]}: clm solves >= default - 1 task", c >= d - 1, f"clm {c} vs default {d}"))
        cp, dp = avg("clm", "peak_prompt"), avg("default", "peak_prompt")
        out.append((f"long {NAME[model]}: clm peak lower", cp < dp, f"clm {cp:.0f} vs default {dp:.0f}"))
        cc, dc = avg("clm", "cost_usd"), avg("default", "cost_usd")
        out.append((f"long {NAME[model]}: clm cost <= default", cc <= dc, f"clm ${cc:.2f} vs default ${dc:.2f}"))
    return out


def main(argv):
    runs = [a for a in argv if a != "--check"]
    rows = [r for r in load(runs) if "score" in r]
    print("\n".join(tables(rows)))
    res = checks(rows)
    print("\n### Pass checks\n")
    for name, ok, detail in res:
        print(f"- [{'x' if ok else ' '}] {name}: {detail}")
    print(f"\nepisodes graded: {len(rows)}; total cost ${sum(r['cost_usd'] for r in rows):.2f}")
    if "--check" in argv and (not rows or not all(ok for _, ok, _ in res)):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
