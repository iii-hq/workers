"""Offline checks for the bench (no stack, no API calls). Run: python test_bench.py"""

import io
import json
import os
import re
import subprocess
import sys
import tempfile
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

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
            ["bash", "-c", answer_cmd("offload/kv.txt").replace("K99999", key)], cwd=d, capture_output=True, text=True
        ).stdout
        assert out == answer("key", key, val) + "\n"
        assert re.findall(answer_re("key", key), out, flags=re.S) == [val]


def test_answer_re_takes_only_a_whole_block_for_that_key():
    good = answer("key", "K1", "v1")

    def got(text):
        return [x.strip() for x in re.findall(answer_re("key", "K1"), text, flags=re.S)]

    assert got(good) == ["v1"]
    # an unclosed block before a good one does not swallow it
    assert got("#### ANSWER key=K1\nhalf an answer\n\n" + good) == ["v1"]
    assert got(answer("key", "K10", "v10")) == [] and got(answer("qid", "K1", "q")) == []
    assert got("the answer is #### ANSWER key=K1\nv1\n#### ANSWER DONE") == []  # not on a line of its own
    assert got("#### ANSWER key=K1\nv1\n#### ANSWER key=K2\nv2\n#### ANSWER DONE") == []


def test_examples_use_keys_no_task_has():
    kv, lg = gen("kv", 4000, 0), gen("log", 3200, 0)
    assert "key=K99999" in kv["instruction"] and "K00042" not in kv["instruction"]
    assert "qid=99" in lg["instruction"] and max(q for q, _ in lg["truth"]["answers"]) < 99
    assert "SET K99999 = " in skill("clm", "kv", ".clm/s.ctx", 60621)
    for bad in (("nope", "kv"), ("clm", "nope")):
        try:
            skill(*bad, ".clm/s.ctx", 60621)
        except AssertionError:
            continue
        raise AssertionError(f"skill{bad} accepted")


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
    clamped = {**good, "budget_tokens": 3000}  # clm lowers the budget to the model's usable room
    real = run.call
    try:
        for arm, reply, ok in [
            ("clm", good, True),
            ("clm", clamped, True),
            ("sum", good, True),
            ("sum", clamped, False),  # the arms never clamp
            ("clm", flat, False),
            ("clm", {**good, "budget_tokens": 65536}, False),
            ("clm", {**good, "budget_tokens": 0}, False),
            ("clm", {**good, "budget_tokens": None}, False),
            ("clm", {"found": False}, False),
        ]:
            run.call = lambda fn, payload, reply=reply: reply
            try:
                got = run.strategy_status(arm, "s", 4096)
            except RuntimeError:
                got = None
            assert got == (reply if ok else None), (arm, reply)
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
    assert all(ok for _, ok, _ in report.checks(rows)) and len(report.checks(rows)) == 8
    # a refused grade (run.py did not grade the episode) fails the check and has its own column, apart from failures
    refused = {**rows[1], "score": 0.0, "grade_refused": True, "grade_refused_reason": "cargo config changed"}
    # (alone, "clm solves >= default - 1" would pass on 0 >= 1 - 1)
    assert [n for n, ok, _ in report.checks([rows[0], refused, rows[2]]) if not ok] == ["no refused grades"]
    code = [line for line in report.tables([refused, rows[1]]) if line.startswith("| http-brace Sonnet 5.5 clm |")]
    assert code and code[0].startswith("| http-brace Sonnet 5.5 clm | 2 | 0.50 |") and code[0].endswith(" | 1 |")
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
        s1 = {**row, "seed": 1, "sid": "clm-kv1000-s1-s55"}
        Path(d, "results", "clm-kv1000-s1-s55.json").write_text(json.dumps(s1))
        assert report.main([d, "--check"]) == 0
        ep = {"arm": "clm", "task": "kv", "level": 1000, "seed": 0, "model": run.S55}
        crash = {"episode": ep, "sid": "clm-kv1000-s0-s55", "crash": "RuntimeError('strategy not effective')"}
        Path(d, "episodes.out").write_text(json.dumps(crash) + "\n")
        assert report.main([d, "--check"]) == 1  # seed 1's result is another episode
        # run.py refuses to rerun the crashed sid (its workspace exists), so the rerun gets a new tag and a new sid
        rerun = {**row, "sid": "clm-kv1000-s0-s55r2", "tag": "r2"}
        Path(d, "results", "clm-kv1000-s0-s55r2.json").write_text(json.dumps(rerun))
        assert report.main([d, "--check"]) == 0
        # a crash line without its episode is never cleared
        Path(d, "episodes.out").write_text(json.dumps({**crash, "episode": {}}) + "\n")
        assert report.main([d, "--check"]) == 1


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
        rc, _, _ = run.run_cargo(["bash", "-c", "sleep 60 & wait"], Path(d), Path(d, "t"), timeout=1)
        assert rc == 124


