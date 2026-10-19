"""Grade the judge arms and check the preregistered gates: python3 -I analyze.py <out_dir>

Reads prompts.jsonl, labels.jsonl, verdicts.jsonl, cache.jsonl, ratios.json and ab.jsonl in out_dir, plus the
effortless cases, and writes out_dir/report.md. A section whose input is not produced yet is skipped with a note, and a
gate with a missing input is INCOMPLETE, which never counts as PASS.
"""

import itertools
import json
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import label  # noqa: E402

# Preregistered: do not change after the first --run.
G1_RIGHT = 0.90
G1_LOW = 0.03
G1_P95_MS = 1500
G2_RETENTION = 0.9
G3_SAVING = -0.10
G3_COST_CAP = 0.10
G3_HARD_GAIN = 2  # more hard-task A/B passes than at medium, out of the 12 hard runs
WARM_MS = 300_000
JUDGES = ("typesafe", "haiku")  # the other arms are baselines: graded, but never the G1 winner
SLICES = ("top", "child", "effortless", "effortless-holdout")
G1_SLICES = ("top", "effortless-holdout")
HARD_TASKS = ("c", "d")


def grade(level: str, ok: list[str]) -> str:
    i = common.LEVELS.index(level)
    if i < common.LEVELS.index(ok[0]):
        return "low"
    if i > common.LEVELS.index(ok[-1]):
        return "high"
    return "right"


