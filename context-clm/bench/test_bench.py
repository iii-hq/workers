"""Offline checks for the bench (no stack, no API calls). Run: python test_bench.py"""

import json
import re
import subprocess
import tempfile
from pathlib import Path

import run
from ctxbench import answer, answer_re, gen
from skills import DROP_FILLER, OFFLOAD, answer_cmd, skill


def test_generator_is_deterministic_and_shaped():
    a, b = gen("kv", 100, 0), gen("kv", 100, 0)
    assert a == b
    assert len(a["ops"]) == 1 + 24 and a["truth"]["first_query_op"] == 1
    assert a["ops"][0].count("\nSET K") == 100 and "SET-BATCH 0000#" in a["ops"][0]
    n = gen("needle", 24, 0)
    assert len(n["ops"]) == 24 and all(x in "\n".join(n["ops"]) for x in n["truth"]["needles"])
    lg = gen("log", 1600, 1)
    assert len(lg["truth"]["answers"]) == 24 and lg["truth"]["first_query_op"] == len(lg["ops"]) - 24


def test_offload_and_answer_commands_work_on_a_mirror():
    op = gen("kv", 100, 0)["ops"][0]
    key, val = gen("kv", 100, 0)["truth"]["gets"][0]
    with tempfile.TemporaryDirectory() as d:
        mirror = Path(d, ".clm", "s.ctx")
        mirror.parent.mkdir()
        mirror.write_text(f"[[CTX_TURN 1 role=user]]\n{op}\n\n[[CTX_TURN 2 role=assistant]]\nok")
        cmd = OFFLOAD.format(mirror=".clm/s.ctx", block="SET-BATCH", file="kv.txt")
        subprocess.run(["bash", "-c", cmd], cwd=d, check=True)
        left = mirror.read_text()
        assert "SET K00000" not in left and "[SET-BATCH 0000#" in left and "offloaded to offload/kv.txt" in left
        out = subprocess.run(
            ["bash", "-c", answer_cmd("offload/kv.txt").replace("K00042", key)], cwd=d, capture_output=True, text=True
        ).stdout
        assert out == answer("key", key, val) + "\n"
        assert re.findall(answer_re("key", key), out, flags=re.S) == [val]


def test_drop_filler_keeps_needles():
    g = gen("needle", 24, 0)
    with tempfile.TemporaryDirectory() as d:
        Path(d, "m.ctx").write_text("[[CTX_TURN 1 role=user]]\n" + g["ops"][0])
        subprocess.run(["bash", "-c", DROP_FILLER.format(mirror="m.ctx")], cwd=d, check=True)
        left = Path(d, "m.ctx").read_text()
        assert "FILLER-BLOCK" not in left and all(n in left for n in g["truth"]["needles"] if n.startswith("[n00000"))


def test_skills_per_arm():
    for task in ("kv", "log", "needle"):
        assert ".clm/s.ctx" in skill("clm", task, ".clm/s.ctx", 60621)
        assert "45.5K of 60,621 tokens" in skill("sum", task, ".clm/s.ctx", 60621)
    assert "readout" in skill("base", "kv", "", 60621) and "readout" not in skill("default", "kv", "", 60621)


def test_grid_and_estimate():
    reg = run.grid("regression")
    assert len(reg) == 65 and {e["arm"] for e in reg} == {"clm", "sum", "base", "default"}
    assert not [e for e in reg if e["arm"] == "default" and e["task"] == "needle"]
    assert len(run.grid("long")) == 36 and len(run.grid("long-pilot")) == 6
    eps = [
        {"arm": "clm", "task": "kv", "level": 1000, "seed": 0, "model": run.S55},
        {"arm": "default", "task": "kv", "level": 1000, "seed": 0, "model": run.S55},
    ]
    expected, cap = run.estimate(eps, run.PRIORS)
    assert (round(expected, 2), cap) == (15.55, 30)
    with tempfile.TemporaryDirectory() as d:
        Path(d, "results").mkdir()
        Path(d, "results", "x.json").write_text(json.dumps({**eps[1], "cost_usd": 2.0}))
        assert run.estimate(eps, run.load_priors([d]))[0] == 2.55
        Path(d, "eps.json").write_text(json.dumps(eps))
        assert run.main(["run", str(Path(d, "r")), str(Path(d, "eps.json"))]) == 2
        assert not Path(d, "r").exists()


def test_graders():
    truth = [{"id": "a::x", "fields": ["p", "q"]}, {"id": "a::y", "fields": []}]
    reply = 'draft {"a::x": ["p"]} final {"a::x": ["p", "q"], "a::y": []}'
    assert run.score_fields(reply, truth) == {"score": 1.0, "precision": 1.0, "recall": 1.0}
    assert run.score_fields('{"a::x": ["p", "z"]}', truth) == {"score": 0.0, "precision": 0.5, "recall": 0.5}
    assert run.needle_score("[[user]]\nx\n[n1] a.  \n", ["[n1] a.", "[n2] b."]) == 0.5
    entries = [{"message": {"role": "assistant", "content": [{"type": "text", "text": answer("key", "K1", "v1")}]}}]
    assert run.answer_score({"t1": entries}, lambda op: "t1", 0, [["K1", "v1"]], "key") == 1.0
    with tempfile.TemporaryDirectory() as d:
        (Path(d) / "logs").mkdir()
        Path(d, "logs", "provider-anthropic.log").write_text(
            f"WARN x: {run.DROP} session_id=clm-a model=m\nWARN x: {run.DROP} session_id=clm-ab model=m\n"
            # tracing's ANSI formatter wraps the field name, the "=" and the message in colour escapes
            f"\x1b[33m WARN\x1b[0m x: {run.DROP} \x1b[3msession_id\x1b[0m\x1b[2m=\x1b[0mclm-a model=m\n"
        )
        assert run.drop_events(Path(d), "clm-a") == 2 and run.drop_events(Path(d), "nope") == 0


def test_harness_truth_matches_the_golden_schemas():
    names = run.git("ls-tree", "--name-only", run.FIELDS_SNAPSHOT, "harness/tests/golden/schemas/").decode().split()
    golden = {}
    for name in names:
        spec = json.loads(run.git("show", f"{run.FIELDS_SNAPSHOT}:{name}"))
        if "function_id" not in spec:
            continue
        schema, props = spec["request_schema"], set()
        for part in [schema, *schema.get("oneOf", []), *schema.get("anyOf", [])]:
            props |= set(part.get("properties", {}))
        golden[spec["function_id"]] = sorted(props)
    truth = {t["id"]: t["fields"] for t in run.TRUTH if t["scope"] == "harness"}
    assert truth == golden and len([t for t in run.TRUTH if t["scope"] == "ide"]) == 24


def test_strategy_status_reads_the_nested_shape():
    good = {"found": True, "budget_tokens": 4096, "stopped": False, "stats": {"peak_tc": 3000, "rollbacks": 1}}
    flat = {"found": True, "budget_tokens": 4096, "peak_tokens": 3000, "rollbacks": 1}
    other_budget = {**good, "budget_tokens": 65536}
    real = run.call
    try:
        for reply, ok in [(good, True), (flat, False), (other_budget, False), ({"found": False}, False)]:
            run.call = lambda fn, payload, reply=reply: reply
            try:
                got = run.strategy_status("clm", "s", 4096)
            except RuntimeError:
                got = None
            assert got == (reply if ok else None), reply
    finally:
        run.call = real


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("ok", name)
