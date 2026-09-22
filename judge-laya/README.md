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
per-question cost (0.26 s at 32 per batch). Rows that fill the 512-token window
cost 1.4–1.8 s each, so a request with a big state and a hundred questions
takes minutes: keep `timeout_ms` honest or route such callers to a hosted
provider. `RAYON_NUM_THREADS` in the worker environment overrides the
`threads` setting.

On macOS the build enables candle's `metal` and `accelerate` features: the
worker picks the Metal GPU when one is present (logged as
`selected inference device`) and falls back to the Accelerate-backed CPU path.
Not yet measured on Apple hardware.

## Configuration

**Settings → Workers → judge-laya**:

| Field | Default | Applied |
|---|---|---|
| `model` | `laya` | next start (`laya-multilingual` for non-English, `laya-typed-decisions` for laya's four workflows) |
| `preload` | `[]` | next start (extra checkpoints, ~1.7 GB of RAM each) |
| `auto_route` | `false` | new calls |
| `auto_task_detection` | `false` | new calls |
| `shortlist_k` | off | new calls |
| `revision` | `main` | next start |
| `threads` | min(8, logical cores) | next start (`RAYON_NUM_THREADS` in the environment wins) |
| `batch_questions` | 16 | new calls |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls |

The form shows which checkpoint the running worker actually loaded (through
`judge-laya::models::list`, one card per loaded checkpoint with its
`context_window`) and warns when the selection needs a restart.

## Routing and shortlist (laya 0.3.5 `Router` and `predict_shortlist`)

A request naming `model` uses that checkpoint, which must be `model` or in
`preload` (`invalid_request` otherwise). Without it, each evaluation picks its
own checkpoint: with `auto_task_detection`, question ids that exactly form one
of laya's four typed-decisions workflows (`agent_trace_observability`,
`customer_service`, `invoice_processing`, `security_incidents`) go to
`laya-typed-decisions`; with `auto_route`, the state's script and language
(laya's dependency-free detector, ported with its fixtures) send non-English
states to `laya-multilingual` and English ones to `laya`. A target that is not
loaded falls back to the default. The response `model` is the checkpoint every
evaluation used, or the default when they differ; one forward never mixes
checkpoints.

`shortlist_k` enables laya's opt-in embedding shortlist: a choice question with
more options than `k` embeds `instructions + state` and every rendered option
with the same encoder (mean-pooled states, cosine), keeps the `k` closest
labels for the decision head and reports the others with probability 0. Kept
options stay in the contract's key order, where laya reorders them by rank.

## Encoding notes

State and structured criteria are serialized exactly as laya's Python
`json.dumps` (`", "` and `": "` separators), because the checkpoint was trained
on that spelling. Choice options are encoded in the contract's key order.
Sequences are capped at the checkpoint's window (512 tokens for `laya`, 1024
for `laya-multilingual` and `laya-typed-decisions`; option text ≤48 tokens
each, header ≤192 or ≤256): a request
whose options cannot all fit answers `payload_too_large`, longer states are
truncated on the right (logged as `state truncated to the checkpoint window`;
the model then answers without the dropped part). Requests naming a `model`
that is not loaded answer `invalid_request`.

For the full API, read the hub's [reference](../judge/reference.md).
