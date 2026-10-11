"""Offline checks for the bench (no stack, no API calls). Run: python test_bench.py"""

import re
import subprocess
import tempfile
from pathlib import Path

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


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("ok", name)
