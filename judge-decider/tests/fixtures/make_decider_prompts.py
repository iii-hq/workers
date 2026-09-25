"""decider's own rows and probabilities for judge-decider's decider tests.

Run from a checkout of github.com/Mapika/decider (decider-ai 1.5.0, a5120cc) with the weights of Mapika/decider-4b at tag v2:
  uv run --no-project --with torch --with transformers==5.17.0 --with safetensors \
    env PYTHONPATH=<checkout> python <this file> <Mapika/decider-4b v2 snapshot dir> <output json>

Each question is planned and tokenized by decider's systemone path (independent rows, isolated Score levels) and
scored by the bf16 weights computed in float32: the logit at each row's answer slot over its label tokens, then a
softmax at decider_config.json's temperature. Choice criteria are listed in sorted key order, as the judge
contract's criteria maps are.
"""
import json, sys
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer
from decider import systemone as S1
from decider.prompt import label_table
from decider.prompt_fast import build_rows

path, out_path = sys.argv[1], sys.argv[2]
cfg = json.load(open(f"{path}/decider_config.json"))
assert cfg["layout"] == "plain" and cfg["isolated_levels"] and not cfg["schema_first"], cfg
T = cfg["temperature"]
tok = AutoTokenizer.from_pretrained(path)
model = AutoModelForCausalLM.from_pretrained(path, dtype=torch.float32).eval()
_, labels, _ = label_table(tok)

ticket = {"subject": "Duplicate charge on invoice #4411", "body": "We were billed twice for March. Refund the duplicate today or we cancel."}
cases = [
    {"id": "ticket", "state": ticket, "questions": {
        "department": {"type": "choice", "instructions": "Which department should handle this request?",
                       "criteria": {"billing": "invoices, payments, refunds", "sales": None, "technical": "bugs, outages"}},
        "urgency": {"type": "score", "instructions": "How urgent is this request?", "criteria": ["not urgent", "soon", "critical deadline or blocking issue"]},
        "churn": {"type": "noul", "instructions": "Does the user threaten to cancel or leave?"},
        "refund": {"type": "noul", "instructions": "Does the user ask for a refund?", "criteria": {"true": "a refund is requested", "false": {"note": "no refund"}}},
        "cancel": {"type": "noul", "criteria": {"true": "the user threatens to cancel the plan"}}}},
    {"id": "text", "state": "Café payments failed — 502 errors since 9am, checkout is blocked.", "questions": {
        "severity": {"type": "score", "instructions": {"ask": "How severe is the incident?"},
                     "criteria": ["0: cosmetic", " 1 : degraded", {"impact": "blocking"}, "-3:total outage"]}}},
    {"id": "wide", "state": {"events": [{"kind": "login", "ok": i % 3 != 0} for i in range(9)], "tags": ["a", "b"], "n": 0.5},
     "questions": {
        "module": {"type": "choice", "instructions": "Which area do the events belong to?",
                   "criteria": dict(sorted(({f"area{i}": (None if i % 4 == 0 else f"area number {i}") for i in range(11)}
                                            | {"auth": {"covers": ["login", "tokens"]}}).items()))}}},
]
out = []
for case in cases:
    ctx = S1.render_state(case["state"])
    rqs = {k: S1.render_question(v) for k, v in case["questions"].items()}
    flat, index = S1.plan_rows(rqs, True)
    items, _ = build_rows(tok, ctx, [[(r["question"], list(r["options"]))] for r in flat])
    rows, probs = [], []
    for r, it in zip(flat, items):
        with torch.no_grad():
            z = model(torch.tensor([it["ids"]])).logits[0, it["slots"][0]]
        lab = z[labels[:len(r["options"])]].double()
        p = torch.softmax(lab / T, -1).tolist()
        probs.append(p)
        rows.append({"question": r["question"], "options": r["options"], "token_ids": it["ids"], "probabilities": p})
    out.append({"id": case["id"], "state": case["state"], "questions": case["questions"], "context": "Context:\n" + ctx,
                "rows": rows, "answers": S1.assemble(rqs, index, probs)})
    print(case["id"], len(rows), "rows", file=sys.stderr)
json.dump({"model": "Mapika/decider-4b@v2", "temperature": T, "labels": labels, "cases": out},
          open(out_path, "w"), ensure_ascii=False, indent=1)
print("wrote", out_path, file=sys.stderr)