def p95(xs: list[float]) -> float:
    """Nearest rank: the smallest value that at least 95% of xs are at or below."""
    return sorted(xs)[-(-95 * len(xs) // 100) - 1]


def stats(verdicts: list[dict], ok: dict[str, list[str]]) -> dict:
    """Grades of the labelled verdicts, kept errors included; kept share, latency and tokens of all of them."""
    grades = Counter(grade(v["level"], ok[v["id"]]) for v in verdicts if v["id"] in ok)
    labelled = grades.total()
    ms = [v["ms"] for v in verdicts]
    return {
        "rows": len(verdicts),
        "labelled": labelled,
        **{g: grades[g] / labelled if labelled else None for g in ("right", "low", "high")},
        "kept": sum(v["kept"] for v in verdicts) / len(verdicts),
        "p50": statistics.median_low(ms),  # the nearest-rank median
        "p95": p95(ms),
        "tokens": sum(v["tokens"] for v in verdicts),
    }


def flips(rows: list[dict]) -> list[dict]:
    """Level changes between consecutive judged turns of each session; a row's `level` is the one the turn runs at."""
    turns = sorted(rows, key=lambda r: (r["session_id"], r["seq"]))
    return [
        {"id": b["id"], "from": a["level"], "to": b["level"], "warm": b["gap_ms"] is not None and b["gap_ms"] < WARM_MS}
        for a, b in itertools.pairwise(turns)
        if a["session_id"] == b["session_id"] and a["level"] != b["level"]
    ]


def model_id(model: str | None) -> str:
    """`claude-code/claude-sonnet-5-5` reads as `claude-sonnet-5-5`."""
    return str(model).split("/")[-1]


def modeled_cost(row: dict, picked: str, ratios: dict, flip_safe: set[str], flipped_warm: bool) -> float:
    """The turn's cost at the picked level, plus the cache rewrite of a warm flip on a model that is not flip-safe."""
    model = model_id(row["model"])
    cost = row["cost_units"] * ratios[model][picked] / ratios[model][row["level"] or "medium"]
    if flipped_warm and model not in flip_safe:
        cost += row["first_read"] * (common.WEIGHTS["cache_write"] - common.WEIGHTS["cache_read"])
    return cost


def safe_models(cache: list[dict]) -> set[str]:
    """Models with 3 or more valid pairs, all keeping G2_RETENTION of the control's cache reads (R3)."""
    valid = defaultdict(list)
    for pair in cache:
        if not pair.get("invalid"):
            valid[model_id(pair["model"])].append(pair["retention"])
    return {model for model, kept in valid.items() if len(kept) >= 3 and min(kept) >= G2_RETENTION}


def judged(arm: str, corpus: list[dict], verdicts: dict) -> list[dict]:
    """The corpus turns the arm judged, in session order, each at the level it would run at under Auto.

    A kept verdict holds the level the session ran at (judge.verdict), but Auto keeps the level of the turn before,
    so a kept turn inherits the previous judged turn's level; only a session's first judged turn keeps its own.
    """
    out = []
    for row in sorted(corpus, key=lambda r: (r["session_id"], r["seq"])):
        if (verdict := verdicts.get((row["id"], arm))) is None:
            continue
        level = verdict["level"]
        if verdict["kept"] and out and out[-1]["session_id"] == row["session_id"]:
            level = out[-1]["level"]
        out.append(dict(row, level=level))
    return out


def costs(arm: str, corpus: list[dict], verdicts: dict, ratios: dict, safe: set[str]) -> dict[str, dict]:
    """Per model id: judged rows, and actual and modeled cost units of the rows with a ratio at both levels."""
    turns = judged(arm, corpus, verdicts)
    warm = {f["id"] for f in flips(turns) if f["warm"]}
    picked = {t["id"]: t["level"] for t in turns}
    out = defaultdict(lambda: {"rows": 0, "priced": 0, "actual": 0.0, "modeled": 0.0})
    for row in corpus:
        if (pick := picked.get(row["id"])) is None:
            continue
        model = model_id(row["model"])
        c = out[model]
        c["rows"] += 1
        levels = ratios.get(model, {})
        if pick in levels and (row["level"] or "medium") in levels:
            c["priced"] += 1
            c["actual"] += row["cost_units"]
            cost = modeled_cost(row, pick, ratios, safe, row["id"] in warm)
            # ponytail: judge tokens count 1:1 as the session model's input; a price table if it decides G3
            c["modeled"] += cost + verdicts[row["id"], arm]["tokens"]
    return out


def g2(cache: list[dict] | None, safe: set[str]) -> tuple[bool, str]:
    if not cache:
        return False, "INCOMPLETE: no cache.jsonl yet, so no model is flip-safe"
    pairs = defaultdict(list)
    for pair in cache:
        pairs[model_id(pair["model"])].append("invalid" if pair.get("invalid") else f"{pair['retention']:.2f}")
    models = "; ".join(
        f"{model} {'flip-safe' if model in safe else 'not flip-safe'} (retention {' '.join(kept)})"
        for model, kept in pairs.items()
    )
    return bool(safe), f"{'PASS' if safe else 'FAIL'}: {models}"


def g3(
    winner: str | None, corpus: list[dict], verdicts: dict, safe: set[str], ratios: dict | None, ab: list[dict] | None
) -> tuple[bool, str]:
    missing = (("no judge arm passes G1", winner is None), ("no ratios.json", not ratios), ("no ab.jsonl", not ab))
    gaps = [gap for gap, absent in missing if absent]
    if gaps:
        return False, "INCOMPLETE: " + "; ".join(gaps)
    per_model = costs(winner, corpus, verdicts, ratios, safe)
    priced = [c for model, c in per_model.items() if model in ratios]
    gaps += [
        f"{model}: {c['rows'] - c['priced']} rows at a level without a ratio"
        for model, c in per_model.items()
        if model in ratios and c["priced"] < c["rows"]
    ]
    actual, n = sum(c["actual"] for c in priced), sum(c["priced"] for c in priced)
    delta = sum(c["modeled"] for c in priced) / actual - 1 if actual else None
    left_out = ", ".join(f"{model} {c['rows']}" for model, c in per_model.items() if model not in ratios)
    cost = "no priced corpus cost" if delta is None else f"modeled cost {delta:+.1%} on {n} rows"
    if delta is None:
        gaps.append(cost)
    if left_out:
        cost += f" (left out, no ratios: {left_out})"
    runs = defaultdict(list)
    for run in ab:
        runs[run["model"], run["level"], run["task"]].append(bool(run["pass"]))
    picked, medium, hard, gain = [], [], 0, 0
    for model, task in sorted({(run["model"], run["task"]) for run in ab}):
        verdict = verdicts.get((f"ab-{model}-{task}", winner))  # R4: the winner's pick on that A/B prompt
        if verdict is None:
            gaps.append(f"no {winner} verdict for ab-{model}-{task}")
            continue
        at_pick, at_medium = runs.get((model, verdict["level"], task)), runs.get((model, "medium", task))
        if not (at_pick and at_medium):
            gaps.append(f"no A/B runs for {model} {task} at {verdict['level']} or medium")
            continue
        picked += at_pick
        medium += at_medium
        if task in HARD_TASKS:
            hard += len(at_medium)
            gain += sum(at_pick) - sum(at_medium)
    numbers = (
        f"{winner}: {cost}; A/B passes {sum(picked)}/{len(picked)} at the picked levels vs {sum(medium)}/{len(medium)}"
        f" at medium; hard tasks {gain:+d} of {hard}; flip-safe: {', '.join(sorted(safe)) or 'none'}"
    )
    if gaps:
        return False, f"INCOMPLETE: {'; '.join(gaps)}. {numbers}"
    saves = delta <= G3_SAVING and sum(picked) / len(picked) >= sum(medium) / len(medium)
    gains = gain >= G3_HARD_GAIN and delta <= G3_COST_CAP
    return saves or gains, f"{'PASS' if saves or gains else 'FAIL'}: {numbers}"


def gates(
    rows: list[dict], verdicts: dict, cache: list[dict] | None, ratios: dict | None, ab: list[dict] | None
) -> dict[str, tuple[bool, str]]:
    """G1, G2 and G3 as (passed, "PASS|FAIL|INCOMPLETE: <numbers>").

    rows are the prompt rows (labels merged in as `ok`) and the effortless rows, each with a slice in SLICES;
    verdicts are keyed by (id, arm); cache, ratios and ab are None until produced.
    """
    ok = {r["id"]: r["ok"] for r in rows if r.get("ok")}
    in_g1 = {r["id"] for r in rows if r["slice"] in G1_SLICES}
    # the preregistered population is top plus the holdout: no arm passes while one of them has no labels
    unlabelled = [s for s in G1_SLICES if not any(r["id"] in ok for r in rows if r["slice"] == s)]
    arms = list(dict.fromkeys(arm for _, arm in verdicts))
    lines, passing = [], []
    for arm in [*JUDGES, *(a for a in arms if a not in JUDGES)]:
        name = arm if arm in JUDGES else f"{arm} (baseline)"
        vs = [v for (rid, a), v in verdicts.items() if a == arm and rid in in_g1]
        s = stats(vs, ok) if vs else None
        if s is None:
            lines.append(f"{name}: no verdicts")
        elif not s["labelled"]:
            lines.append(f"{name} FAIL: no labelled rows")
        else:
            passed = not unlabelled and s["right"] >= G1_RIGHT and s["low"] <= G1_LOW and s["p95"] <= G1_P95_MS
            if passed and arm in JUDGES:
                passing.append((s["right"], -s["low"], arm))
            lines.append(
                f"{name} {'PASS' if passed else 'FAIL'} right {s['right']:.1%} low {s['low']:.1%} p95 {s['p95']} ms"
                f" on {s['labelled']} labelled of {s['rows']} rows"
            )
    winner = max(passing)[2] if passing else None
    if unlabelled:
        head = f"FAIL: no labels for {', '.join(unlabelled)} (label.py --run)."
    else:
        head = f"PASS: winner {winner}." if winner else "FAIL: no judge arm passes."
    safe = safe_models(cache or [])
    corpus = [r for r in rows if r["slice"] in ("top", "child")]
    return {
        "G1": (winner is not None, f"{head} {'; '.join(lines)}"),
        "G2": g2(cache, safe),
        "G3": g3(winner, corpus, verdicts, safe, ratios, ab),
    }


def pct(x: float | None) -> str:
    return "–" if x is None else f"{x:.1%}"


def report(
    rows: list[dict], verdicts: dict, cache: list[dict] | None, ratios: dict | None, ab: list[dict] | None
) -> str:
    ok = {r["id"]: r["ok"] for r in rows if r.get("ok")}
    slice_of = {r["id"]: r["slice"] for r in rows}
    corpus = [r for r in rows if r["slice"] in ("top", "child")]
    arms = list(dict.fromkeys(arm for _, arm in verdicts))
    out = ["# Auto-effort phase 0 report", "", "## Gates", ""]
    out.append(
        f"Bars: G1 right ≥ {G1_RIGHT:.0%}, low ≤ {G1_LOW:.0%} and p95 ≤ {G1_P95_MS} ms on top plus effortless-holdout;"
        f" G2 retention ≥ {G2_RETENTION} in every valid pair, at least 3 of them; G3 modeled cost ≤ {G3_SAVING:+.0%}"
        f" with A/B passes at the picked levels no lower than at medium, or {G3_HARD_GAIN} more hard-task passes"
        f" for ≤ {G3_COST_CAP:+.0%}."
    )
    out.append("")
    out += [f"- {name} {text}" for name, (_, text) in gates(rows, verdicts, cache, ratios, ab).items()]

    out += ["", "## Judge arms", ""]
    if not corpus:
        out.append("No prompts.jsonl rows yet (extract.py), so top and child are empty.")
    labelled = {s: [r["id"] in ok for r in rows if r["slice"] == s] for s in SLICES}
    if unlabelled := [s for s, has in labelled.items() if has and not any(has)]:
        out.append(f"No labels for: {', '.join(unlabelled)}. Their right, low and high stay blank (label.py --run).")
    if not verdicts:
        out.append("Skipped: no verdicts.jsonl yet (judge.py --run).")
    else:
        out += [
            "Right, low and high count the labelled rows, kept errors included; kept, ms and tokens count every row.",
            "",
            "| arm | slice | rows | labelled | right | low | high | kept | p50 ms | p95 ms | judge tokens |",
            "|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
        ]
        for arm, slice_ in itertools.product(arms, SLICES):
            if vs := [v for (rid, a), v in verdicts.items() if a == arm and slice_of.get(rid) == slice_]:
                s = stats(vs, ok)
                shares = " | ".join(pct(s[k]) for k in ("right", "low", "high", "kept"))
                out.append(
                    f"| {arm} | {slice_} | {s['rows']} | {s['labelled']} | {shares} | {s['p50']} | {s['p95']}"
                    f" | {s['tokens']} |"
                )

    out += ["", "## Flips", ""]
    if not (corpus and verdicts):
        out.append("Skipped: needs prompts.jsonl and verdicts.jsonl.")
    else:
        out += [
            "Level changes between consecutive judged turns of a session, where a kept turn stays at the level of the"
            f" judged turn before it; warm means under {WARM_MS // 1000} s since the previous assistant message.",
            "",
            "| arm | warm | cold |",
            "|---|--:|--:|",
        ]
        for arm in arms:
            changes = flips(judged(arm, corpus, verdicts))
            warm = sum(f["warm"] for f in changes)
            out.append(f"| {arm} | {warm} | {len(changes) - warm} |")

    out += ["", "## Modeled cost", ""]
    if ratios is None:
        out.append("Skipped: no ratios.json yet (ab.py --run).")
    elif not (corpus and verdicts):
        out.append("Skipped: needs prompts.jsonl and verdicts.jsonl.")
    else:
        safe = safe_models(cache or [])
        out += [
            "Corpus cost units per model under each arm, against the levels the sessions ran. Judge tokens count 1:1,"
            " and a warm flip pays its cache rewrite unless the model is flip-safe"
            f" ({', '.join(sorted(safe)) or 'none'}). Rows whose model or level has no ratio are not priced.",
            "",
            "| arm | model | rows | priced | actual | modeled | delta |",
            "|---|---|--:|--:|--:|--:|--:|",
        ]
        for arm in arms:
            for model, c in sorted(costs(arm, corpus, verdicts, ratios, safe).items()):
                units = "– | – | –"
                if c["actual"]:
                    units = f"{c['actual']:.0f} | {c['modeled']:.0f} | {c['modeled'] / c['actual'] - 1:+.1%}"
                out.append(f"| {arm} | {model} | {c['rows']} | {c['priced']} | {units} |")
    return "\n".join(out) + "\n"


def main(argv: list[str]) -> None:
    if len(argv) != 1:
        sys.exit("usage: python3 -I analyze.py <out_dir>")
    out = Path(argv[0])

    def read(name: str) -> list[dict] | None:
        return common.read_jsonl(out / name) if (out / name).exists() else None

    labels = {r["id"]: r["ok"] for r in read("labels.jsonl") or []}
    rows = [dict(r, ok=labels[r["id"]]) if r["id"] in labels else r for r in read("prompts.jsonl") or []]
    rows += [dict(r, slice="effortless-holdout" if r["holdout"] else "effortless") for r in label.effortless_rows()]
    verdicts = {(v["id"], v["arm"]): v for v in read("verdicts.jsonl") or []}  # R5: the last row of an (id, arm) wins
    path = out / "ratios.json"
    ratios = json.loads(path.read_text(encoding="utf-8")) if path.exists() else None
    text = report(rows, verdicts, read("cache.jsonl"), ratios, read("ab.jsonl"))
    (out / "report.md").write_text(text, encoding="utf-8")
    print(f"wrote {out / 'report.md'}")
    print("\n".join(line.removeprefix("- ") for line in text.splitlines() if line.startswith("- G")))


if __name__ == "__main__":
    main(sys.argv[1:])
