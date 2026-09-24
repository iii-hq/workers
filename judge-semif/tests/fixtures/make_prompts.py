"""SemIf's own prompts for judge-semif's template test.

Run from a SemIf checkout (github.com/TheoLeeCJ/SemIf):
  uv run --no-project --with-editable . --with transformers==5.17.0 --with tokenizers==0.23.2 \
    python <this file> <Qwen3.5-4B tokenizer dir> <output json>
"""
import json, sys
from transformers import AutoTokenizer
from semif_phase1.core import direct_messages

tok = AutoTokenizer.from_pretrained(sys.argv[1])
rows = [
    {"id": "text", "state": "Customer cannot access an account after a password reset.", "question": "Which queue should handle this request?",
     "options": [{"id": "access", "description": "Account access support."}, {"id": "billing", "description": "Billing support."}]},
    {"id": "object", "state": {"from": "ops@acme.com", "body": "Café payments failed — 502 errors", "n": 3, "tags": ["urgent", None, True], "ratio": 0.5},
     "question": "Is the checkout flow impacted?", "options": [{"id": "true", "description": "Yes, the statement holds."}, {"id": "false", "description": "No, the statement does not hold."}]},
]
out = []
for row in rows:
    prompt = tok.apply_chat_template(direct_messages(row), tokenize=False, add_generation_prompt=True, enable_thinking=False)
    out.append({"state": row["state"], "question": row["question"], "options": [o["description"] for o in row["options"]],
                "prompt": prompt, "token_ids": tok.encode(prompt, add_special_tokens=False)})
json.dump(out, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
print("wrote", len(out))
