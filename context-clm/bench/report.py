"""Aggregate RUN/results/*.json into markdown tables and the spec §7 pass checks.

  python report.py RUN [RUN ...] [--check]

--check exits 1 when a pass check fails or nothing was graded. A crashed episode (a `crash` line in RUN/episodes.out
with no result for the same arm, task, level, seed and model since, under any tag), a refused grade (run.py did not
grade it), a group with rows for another arm but none for clm, and an Anthropic clm row without a provider log all
fail it: a missing result never passes as a good one. Rows of other models have no spike baseline, and rows of other
providers no thinking-drop count (n/a).
"""

import json
import re
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
WALL = re.compile(r"context_budget_stop|context_overflow|max_turns|max_cost|budget")
REGRESSION = {(t, lv) for t, lv, _ in SPIKE_CLM}  # the §7.1 groups


def name(model):
    return NAME.get(model, model)


def models(rows):
    """The Claude models, then every other model the rows have."""
    return [*NAME, *sorted({r["model"] for r in rows} - NAME.keys())]


def anthropic(r):
    return r.get("provider", "anthropic") == "anthropic"


def drops(xs):
    """Thinking-drop events (provider-anthropic's log only); "+?" when some rows have no log, n/a on other providers."""
    xs = [x for x in xs if anthropic(x)]
    if not xs:
        return "n/a"
    return f"{sum(x.get('drops') or 0 for x in xs)}{'+?' * any(x.get('drops') is None for x in xs)}"


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


def total(xs, k):
    v = [st(x, k) for x in xs if st(x, k) is not None]
    return str(sum(v)) if v else "–"


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
    # summary calls that timed out or failed: billed or not, their cost is missing from "cost $"
    ("summary cost unknown", lambda xs: total(xs, "summary_cost_unknown")),
    ("drops", drops),
]


def tables(rows):
    out = []
    ctx = [r for r in rows if r["task"] in ("kv", "log", "needle")]
    ops = ("ops done", lambda xs: fmt(mean([x["ops_delivered"] / x["ops_total"] for x in xs])))
    for model in models(ctx):
        sel = [r for r in ctx if r["model"] == model]
        out += table(
            f"ContextBench-style — {name(model)}", lambda r: f"{r['task']} {r['level']} {r['arm']}", sel, [*COMMON, ops]
        )
    fields = [r for r in rows if r["task"] == "fields"]
    recall = ("recall", lambda xs: fmt(mean([x.get("recall") for x in xs])))
    out += table(
        "Code reading (exact answer)",
        lambda r: f"fields{r['level']} {name(r['model'])} {r['arm']}",
        fields,
        [*COMMON, recall],
    )
    code = [r for r in rows if r["task"] in CODE]
    calls = ("llm calls", lambda xs: fmt(mean([x["llm_calls"] for x in xs]), 0))
    # rows run.py did not grade (score 0 until cleared, see the README; --check fails on them)
    refused = ("grade refused", lambda xs: str(sum(1 for x in xs if x.get("grade_refused"))))
    out += table(
        "Long coding tasks (cargo test)",
        lambda r: f"{r['task']} {name(r['model'])} {r['arm']}",
        code,
        [*COMMON, calls, refused],
    )
    return out