def test_grader_refuses_a_changed_cargo_config():
    saved = {k: os.environ.get(k) for k in ("ANTHROPIC_API_KEY", "CARGO_HOME")}
    baseline, long, restore = run.CARGO_CONFIG, run.LONG, run.restore_protected
    try:
        with tempfile.TemporaryDirectory() as d:
            os.environ["ANTHROPIC_API_KEY"] = "secret"
            os.environ["CARGO_HOME"] = d
            run.CARGO_CONFIG = run.cargo_config()
            Path(d, "config.toml").write_text('[build]\nrustflags = ["--cfg", "graded"]\n')
            ok, missing, log, refused = run.grade_code("http-brace", Path(d, "never-restored"))
            assert (ok, missing) == (False, []) and "changed" in refused and log == f"not graded: {refused}", log
            assert not Path(d, "never-restored").exists()
            # a config written while this grade's cargo runs (by a parallel episode) refuses this grade too
            Path(d, "config.toml").unlink()
            run.restore_protected = lambda t, root: None
            poison = 'echo "test t ... ok"; echo "[build]" > "$CARGO_HOME/config.toml"'
            run.LONG = {"x": {"cwd": ".", "grade": [["bash", "-c", poison]], "required": ["t"]}}
            root = Path(d, "workspace", "s")
            root.mkdir(parents=True)
            ok, _, log, refused = run.grade_code("x", root)
            assert not ok and "during" in refused and log.endswith(f"not graded: {refused}"), log
            Path(d, "config.toml").unlink()
            run.LONG["x"]["grade"] = [["echo", "test t ... ok"]]
            assert run.grade_code("x", root)[::3] == (True, None)
    finally:
        run.CARGO_CONFIG, run.LONG, run.restore_protected = baseline, long, restore
        for k, v in saved.items():
            os.environ.pop(k, None) if v is None else os.environ.__setitem__(k, v)


def test_grader_builds_in_its_own_target_dir_with_a_minimal_env():
    """Cargo gets no API keys and builds in <RUN>/grade-target/<task>, never in the episode's own <crate>/target."""
    long, restore = run.LONG, run.restore_protected
    try:
        run.restore_protected = lambda t, root: None
        run.LONG = {"x": {"cwd": "c", "grade": [["env"]], "required": []}}
        with tempfile.TemporaryDirectory() as d, mock.patch.dict(os.environ, ANTHROPIC_API_KEY="secret"):
            root = Path(d, "workspace", "s")
            (root / "c").mkdir(parents=True)
            ok, _, log, refused = run.grade_code("x", root)
            assert ok and refused is None, log
            env = dict(line.split("=", 1) for line in log.splitlines()[1:] if "=" in line)
            assert set(env) <= {*run.GRADE_ENV, "CARGO_TARGET_DIR"}, env
            assert env["CARGO_TARGET_DIR"] == f"{d}/grade-target/x" and env["HOME"] == os.environ["HOME"], env
    finally:
        run.LONG, run.restore_protected = long, restore


def test_grades_of_one_task_never_reuse_each_others_build():
    """Two workspaces share their task's grade target dir. Cargo reuses a build whose sources are older than it, so
    the workspace graded second (with older files, as an episode that started earlier has) must still be built."""
    lib = "pub fn v() -> u32 {{ {} }}\n#[test]\nfn t() {{ assert_eq!(v(), 2); }}\n"
    long, restore = run.LONG, run.restore_protected
    try:
        run.restore_protected = lambda t, root: None
        run.LONG = {"x": {"cwd": "c", "grade": [["cargo", "test", "--offline", "--lib"]], "required": ["t"]}}
        with tempfile.TemporaryDirectory() as d:
            for sid, v in (("a", 2), ("b", 1)):
                Path(d, "workspace", sid, "c", "src").mkdir(parents=True)
                Path(d, "workspace", sid, "c", "Cargo.toml").write_text(
                    '[package]\nname = "c"\nversion = "0.1.0"\nedition = "2021"\n'
                )
                Path(d, "workspace", sid, "c", "src", "lib.rs").write_text(lib.format(v))
            for f in Path(d, "workspace", "b").rglob("*"):
                os.utime(f, (1, 1))
            assert run.grade_code("x", Path(d, "workspace", "a"))[0]
            ok, missing, log, _ = run.grade_code("x", Path(d, "workspace", "b"))
            assert not ok and missing == ["t"], log
    finally:
        run.LONG, run.restore_protected = long, restore


