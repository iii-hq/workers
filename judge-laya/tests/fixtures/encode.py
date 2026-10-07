"""Regenerate encode.json from laya's own `build_sequence` and the real tokenizer.

    git -C <laya checkout> show v0.3.28:laya/common.py > /tmp/common.py
    uv run --with 'tokenizers==0.22.*' python tests/fixtures/encode.py /tmp/common.py

Only the sequence-building half of common.py runs: it is executed up to
`parallel_layout`, without its torch/numpy imports, so no model stack is needed.
A thin adapter gives the `tokenizers` tokenizer the slice of the transformers
interface those functions call. Each case calls `build_sequence` on its own, as
laya did before it shared one tokenized state between questions (#109), with
laya's default `truncate_left = isinstance(state, list)` (agent.py
`_encode_state`).
"""
import json
import re
import sys
from pathlib import Path

from tokenizers import Tokenizer

HERE = Path(__file__).resolve().parent


def load_common(path):
    source = Path(path).read_text()
    source = source[: source.index("\ndef parallel_layout")]
    source = re.sub(r"^(import (torch|numpy)|from torch).*$", "", source, flags=re.M)
    namespace = {"__name__": "laya_common"}
    exec(compile(source, path, "exec"), namespace)
    return namespace


class Tok:
    def __init__(self, path):
        self.tok = Tokenizer.from_file(str(path))
        self.mask_token = "[MASK]"
        self.mask_token_id = self.tok.token_to_id("[MASK]")
        self.cls_token_id = self.tok.token_to_id("[CLS]")
        self.sep_token_id = self.tok.token_to_id("[SEP]")

    def __call__(self, text, add_special_tokens=True, truncation=False, max_length=None):
        ids = self.tok.encode(text, add_special_tokens=add_special_tokens).ids
        return {"input_ids": ids[:max_length] if truncation else ids}


TICKET = {
    "from": "user@acme.com",
    "subject": "Duplicate charge on invoice #4411",
    "nested": {"tags": ["billing", "urgent"], "count": 2, "flag": True, "none": None},
    "body": "Refund the duplicate today or we cancel. Also [MASK] appears here.",
}
LONG = "a fairly long description of this option, repeated to overflow the per-option budget " * 3
# A chronological conversation, newest last, far past the 512-token window.
CONVERSATION = [
    {
        "role": "user" if i % 2 == 0 else "assistant",
        "content": "Turn %d: still waiting on the delivery status of my order." % i,
    }
    for i in range(60)
] + [{"role": "user", "content": "Forget the delivery, please refund order #4411 today."}]
SHORT_CONVERSATION = [
    {"role": "user", "content": "My invoice was charged twice."},
    {"role": "assistant", "content": "Sorry about that, checking now."},
    {"role": "user", "content": "Please refund the duplicate."},
]
REFUND = "Does the user ask for a refund in the latest message?"
WANTS = {"refund": "money back", "delivery": "shipping status", "other": None}

# (name, state, type, instructions, criteria)
CASES = [
    ("choice_strings", TICKET, "choice", "Which department should handle this request?",
     {"billing": "invoices, payments, refunds", "technical": "bugs, outages, system errors",
      "sales": "pricing, new contracts", "other": None}),
    ("choice_structured", TICKET, "choice", "Pick",
     {"a": {"scope": "new contracts", "n": 1}, "b": ["x", "y"], "c": ""}),
    ("score", TICKET, "score", "How urgent is this request?",
     ["not urgent", {"impact": "critical deadline or blocking issue"}, "critical"]),
    ("noul_defaults", "Plain text state", "noul", "Does the user threaten to cancel or leave?", None),
    ("noul_criteria", ["a", {"b": 1}], "noul", "Refund requested? [MASK]",
     {"true": "an explicit refund request"}),
    ("long_options", "short", "choice", "x", {"opt%d" % i: LONG for i in range(12)}),
    ("long_state", "word " * 2000, "score", "Length?", ["short", "long"]),
    ("empty_instructions", TICKET, "noul", "", None),
    # An array state keeps its end (the newest turn); two questions on one
    # state get different rooms from their different heads.
    ("conversation_overflow", CONVERSATION, "noul", REFUND, None),
    ("conversation_overflow_choice", CONVERSATION, "choice", "What does the user want now?", WANTS),
    # Any other state keeps its start, even one holding a conversation.
    ("object_overflow", {"subject": "Order #4411", "history": CONVERSATION}, "noul", REFUND, None),
    ("conversation_short", SHORT_CONVERSATION, "score", "How upset is the user?",
     ["calm", "annoyed", "angry"]),
]


def main(common_path):
    common = load_common(common_path)
    tok = Tok(HERE / "tiny" / "tokenizer.json")
    out = []
    for name, state, qtype, instructions, criteria in CASES:
        q = {"t": qtype, "ins": instructions, "crit": criteria}
        ids, markers, stats = common["build_sequence"](
            tok, state, q, 512, 192,
            truncate_left=isinstance(state, list), return_truncation_stats=True,
        )
        out.append({
            "name": name, "state": state, "type": qtype, "instructions": instructions,
            "criteria": criteria, "options": common["render_options"](q), "ids": ids,
            "markers": markers, "state_dropped": stats["state_tokens_dropped"],
        })
    (HERE / "encode.json").write_text(json.dumps(out, indent=0, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(sys.argv[1])
