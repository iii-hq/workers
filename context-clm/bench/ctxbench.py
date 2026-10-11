"""ContextBench-style task generator (the benchmark itself is unreleased).

Task shapes (key-value store, log triage, needle retention) follow the CLM paper (arXiv 2609.37725, appendix A). Its
licence does not allow reusing its text, so every model-facing string and delimiter here is our own wording, and scores
are not comparable with the spike's run-03 (which used the paper's formats). gen(task, level, seed) returns
{"instruction", "ops": [str], "truth"}. Sizes are o200k tokens, as in the paper. `python ctxbench.py --check
kv:1000:0 ...` prints op/pressure stats (needs tiktoken).
"""

import hashlib
import json
import random
import re
import sys

VOCAB = (
    "ember crimson saffron falcon marble quartz thistle mistral cobalt cinder garnet copper meadow spruce pewter "
    "cobble glacier orbit lantern lattice"
).split()
SERVICES = ["db-proxy", "billing", "auth", "gateway", "search", "cache", "payments", "notifier"]
LEVELS = ["INFO", "WARN", "ERROR", "DEBUG"]
LIMIT, RESERVE = 32768, 2048
# Claude tokens per o200k token, measured with router::count_tokens on each family's ops (Sonnet/Opus 5.5 tokenize
# identically here). The wall counts Claude tokens, so budget = LIMIT * scale keeps the paper's o200k pressure.
CLAUDE_PER_O200K = {"kv": 1.85, "log": 1.37, "needle": 1.67}
OP_CAP, RETAIN_CAP = int(0.9 * (LIMIT - RESERVE) / 5), int(0.9 * (LIMIT - RESERVE) / 2)  # 5,530 and 13,824

PROTOCOL = (
    "Each operation arrives as a new user message in this conversation. When you have handled an operation, "
    "run the command `echo READY_FOR_NEXT_OP` and end your turn; the next operation then arrives. "
    "You have a shell tool (shell::exec) and a working directory you may use. shell::exec runs no shell: for "
    'redirection, pipes, globs or heredocs run {"command":"bash","args":["-c","<command>"]}.'
)
GRADED = "Only this conversation is graded (your replies and the command output in it); files do not count. "
END = "--- end (0 more after this one) ---\nNo operations are left. Reply DONE."


def h8(seed, s):
    return hashlib.sha256(f"{seed}:{s}".encode()).hexdigest()[:8]


def words(rng, n):
    return " ".join(rng.choice(VOCAB) for _ in range(n))


def remaining(i, n):
    return f"({n - i - 1} more after this one)"


def answer(attr, key, value):
    """One ANSWER block: the reply format the kv and log instructions ask for."""
    return f"#### ANSWER {attr}={key}\n{value}\n#### ANSWER DONE"


def answer_re(attr, key):
    """Regex for the ANSWER block of one key or qid (graders); group 1 is the answer."""
    return rf"#### ANSWER {attr}={re.escape(str(key))}\s*\n(.*?)\n\s*#### ANSWER DONE"


def kv(level, seed):
    rng = random.Random(f"kv:{level}:{seed}")
    vals = {f"K{i:05d}": f"{words(rng, 24)} #{h8(seed, f'K{i:05d}')}" for i in range(level)}
    keys = list(vals)
    batches = [keys[i : i + 100] for i in range(0, level, 100)]
    gets = [keys[i] for i in rng.sample(range(level), 24)]
    n = len(batches) + len(gets)
    ops = []
    for b, batch in enumerate(batches):
        tag = f"{b:04d}#{h8(seed, f'batch{b}')}"
        body = "\n".join(f"SET {k} = {vals[k]}" for k in batch)
        head = f"--- SET batch: keys {b * 100}-{b * 100 + len(batch) - 1} {remaining(len(ops), n)} ---"
        ops.append(
            f"{head}\n#### SET-BATCH {tag} OPEN (take this whole block out of your context on this turn)\n{body}\n"
            f"#### SET-BATCH {tag} CLOSE"
        )
    for q, k in enumerate(gets):
        ops.append(
            f"--- GET {q} {remaining(len(ops), n)} ---\nGET {k}\n"
            f"(Print the value stored under {k} in an ANSWER block for that key.)"
        )
    instruction = (
        "TASK: key-value store. First come SET batches, each with 100 lines of the form `SET <key> = <value>`; then "
        "come GET queries. Reply to each `GET <key>` with the value that key was set to, character for character "
        f"(its #hash suffix included), in this form:\n{answer('key', 'K00042', '<value>')}\n(put the queried key "
        "where K00042 is). " + GRADED + PROTOCOL
    )
    truth = {"kind": "kv", "gets": [[k, vals[k]] for k in gets], "first_query_op": len(batches)}
    return {"instruction": instruction, "ops": ops, "truth": truth}