def test_restore_puts_back_the_toolchain_and_refuses_a_planted_cargo_config():
    t = run.LONG["http-brace"]
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        run.build_snapshot("http-brace", root)
        fake = '[toolchain]\npath = "/tmp/fake"\n'
        stray = ["rust-toolchain", "http/rust-toolchain.toml", "http/.cargo/config.toml", ".cargo/credentials.toml"]
        for f in ["rust-toolchain.toml", *stray]:
            (root / f).parent.mkdir(parents=True, exist_ok=True)
            (root / f).write_text(fake)
        assert run.restore_protected(t, root).endswith("the fix commit does not have: http/.cargo/config.toml")
        assert (root / "rust-toolchain.toml").read_bytes() == run.git("show", f"{t['fix']}:rust-toolchain.toml")
        assert not [f for f in [*stray, "http/.cargo", ".cargo"] if (root / f).exists()]
        (root / ".cargo").mkdir()
        (root / ".cargo" / "config").write_text("[build]\n")
        ok, missing, log, refused = run.grade_code("http-brace", root)  # refuses before cargo runs
        assert (ok, missing) == (False, []) and refused.endswith("the fix commit does not have: .cargo/config"), log


def test_grader_refuses_a_symlinked_graded_path():
    """A graded path (or a parent dir) linked out of the snapshot would let the unconfined grader write through it."""
    t = run.LONG["http-brace"]
    with tempfile.TemporaryDirectory() as d:
        root, outside = Path(d, "ws"), Path(d, "outside")
        root.mkdir()
        outside.mkdir()
        run.build_snapshot("http-brace", root)
        victim = outside / "victim.rs"
        victim.write_text("mine\n")
        (root / "http/src/trigger.rs").unlink()
        (root / "http/src/trigger.rs").symlink_to(victim)
        why = run.restore_protected(t, root)
        assert victim.read_text() == "mine\n" and (root / "http/src/trigger.rs").is_symlink()
        assert why and why.endswith(": http/src/trigger.rs"), why
        ok, missing, log, refused = run.grade_code("http-brace", root)  # refuses before cargo runs
        assert (ok, missing) == (False, []) and refused == why and log == f"not graded: {why}", log
        # a linked parent dir (the restored tests/) is refused too, before anything is written
        (root / "http/src/trigger.rs").unlink()
        (root / "http/tests").rename(outside / "tests")
        (root / "http/tests").symlink_to(outside / "tests")
        before = {p.name: p.read_bytes() for p in (outside / "tests").iterdir() if p.is_file()}
        why = run.restore_protected(t, root)
        assert why and why.endswith(": http/tests"), why
        assert {p.name: p.read_bytes() for p in (outside / "tests").iterdir() if p.is_file()} == before
        assert not (root / "http/src/trigger.rs").exists()  # nothing was spliced either


def test_grid_on_another_provider():
    """Sonnet slots take --model, Opus slots --model2; without --model2 the Opus groups are dropped (logged)."""
    err = io.StringIO()
    with redirect_stderr(err):
        one = run.grid("regression", "openai-codex", "gpt-a")
    assert len(one) == 65 - 17 and {(e["model"], e["provider"]) for e in one} == {("gpt-a", "openai-codex")}
    assert "dropped 17 " in err.getvalue()
    two = run.grid("regression", "openai-codex", "gpt-a", "gpt-b")
    assert len(two) == 65 and {e["model"] for e in two if e.get("max_cost_usd") == 30} == {"gpt-b"}
    assert len({run.sid_of(e) for e in two}) == 65 and run.sid_of(two[0]) == "clm-kv1000-s0-gpt-a"
    assert run.grid("regression") == run.grid("regression", "anthropic", run.S55, run.O55)  # the default is unchanged
    for bad in (("openai-codex",), ("openai-codex", "m", "m"), ("anthropic", None, "m")):
        try:
            run.grid("long", *bad)
        except SystemExit:
            continue
        raise AssertionError(f"grid{bad} accepted")
    out = io.StringIO()
    with redirect_stdout(out), redirect_stderr(io.StringIO()):
        assert run.main(["grid", "long-pilot", "--provider", "openai-codex", "--model", "gpt-a"]) == 0
    assert [e["model"] for e in json.loads(out.getvalue())] == ["gpt-a"] * 6
    out = io.StringIO()
    with redirect_stdout(out):
        run.estimate(one[:1], run.PRIORS)
    assert "max_turns" in out.getvalue()  # a subscription may report $0: the cap may never bind


