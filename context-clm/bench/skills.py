"""Per-arm context skills for the ContextBench-style tasks (our wording).

The skill goes right after the task instruction, placed identically for every arm. Arms: clm (mirror edits), sum
(summary at 75% of the budget), base (no management, one final turn at the wall), default (no strategy, no wall).
"""

OFFLOAD = """python3 - <<'PY'
import re, os
p = "{mirror}"; s = open(p).read(); os.makedirs("offload", exist_ok=True)
for m in reversed(list(re.finditer(r"#### {block} (\\S+) OPEN.*?#### {block} \\1 CLOSE", s, flags=re.S))):
    open("offload/{file}", "a").write(m.group(0) + "\\n")
    s = s[:m.start()] + "[{block} " + m.group(1) + " offloaded to offload/{file}]" + s[m.end():]
open(p, "w").write(s)
PY"""

DROP_FILLER = """python3 - <<'PY'
import re
p = "{mirror}"; s = open(p).read()
s = re.sub(r"#### FILLER-BLOCK (\\S+) OPEN.*?#### FILLER-BLOCK \\1 CLOSE\\n?", "", s, flags=re.S)
open(p, "w").write(s)
PY"""

SUMMARY_MECH = (
    "When your context reaches three quarters of the budget (about {at:.1f}K of {budget:,} tokens), everything "
    "except the system prompt and this task message is replaced by one summary message written from it."
)
ANS = " Print the ANSWER block in your reply; a grep result alone is not an answer."
BASE_MECH = (
    "Every operation arrives as a message in this conversation and stays in your context for the rest of the run; "
    "no command, tool or edit takes it back out."
)
CLM_MECH = (
    "Your context is mirrored to a file you can edit ({mirror}; the system prompt explains it); changing that file "
    "changes what you hold."
)
READOUT = "\n- The [context: ~N/M tokens] readout rises by the size of each {unit} and by almost nothing else."


def answer_cmd(files):
    return (
        "grep -h '^SET K00042 = ' " + files + " | head -1 | "
        "sed 's/^SET \\([^ ]*\\) = \\(.*\\)$/#### ANSWER key=\\1\\n\\2\\n#### ANSWER DONE/'"
    )


def skill(arm, task, mirror, budget):
    summary_mech = SUMMARY_MECH.format(at=0.75 * budget / 1000, budget=budget)
    if task == "kv":
        if arm == "clm":
            body = (
                f"{CLM_MECH.format(mirror=mirror)}\n- Each SET-BATCH: on the SAME turn you read it, move the whole "
                f"block out of your context and onto disk with this one command, then run echo READY_FOR_NEXT_OP:\n"
                f"{OFFLOAD.format(mirror=mirror, block='SET-BATCH', file='kv.txt')}\n"
                f"- Each GET: print the ANSWER block with this one command (with the queried key in place of "
                f"K00042):\n{answer_cmd('offload/kv.txt')}\n"
                "- Check: after an offload the [context: ~N/M tokens] readout should drop by about the size of the "
                "batch; if it did not, the batch is still in your context."
            )
        elif arm == "sum":
            body = (
                f"{summary_mech}\n- Each SET-BATCH: on the SAME turn it arrives, copy the whole block verbatim to its "
                "own file store/batch_<NNNN>.txt with one command (a quoted heredoc), check that grep -c '^SET ' on "
                "that file matches the number of SET lines in the block, then run echo READY_FOR_NEXT_OP.\n"
                "- Write the summary as a pointer, not an inventory: say that every batch so far is stored under "
                "store/ and how to look a key up.\n- Each GET: print the ANSWER block with this one command (with "
                f"the queried key in place of K00042):\n{answer_cmd('store/*.txt')}"
            )
        else:
            body = (
                f"{BASE_MECH}\n- On a SET-BATCH, run echo READY_FOR_NEXT_OP and nothing else. Do not copy batches "
                "to disk.\n- On a GET, find the `SET <key> =` line in the batch in your context and print its value "
                f"verbatim in an ANSWER block.{ANS}" + (READOUT.format(unit="batch") if arm == "base" else "")
            )
    elif task == "log":
        q = "count with grep -c '\\[LEVEL\\] service ' on the stored lines; find a request with grep 'req=<id>'"
        if arm == "clm":
            body = (
                f"{CLM_MECH.format(mirror=mirror)}\n- Each LOG-BATCH: on the SAME turn you read it, move the whole "
                f"block out of your context and onto disk with this one command, then run echo READY_FOR_NEXT_OP:\n"
                f"{OFFLOAD.format(mirror=mirror, block='LOG-BATCH', file='log.txt')}\n"
                f"- Each QUERY: answer from offload/log.txt ({q}), then print the ANSWER block.{ANS}\n"
                "- Check: after an offload the [context: ~N/M tokens] readout should drop by about the size of the "
                "batch."
            )
        elif arm == "sum":
            body = (
                f"{summary_mech}\n- Each LOG-BATCH: on the SAME turn it arrives, copy the whole block verbatim to its "
                "own file store/log_<NNNN>.txt with one command (a quoted heredoc), check the line count, then run "
                "echo READY_FOR_NEXT_OP.\n- Write the summary as a pointer, not an inventory: say that every batch "
                f"so far is stored under store/.\n- Each QUERY: answer from store/*.txt ({q}), then print the ANSWER "
                f"block.{ANS}"
            )
        else:
            body = (
                f"{BASE_MECH}\n- On a LOG-BATCH, run echo READY_FOR_NEXT_OP and nothing else. Do not copy batches "
                "to disk.\n- On a QUERY, answer from the log lines in your context and print the ANSWER block."
                f"{ANS}" + (READOUT.format(unit="batch") if arm == "base" else "")
            )
    else:  # needle
        if arm == "clm":
            body = (
                f"{CLM_MECH.format(mirror=mirror)}\n- Each chunk: on the SAME turn it arrives, delete its "
                "FILLER-BLOCK (OPEN line through CLOSE line) from your context with this one command, keeping the "
                f"NEEDLES lines verbatim, then run echo READY_FOR_NEXT_OP:\n{DROP_FILLER.format(mirror=mirror)}\n"
                "- Check: the [context: ~N/M tokens] readout should drop by about the size of the filler."
            )
        elif arm == "sum":
            body = (
                f"{summary_mech}\n- The summary must reproduce every needle line seen so far verbatim, one per line; "
                "filler can be dropped.\n- Each chunk: run echo READY_FOR_NEXT_OP."
            )
        else:
            body = f"{BASE_MECH}\n- Each chunk: run echo READY_FOR_NEXT_OP and nothing else."
    return "CONTEXT SKILL (how to manage your context in this task):\n" + body