def checks(rows, crashed=()):
    """[(name, ok, detail)] for spec §7.1 (regression) and §7.2 (long tasks), over whatever rows are present.
    `crashed`: session ids of episodes that crashed without a result."""
    out = [("no crashed episodes", not crashed, ", ".join(sorted(crashed)[:10]) or "none")]
    refused = sorted(r.get("sid") or "?" for r in rows if r.get("grade_refused"))
    out.append(("no refused grades", not refused, ", ".join(refused[:10]) or "none"))
    clm = [r for r in rows if r["arm"] == "clm"]
    out.append(("clm rows graded", bool(clm), f"{len(clm)} rows"))
    # a turn may end at the wall (budget stop, overflow, max_turns, cost cap); anything else is an upstream failure
    # (expired login, rate limit, provider error) that would grade every arm 0 and pass the comparisons vacuously
    upstream = [r for r in rows if r.get("status") == "failed" and not WALL.search(json.dumps(r.get("error")))]
    out.append(
        ("no upstream turn failures", not upstream, ", ".join(r.get("sid", "?") for r in upstream[:5]) or "none")
    )
    # the spike's groups first, then the regression groups of models the spike never ran (no baseline to fail)
    other = sorted({(r["task"], r["level"], r["model"]) for r in rows if (r["task"], r["level"]) in REGRESSION})
    other = [k for k in other if k not in SPIKE_CLM]
    for task, level, model in [*SPIKE_CLM, *other]:
        group = [r for r in rows if (r["task"], r["level"], r["model"]) == (task, level, model)]
        xs = [r["score"] for r in group if r["arm"] == "clm"]
        label, spike = f"regression {task} {level} {name(model)}", SPIKE_CLM.get((task, level, model))
        if xs and spike is None:
            # no spike baseline (a model the spike never ran): clm must match the unmanaged base arm of the same group
            got, base = statistics.mean(xs), [r["score"] for r in group if r["arm"] == "base"]
            if base:
                b = statistics.mean(base)
                out.append((label, got >= b - 0.05, f"clm {got:.2f} vs base {b:.2f} (no spike baseline)"))
            else:
                out.append((label, False, f"clm {got:.2f}; no spike baseline and no base rows to compare"))
        elif xs:
            got = statistics.mean(xs)
            out.append((label, got >= spike, f"clm {got:.2f} vs spike {spike:.2f}"))
        elif group:
            out.append((label, False, "no clm rows (other arms have some)"))
    if clm:
        # only provider-anthropic logs the event: other providers' clm rows are n/a, never a failure
        ant = [r for r in clm if anthropic(r)]
        n = sum(r.get("drops") or 0 for r in ant)
        unknown = sum(1 for r in ant if r.get("drops") is None)
        detail = f"{n} events" + (f"; {unknown} rows without a provider log" if unknown else "")
        if len(ant) < len(clm):
            detail += f"; n/a on {len(clm) - len(ant)} rows of other providers"
        out.append(("zero thinking-drop events on clm", n == 0 and not unknown, detail))
    long = [r for r in rows if r["task"] in CODE or (r["task"] == "fields" and r["level"] == 40)]
    for model in models(long):
        by = defaultdict(list)
        for r in long:
            if r["model"] == model:
                by[(r["arm"], r["task"])].append(r)
        has = {a for a, _ in by}
        if not has & {"clm", "default"}:
            continue
        if not {"clm", "default"} <= has:
            out.append((f"long {name(model)}: clm and default graded", False, f"only {sorted(has)}"))
            continue

        def solved(arm, by=by):
            return sum(1 for (a, _), xs in by.items() if a == arm and sum(x["score"] == 1.0 for x in xs) * 2 > len(xs))

        def avg(arm, key, by=by):
            return statistics.mean(x[key] for (a, _), xs in by.items() if a == arm for x in xs)

        c, d = solved("clm"), solved("default")
        out.append((f"long {name(model)}: clm solves >= default - 1 task", c >= d - 1, f"clm {c} vs default {d}"))
        cp, dp = avg("clm", "peak_prompt"), avg("default", "peak_prompt")
        out.append((f"long {name(model)}: clm peak lower", cp < dp, f"clm {cp:.0f} vs default {dp:.0f}"))
        if all(anthropic(x) for xs in by.values() for x in xs):
            cc, dc = avg("clm", "cost_usd"), avg("default", "cost_usd")
            out.append((f"long {name(model)}: clm cost <= default", cc <= dc, f"clm ${cc:.2f} vs default ${dc:.2f}"))
        else:
            # no pricing (cost reads 0): compare the prompt tokens the provider billed instead
            def toks(arm, by=by):
                return statistics.mean(
                    (x.get("input") or 0) + (x.get("cache_read") or 0) + (x.get("cache_write") or 0)
                    for (a, _), xs in by.items()
                    if a == arm
                    for x in xs
                )

            ct, dt = toks("clm"), toks("default")
            out.append(
                (f"long {name(model)}: clm prompt tokens <= default", ct <= dt, f"clm {ct:.0f} vs default {dt:.0f}")
            )
    return out


def episode_id(x):
    """What a rerun keeps: the sid without its tag. run.py refuses to rerun a crashed sid, so a rerun has a new tag."""
    return (x.get("arm"), x.get("task"), x.get("level") or 0, x.get("seed"), x.get("model"))


def crashes(runs, loaded):
    """Session ids that crashed (RUN/episodes.out `crash` lines, results without a score) with no graded result for
    the same episode (`episode_id`) since."""
    graded = {episode_id(r) for r in loaded if "score" in r}
    out = {r.get("sid") or "?" for r in loaded if "score" not in r and episode_id(r) not in graded}
    for r in runs:
        p = Path(r, "episodes.out")
        lines = p.read_text().splitlines() if p.exists() else []
        crashed = [c for c in map(json.loads, lines) if "crash" in c]
        out |= {c.get("sid") or "?" for c in crashed if episode_id(c.get("episode") or {}) not in graded}
    return sorted(out)


def main(argv):
    runs = [a for a in argv if a != "--check"]
    loaded = load(runs)
    rows = [r for r in loaded if "score" in r]
    print("\n".join(tables(rows)))
    res = checks(rows, crashes(runs, loaded))
    print("\n### Pass checks\n")
    for name, ok, detail in res:
        print(f"- [{'x' if ok else ' '}] {name}: {detail}")
    print(f"\nepisodes graded: {len(rows)}; total cost ${sum(r['cost_usd'] for r in rows):.2f}")
    if "--check" in argv and (not rows or not all(ok for _, ok, _ in res)):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