def test_budget_per_provider():
    kv = {"arm": "clm", "task": "kv", "level": 1000, "seed": 0, "model": run.S55}
    assert run.budget_of(kv) == round(32768 * 1.85)
    assert run.budget_of({**kv, "provider": "openai-codex", "model": "gpt-a"}) == 32768  # o200k-native
    assert run.budget_of({**kv, "task": "http-brace", "provider": "openai-codex"}) == 65536
    assert run.budget_of({**kv, "budget": 16384, "provider": "openai-codex"}) == 16384


def test_report_on_another_provider():
    row = {"arm": "clm", "task": "kv", "level": 1000, "seed": 0, "status": "completed", "llm_calls": 10}
    row.update(model="gpt-a", provider="openai-codex", drops=None, strategy={}, score=1.0, cost_usd=0.0)
    row.update(peak_prompt=3e4, ops_delivered=34, ops_total=34, input=100, cache_read=900, cache_write=0)
    base = {**row, "arm": "base", "score": 0.5}
    long = [
        {**row, "task": "http-brace", "level": 0, "arm": a, "peak_prompt": p, "input": t}
        for a, p, t in (("clm", 2e4, 1000), ("default", 3e4, 5000))
    ]
    res = report.checks([row, base, *long])
    assert all(ok for _, ok, _ in res), res
    assert ("regression kv 1000 gpt-a", True, "clm 1.00 vs base 0.50 (no spike baseline)") in res
    assert any(n == "zero thinking-drop events on clm" and "n/a" in d for n, _, d in res)
    names = {n for n, _, _ in res}
    assert names >= {"long gpt-a: clm solves >= default - 1 task", "long gpt-a: clm peak lower"}
    assert "long gpt-a: clm prompt tokens <= default" in names and "long gpt-a: clm cost <= default" not in names
    # no spike baseline is never a free pass: a clm score below base fails, and so does a group without base rows
    worse = {**row, "score": 0.0}
    assert ("regression kv 1000 gpt-a", False, "clm 0.00 vs base 0.50 (no spike baseline)") in report.checks(
        [worse, base]
    )
    assert not dict((n, ok) for n, ok, _ in report.checks([row]))["regression kv 1000 gpt-a"]
    lines = report.tables([row, *long])
    assert "\n### ContextBench-style — gpt-a\n" in lines
    assert any(x.startswith("| kv 1000 clm | 1 |") and x.endswith(" | n/a | 1.00 |") for x in lines), lines
    assert any(x.startswith("| http-brace gpt-a clm |") for x in lines)
    # the same row on Anthropic still needs its provider log
    claude = {**row, "model": run.S55, "provider": "anthropic"}
    assert [n for n, ok, _ in report.checks([claude]) if not ok] == ["zero thinking-drop events on clm"]


def test_codex_episodes_carry_no_cost_cap():
    ep = {"arm": "clm", "task": "http-brace", "level": 0, "seed": 0, "model": "gpt-a", "provider": "openai-codex"}
    assert run.cost_cap(ep) == {}
    assert run.cost_cap({**ep, "provider": "anthropic"}) == {"max_cost_usd": 10}
    assert run.cost_cap({k: v for k, v in ep.items() if k != "provider"}) == {"max_cost_usd": 10}


def test_run_refuses_the_live_engine():
    for url in ("ws://127.0.0.1:49134", "ws://localhost"):
        env = {**os.environ, "BENCH_ENGINE": url}
        p = subprocess.run([sys.executable, "-c", "import run"], cwd=run.HERE, env=env, capture_output=True, text=True)
        assert p.returncode != 0 and "49134" in p.stderr, (url, p.stderr)


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("ok", name)
