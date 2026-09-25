# judge-laya

[laya](https://github.com/NandhaKishorM/laya) provider for the [`judge`](../judge/)
hub, running **inside the worker**: no API key, no Python, no external service.
laya is a typed-decision model (ModernBERT-large 421M for English,
mmBERT-base 322M for 100+ languages, Apache-2.0) that answers Noul, Choice and
Score questions over JSON state in one encoder pass per question. The
encoder runs through llama.cpp (the runtime shared with judge-semif) on the
CPU, on Metal (macOS) or on Vulkan (Linux x86_64: AMD, NVIDIA and Intel GPUs),
picked automatically at start; laya's decision head runs in candle on the CPU.

## Install

```bash
iii trigger compose::add worker=judge-laya
```

The checkpoints load only while laya is the judge hub's **default provider**
(`provider: laya` under **Settings → Workers → judge**): the worker follows
the hub's configuration and, when another provider becomes the default,
unregisters its functions and releases the checkpoints (VRAM included); they
load again when laya is selected. The first load downloads the checkpoint
(843 MB for `laya`, 644 MB for `laya-multilingual`) and its encoder as an f16
GGUF (791 MB for `laya`, from `iii-dev/laya-encoder-gguf`) from the Hugging
Face Hub into the hf-hub cache (`$HF_HOME`, default `~/.cache/huggingface`).
The functions register only once the model answers; until then, and while
another provider is the default, the hub reports `provider_unavailable`, also
for calls that name `"provider": "laya"`. To keep the checkpoints loaded while another
provider is the default, turn on **Keep every local provider loaded**
(`preload_all`) in the judge settings. A hub build that does not expose
`judge::configuration-id` leaves the checkpoints loaded from the start.
Air-gapped installs point `III_LAYA_CHECKPOINT_DIR` at
a directory holding `model.safetensors`, `encoder/config.json`,
`rl_agent_config.json`, `tokenizer.json` and `encoder.gguf`;
`III_LAYA_ENCODER_GGUF` swaps only the GGUF of the default model.

The GGUF holds the checkpoint's own (fine-tuned) `encoder.*` tensors, repacked
by `scripts/convert-encoder.py` and converted with llama.cpp's
`convert_hf_to_gguf.py`. Only f16 keeps laya's calibrated probabilities: Q8_0
moved them by up to 0.04 and flipped one fixture answer at 512 tokens.

## Hardware selection

- **macOS**: Metal is linked in; Apple Silicon GPUs run the encoder.
- **Linux x86_64**: llama.cpp's backends are modules loaded at start from the
  binary's directory. The Vulkan module loads wherever a Vulkan loader and
  driver exist (`libvulkan.so.1`); without them it is skipped and the CPU runs
  the encoder. The package ships `libllama`, `libggml`, `libggml-base`, the
  CPU variants and the Vulkan module beside the binary (the same layout as
  judge-semif).
- **Linux aarch64**: CPU, statically linked.

`gpu_layers: 0` keeps the encoder on the CPU even when a GPU is present. The
chosen device is logged at start as `selected inference device` and shown in
the settings form.

Make it the default under **Settings → Workers → judge**, then call the hub:

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

Measured on an i9-14900K and an RX 6900 XT (8 threads, 100–180-token rows):
one question answers in about 0.12 s on the CPU and 0.04–0.05 s on Vulkan,
against 0.22–0.32 s with the previous candle encoder. Rows that fill the
512-token window cost more; laya's bidirectional encoder reads state and
question together, so nothing is shared between questions (no prefix reuse).
Keep `timeout_ms` honest for a big state with a hundred questions, or route
such callers to a hosted provider. `RAYON_NUM_THREADS` in the worker
environment overrides the `threads` setting for the head.

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
| `threads` | min(8, logical cores) | next start (`RAYON_NUM_THREADS` in the environment wins for the head) |
| `gpu_layers` | all layers when a GPU backend and device exist | next start (`0` = CPU) |
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
on that spelling. Choice options are encoded in the contract's key order, and
JSON object keys (state, criteria) arrive **sorted** through the bus, so the
same logical request can tokenize differently than in Python laya, whose dicts
keep insertion order. laya is order-sensitive (one ticket flipped from
`billing` 0.57 to `sales` 0.69 when its four options were reordered), so
answers here are deterministic but not byte-identical to Python's; the port
itself matches Python's logits to 1e-4 on identical token sequences for the
English and multilingual checkpoints.
Sequences are capped at the checkpoint's window (512 tokens for `laya`, 1024
for `laya-multilingual` and `laya-typed-decisions`; option text ≤48 tokens
each, header ≤192 or ≤256): a request
whose options cannot all fit answers `payload_too_large`, longer states are
truncated on the right (logged as `state truncated to the checkpoint window`;
the model then answers without the dropped part). Requests naming a `model`
that is not loaded answer `invalid_request`.

For the full API, read the hub's [reference](../judge/reference.md).

## Building

llama.cpp is compiled from source through `crates/llama-runtime`: `cmake`, a
C++ compiler and `libclang` (for bindgen) are required; if libclang lives
outside the default search path set `LIBCLANG_PATH` (and
`BINDGEN_EXTRA_CLANG_ARGS=-I<clang>/include` when its builtin headers are not
found). Linux x86_64 builds also need the Vulkan loader headers, the SPIR-V
headers and `glslc` (Ubuntu: `libvulkan-dev spirv-headers glslc`) to compile
the Vulkan module. The build copies the modules and libraries beside the
binary, so `target/release` has the published layout; the release catalog
ships them as the artifact's `companions`. Windows is not published yet.