def log(level, seed):
    rng = random.Random(f"log:{level}:{seed}")
    t = 14 * 3600
    lines, recs = [], []
    for i in range(level):
        t += rng.randint(1, 3)
        ts = f"2026-06-20T{t // 3600:02d}:{t // 60 % 60:02d}:{t % 60:02d}Z"
        lv, svc, req = rng.choice(LEVELS), rng.choice(SERVICES), f"{rng.getrandbits(32):08x}"
        lines.append(f"{ts} [{lv}] {svc} req={req} {words(rng, rng.randint(3, 5))} #{h8(seed, f'line{i}')}")
        recs.append((lv, svc, req))
    batches, i = [], 0
    while i < level:
        n = rng.randint(14, 54)
        batches.append((i, lines[i : i + n]))
        i += n
    combos = sorted({(lv, svc) for lv, svc, _ in recs})
    queries = [("count", rng.choice(combos)) for _ in range(12)] + [("lookup", rng.choice(recs)) for _ in range(12)]
    rng.shuffle(queries)
    total = len(batches) + len(queries)
    ops, truth = [], []
    for b, (start, chunk) in enumerate(batches):
        tag = f"{b:04d}#{h8(seed, f'logbatch{b}')}"
        head = f"--- log batch: lines {start}-{start + len(chunk) - 1} {remaining(len(ops), total)} ---"
        ops.append(
            f"{head}\n#### LOG-BATCH {tag} OPEN (take this whole block out of your context on this turn)\n"
            + "\n".join(chunk)
            + f"\n#### LOG-BATCH {tag} CLOSE"
        )
    for q, (kind, x) in enumerate(queries):
        if kind == "count":
            lv, svc = x
            text = f'QUERY {q}: Count the [{lv}] lines logged by service "{svc}".'
            ans = str(sum(1 for r in recs if r[0] == lv and r[1] == svc))
        else:
            text = f"QUERY {q}: Which service logged req={x[2]}?"
            ans = x[1]
        ops.append(f"--- QUERY {q} {remaining(len(ops), total)} ---\n{text}\n(Reply with an ANSWER block for qid={q}.)")
        truth.append([q, ans])
    instruction = (
        "TASK: log triage. First come batches of service log lines; then come questions about those lines. Reply to "
        f"each question in exactly this form:\n{answer('qid', 7, '<answer>')}\n(put the question's qid where 7 is; "
        "a count is a plain integer and a service is just its name). " + GRADED + PROTOCOL
    )
    truth = {"kind": "log", "answers": truth, "first_query_op": len(batches)}
    return {"instruction": instruction, "ops": ops, "truth": truth}


def needle(level, seed):
    rng = random.Random(f"needle:{level}:{seed}")
    k_max = min(8, max(2, RETAIN_CAP // (level * 30)))  # keeps all needles within the retain cap
    ops, needles = [], []
    for c in range(level):
        ns = [f"[n{c:05d}i{j:02d}#{h8(seed, f'n{c}.{j}')}] {words(rng, 10)}." for j in range(rng.randint(2, k_max))]
        needles += ns
        tag = f"{c:05d}#{h8(seed, f'fb{c}')}"
        filler = "\n".join(f"[f{c:05d}x{j:03d}#{h8(seed, f'f{c}.{j}')}] {words(rng, 12)}" for j in range(140))
        ops.append(
            f"--- chunk {c + 1}/{level} {remaining(c, level)} ---\n"
            "NEEDLES (these lines must stay in your context, unchanged):\n"
            + "\n".join(ns)
            + f"\n#### FILLER-BLOCK {tag} OPEN (remove everything from here through its CLOSE line from your context "
            f"on this turn)\n{filler}\n#### FILLER-BLOCK {tag} CLOSE"
        )
    instruction = (
        "TASK: needle retention. Every operation is a chunk with a few NEEDLES lines and one large FILLER-BLOCK. "
        "Grading happens once the run is over: each needle line has to be in your context at that point (this "
        "conversation as you then see it), unchanged; files do not count. You do not need to print anything. "
        + PROTOCOL
    )
    return {"instruction": instruction, "ops": ops, "truth": {"kind": "needle", "needles": needles}}


GEN = {"kv": kv, "log": log, "needle": needle}


def gen(task, level, seed):
    return GEN[task](int(level), int(seed))


if __name__ == "__main__" and "--check" in sys.argv:
    import tiktoken

    enc = tiktoken.get_encoding("o200k_base")
    for spec in sys.argv[2:]:
        task, level, seed = spec.split(":")
        g = gen(task, level, seed)
        sizes = [len(enc.encode(o)) for o in g["ops"]]
        total = len(enc.encode(g["instruction"])) + sum(sizes)
        extra = ""
        if task == "needle":
            extra = f" retained={len(enc.encode(chr(10).join(g['truth']['needles'])))}/{RETAIN_CAP}"
        stats = f"ops={len(sizes)} max_op={max(sizes)}/{OP_CAP} total={total} pressure={total / LIMIT:.2f}x"
        print(f"{spec}: {stats}{extra}")
        assert max(sizes) <= OP_CAP, "op over the paper's per-op cap"
elif __name__ == "__main__":
    print(json.dumps(gen(*sys.argv[1:4]))[:2000])
