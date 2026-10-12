"""Offline checks for the bench (no stack, no API calls). Run: python test_bench.py"""

import json
import re
import subprocess
import tempfile
from pathlib import Path

import report
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


def test_long_snapshots():
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        run.build_snapshot("http-brace", root)
        assert "normalize_api_path" not in (root / "http/src/types.rs").read_text()
        assert "fn register_trigger_brace_and_colon_routes_conflict" in (root / "http/src/trigger.rs").read_text()
        assert (root / "crates/config-client/src/initialization.rs").exists()
        assert (root / "rust-toolchain.toml").exists()
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        run.build_snapshot("ide-update-op", root)
        assert "inferred_ops" in (root / "ide/tests/code_update_ops.rs").read_text()
        assert "inferred_ops" not in (root / "ide/src/code/functions/update_file.rs").read_text()
        assert (root / "ide/ui/dist/page.js").exists() and not (root / "ide/ui/package.json").exists()


def test_report_checks():
    base = {"level": 0, "seed": 0, "status": "completed", "llm_calls": 10, "drops": 0, "strategy": {}}
    kv = {"arm": "clm", "task": "kv", "level": 1000, "ops_delivered": 34, "ops_total": 34}
    rows = [
        {**base, **kv, "model": run.S55, "score": 1.0, "cost_usd": 1.0, "peak_prompt": 30000},
        {
            **base,
            "arm": "clm",
            "task": "http-brace",
            "model": run.S55,
            "score": 1.0,
            "cost_usd": 2.0,
            "peak_prompt": 4e4,
        },
        {
            **base,
            "arm": "default",
            "task": "http-brace",
            "model": run.S55,
            "score": 1.0,
            "cost_usd": 3.0,
            "peak_prompt": 9e4,
        },
    ]
    assert all(ok for _, ok, _ in report.checks(rows)) and len(report.checks(rows)) == 7
    stats = {"peak_tc": 28000, "edits_applied": 3, "rollbacks": 1}
    rows[0]["strategy"] = {"found": True, "budget_tokens": 60621, "stats": stats}
    row = "| kv 1000 clm | 1 | 1.00 | 1.00 | 1.00 | 30000 | 28000 | 0/1 | 3.0 | 1.0 | – | – | – | 0 | 1.00 |"
    assert row in report.tables(rows)
    sums = [{**rows[0], "arm": "sum", "strategy": {"stats": {"summary_cost_unknown": n}}} for n in (1, 2)]
    assert any(line.startswith("| kv 1000 sum | 2 |") and "| 3 | 0 |" in line for line in report.tables(sums))
    rows[0]["drops"] = 2
    rows[1]["cost_usd"] = 5.0
    failed = [name for name, ok, _ in report.checks(rows) if not ok]
    assert failed == ["zero thinking-drop events on clm", "long Sonnet 5.5: clm cost <= default"]
    with tempfile.TemporaryDirectory() as d:
        Path(d, "results").mkdir()
        for i, r in enumerate(rows):
            Path(d, "results", f"{i}.json").write_text(json.dumps(r))
        assert report.main([d, "--check"]) == 1 and report.main([d]) == 0


def test_report_check_fails_without_clm_results():
    base = {"level": 1000, "seed": 0, "status": "completed", "llm_calls": 10, "drops": 0, "strategy": {}}
    row = {**base, "arm": "clm", "task": "kv", "model": run.S55, "score": 1.0, "cost_usd": 1.0, "peak_prompt": 3e4}
    row["ops_delivered"] = row["ops_total"] = 34

    def failed(rows, crashed=()):
        return [name for name, ok, _ in report.checks(rows, crashed) if not ok]

    assert failed([row]) == []
    # every clm episode of a group crashed while another arm finished
    assert failed([{**row, "arm": "sum"}]) == ["clm rows graded", "regression kv 1000 Sonnet 5.5"]
    # a clm row whose provider log was not found is not zero drops
    assert failed([{**row, "drops": None}]) == ["zero thinking-drop events on clm"]
    long = {**row, "arm": "default", "task": "http-brace", "level": 0}
    assert failed([long]) == ["clm rows graded", "long Sonnet 5.5: clm and default graded"]
    assert failed([row], ["clm-kv1000-s0-s55"]) == ["no crashed episodes"]
    with tempfile.TemporaryDirectory() as d:
        Path(d, "results").mkdir()
        Path(d, "results", "clm-kv1000-s1-s55.json").write_text(json.dumps({**row, "sid": "clm-kv1000-s1-s55"}))
        assert report.main([d, "--check"]) == 0
        crash = {"episode": {}, "sid": "clm-kv1000-s0-s55", "crash": "RuntimeError('strategy not effective')"}
        Path(d, "episodes.out").write_text(json.dumps(crash) + "\n")
        assert report.main([d, "--check"]) == 1
        # a crash whose episode was rerun to a result no longer counts
        Path(d, "results", "clm-kv1000-s0-s55.json").write_text(json.dumps({**row, "sid": "clm-kv1000-s0-s55"}))
        assert report.main([d, "--check"]) == 0


def test_missing_tests_needs_an_ok_line():
    out = "\n".join(
        [
            "test a::passes ... ok",
            "test a::fails ... FAILED",
            "test a::skipped ... ignored",
            "test a::skipped_with_reason ... ignored, slow",
            "test a::long_name_extra ... ok",
            "test a::panics - should panic ... ok",
        ]
    )
    required = [
        "a::passes",
        "a::fails",
        "a::skipped",
        "a::skipped_with_reason",
        "a::absent",
        "a::long_name",
        "a::panics",
    ]
    assert run.missing_tests(out, required) == [
        "a::fails",
        "a::skipped",
        "a::skipped_with_reason",
        "a::absent",
        "a::long_name",
    ]
    assert run.missing_tests(out, []) == []


def test_splice_test_module_restores_the_fix_tests():
    fix = "fn f() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn pins_f() {}\n}\n"
    mine = "fn f() { /* mine */ }\nfn g() {}\n\n#[cfg(test)]\nmod tests {\n}\n"
    got = run.splice_test_module(mine, fix)
    assert got == "fn f() { /* mine */ }\nfn g() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn pins_f() {}\n}\n"
    # a solution that dropped the module gets the fix's module appended
    assert (
        run.splice_test_module("fn f() {}", fix)
        == "fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn pins_f() {}\n}\n"
    )
    assert run.splice_test_module(got, fix) == got


def test_run_cargo_timeout_kills_the_group():
    with tempfile.TemporaryDirectory() as d:
        rc, _, _ = run.run_cargo(["bash", "-c", "sleep 60 & wait"], Path(d), timeout=1)
        assert rc == 124
        rc, out, _ = run.run_cargo(["bash", "-c", "echo $CARGO_TARGET_DIR"], Path(d))
        assert rc == 0 and out.strip() == f"{d}/target"


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("ok", name)
