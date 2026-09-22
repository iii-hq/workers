# judge-laya

[laya](https://github.com/NandhaKishorM/laya) provider for the [`judge`](../judge/)
hub, running **inside the worker**: no API key, no Python, no external service.
laya is a typed-decision model (ModernBERT-large 421M for English,
mmBERT-base 322M for 100+ languages, Apache-2.0) that answers Noul, Choice and
Score questions over JSON state in one encoder pass per question.

## Install

```bash
iii trigger compose::add worker=judge-laya
```

On the first start the worker downloads the checkpoint (843 MB for `laya`,
644 MB for `laya-multilingual`) from the Hugging Face Hub into the hf-hub cache
(`$HF_HOME`, default `~/.cache/huggingface`) and loads it (about 1.7 GB of RAM
for `laya`). Its functions register only once the model answers; until then the
hub reports `provider_unavailable`. Air-gapped installs point
`III_LAYA_CHECKPOINT_DIR` at a directory holding `model.safetensors`,
`encoder/config.json`, `rl_agent_config.json` and `tokenizer.json`.

Route the hub to it per call with `"provider": "laya"`, or make it the default
under **Settings → Workers → judge**:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
  "timeout_ms": 60000, "provider": "laya",
  "evaluations": [{
    "id": "ticket-4411",
    "state": {"subject": "Duplicate charge on invoice #4411",
              "body": "We were billed twice for March. Refund the duplicate today or we cancel."},
    "questions": {
      "department": {"type": "choice", "criteria": {"billing": "invoices, payments, refunds", "technical": "bugs, outages"}},
      "urgency": {"type": "score", "criteria": ["not urgent", "soon", "critical deadline or blocking issue"]},
      "churn_risk": {"type": "noul", "instructions": "Does the user threaten to cancel or leave?"}
    }
  }]
}'
```

Answers follow the shared contract exactly: `noul` is P(true); `choice` carries
the option probabilities and `confidence`; `score` carries the level
distribution, the probability-weighted `score`, `confidence` and the `legend`.
`confidence` is laya's own calibrated readout (1 − normalized entropy after the
checkpoint's per-bucket temperature). `usage.input_tokens` counts encoder
tokens, `output_tokens` is always 0 and `usage_complete` is true.

Measured on an i9-14900K (CPU only, 140-token rows): one question answers in
about 0.3 s with 6–8 threads and 0.8 s when all 32 logical cores are used, so
the default caps threads at 8; batching questions barely changes the
per-question cost (0.26 s at 32 per batch). `RAYON_NUM_THREADS` in the worker
environment overrides the `threads` setting.

## Configuration

**Settings → Workers → judge-laya**:

| Field | Default | Applied |
|---|---|---|
| `model` | `laya` | next start (`laya-multilingual` for non-English) |
| `revision` | `main` | next start |
| `threads` | min(8, logical cores) | next start (`RAYON_NUM_THREADS` in the environment wins) |
| `batch_questions` | 16 | new calls |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls |

The form shows which checkpoint the running worker actually loaded (through
`judge-laya::models::list`) and warns when the selection needs a restart.

## Encoding notes

State and structured criteria are serialized exactly as laya's Python
`json.dumps` (`", "` and `": "` separators), because the checkpoint was trained
on that spelling. Choice options are encoded in the contract's key order.
Sequences are capped at the checkpoint's window (512 tokens for `laya`, 1024
for `laya-multilingual`; option text ≤48 tokens each, header ≤192): a request
whose options cannot all fit answers `payload_too_large`, longer states are
truncated on the right. Requests naming another `model` answer `invalid_request`.

For the full API, read the hub's [reference](../judge/reference.md).
