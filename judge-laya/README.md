# judge-laya

[laya](https://github.com/NandhaKishorM/laya) provider for the [`judge`](../judge/)
hub, running **inside the worker**: no API key, no Python, no external service.
laya is a typed-decision model (ModernBERT-large 421M for English,
mmBERT-base 322M for 100+ languages, Apache-2.0) that answers Noul, Choice and
Score questions over JSON state in one encoder pass per question. The
checkpoint runs in llama.cpp (`crates/llama-native`, the engine every local
judge provider shares), the encoder and laya's decision head in one graph,
on the CPU, on Metal
(macOS) or on Vulkan (Linux x86_64: AMD, NVIDIA and Intel GPUs), picked
automatically at start.

## Install

```bash
iii trigger compose::add worker=judge-laya
```

The functions register at start; the checkpoints load on demand. While laya
is the judge hub's **default provider** (`provider: laya` under **Settings →
Workers → judge**) they load at once and stay loaded. Otherwise the first call
that names `"provider": "laya"` (or comes from a session that picked it) loads
them, and they are released (VRAM included) after 10 minutes without calls; a
call that cannot wait for the load answers `deadline` while the load goes on
for the next one. The first load downloads the checkpoint
(843 MB for `laya`, 644 MB for `laya-multilingual`) from the Hugging Face Hub
into the hf-hub cache (`$HF_HOME`, default `~/.cache/huggingface`) and
converts it once to the GGUF llama.cpp loads, under `$HF_HOME/judge-laya/`
(keyed by model and revision; under a second for `laya`, 844 MB).
`laya` reads the tokenizer in its own `tokenizer/`, as laya 0.3.28 does; a
cache filled by an earlier judge-laya, which read ModernBERT-large's, lacks
it, so the first load after upgrading needs the Hub once (it downloads 3.6 MB,
or nothing when `laya-typed-decisions` is cached: they share the file).
To keep the checkpoints loaded while another provider is the default, turn
on **Keep every local provider loaded** (`preload_all`) in the judge settings.
A hub build that does not expose `judge::configuration-id` keeps the
checkpoints loaded from the start.
Air-gapped installs point `III_LAYA_CHECKPOINT_DIR` at
a directory holding `model.safetensors`, `encoder/config.json`,
`rl_agent_config.json` and `tokenizer.json` (plus an optional pre-converted
`model.gguf`; without it the checkpoint is converted into the temporary
directory); `III_LAYA_GGUF` swaps only the GGUF of the default model, and
`cargo run --example convert_gguf` converts one ahead of time.

The GGUF holds the checkpoint's own (fine-tuned) encoder and decision head
(two pre-norm blocks, the scorer and the question-type embedding) in
llama.cpp's `modern-bert` layout with a `laya` decision head, written by the
worker (`src/gguf.rs`): its tensors match b11379's `convert_hf_to_gguf.py`
byte for byte on all three checkpoints, and so do its model and `decision.*`
keys; it leaves out only that converter's names, classifier pooling, chat
template and llama.cpp tokenizer settings, and names pooling none and the
vocabulary's end of text as EOS, so llama.cpp loads it without warnings
(`tests/gguf.rs` checks the tiny checkpoint against the converter's output).
The graph scores every token for each question type; the worker reads its
question's column at the `[MASK]` markers (no pooling). Matrices stay f16:
Q8_0 moved laya's calibrated probabilities by up to 0.04 and flipped one
fixture answer at 512 tokens.

## Hardware selection

- **macOS**: Metal is linked in; Apple Silicon GPUs run the model.
- **Linux x86_64**: llama.cpp's backends are modules loaded at start from the
  binary's directory. The Vulkan module loads wherever a Vulkan loader and
  driver exist (`libvulkan.so.1`); without them it is skipped and the CPU runs
  the model. The package ships `libllama`, `libggml`, `libggml-base`, the
  CPU variants and the Vulkan module beside the binary (the same layout as
  judge-clef).
- **Linux aarch64**: CPU, statically linked.

`gpu_layers: 0` keeps the model on the CPU even when a GPU is present. The
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
`confidence` is TypeSafe's (`judge_contract::confidence`), read from the
probabilities after the checkpoint's per-bucket temperature:
`(n·p_max − 1) / (n − 1)` for a choice, 1 − the expected distance from the
likeliest level over the levels' mean distance from the middle for a score. `usage.input_tokens` counts encoder
tokens, `output_tokens` is always 0 and `usage_complete` is true.

Measured on an i9-14900K and an RX 6900 XT (`laya`, warm medians): on Vulkan
one 43-token question answers in 17 ms, four questions over a ticket in 37 ms,
one full 512-token row in 47 ms and 16 of them (one 8192-token pass) in
0.67 s, 2–5x faster than when the decision head ran in candle on the CPU. On
the CPU (8 threads) the same requests take 0.08 s, 0.47 s, 0.8 s and 15 s:
the graph runs the head for all three question types, up to 16% slower than
the candle head was. Rows that fill the window cost more; laya's
bidirectional encoder reads state and question together, so nothing is shared
between questions (no prefix reuse). Keep `timeout_ms` honest for a big state
with a hundred questions, or route such callers to a hosted provider.

## Configuration

**Settings → Workers → judge-laya**:

| Field | Default | Applied |
|---|---|---|
| `model` | `laya` | next start (`laya-multilingual` for non-English, `laya-typed-decisions` for laya's four workflows) |
| `preload` | `[]` | next start (extra checkpoints, ~1 GB of RAM each; 1.6 GB on the CPU) |
| `auto_route` | `false` | new calls |
| `auto_task_detection` | `false` | new calls |
| `revision` | `main` | next start |
| `threads` | min(8, logical cores) | next start |
| `gpu_layers` | all layers when a GPU backend and device exist | next start (`0` = CPU) |
| `batch_questions` | 16 | new calls (at most 8192 tokens per pass: 8 questions for the 1024-token checkpoints) |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls |

The form shows which checkpoint the running worker actually loaded (through
`judge-laya::models::list`, one card per loaded checkpoint with its
`context_window`) and warns when the selection needs a restart.

## Routing (laya 0.3.28 `Router`)

A request naming `model` uses that checkpoint, which must be `model` or in
`preload` (`invalid_request` otherwise). Without it, each evaluation picks its
own checkpoint: with `auto_task_detection`, question ids that exactly form one
of laya's four typed-decisions workflows (`agent_trace_observability`,
`customer_service`, `invoice_processing`, `security_incidents`) go to
`laya-typed-decisions`; with `auto_route`, the state's script and language
(laya's dependency-free detector, checked against laya's own answers in
`tests/fixtures/lang.json`) send non-English states to `laya-multilingual` and
English ones to `laya`. Latin text no word list identifies (too short, or
content words only, such as "Quero cancelar" or "ok thanks") is no evidence of
English either: it takes the default `model`, like a state with no letters
(laya #203). One non-English line or string field is enough, even
past the first 4000 characters: a Portuguese message beside a longer English
stack trace, form template or note goes to `laya-multilingual`, and so does
CJK text around Latin brand names. A state sent as a single-line string (such
as compact JSON) is judged as a whole, on its first 4000 characters. A target
that is not loaded falls back to the default. The response `model` is the checkpoint every
evaluation used, or the default when they differ; one forward never mixes
checkpoints.

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
each, header ≤192 or ≤256). A question whose options cannot all fit answers
`payload_too_large`: a choice of 127 or more options on `laya` (255 on the
1024-token checkpoints), unless it plays a tournament (below); ten score
levels always fit. Longer states are
truncated: a JSON array state (a conversation, newest last) keeps its end, any
other state its start (logged as `state truncated to the checkpoint window`;
the model then answers without the dropped part). Requests naming a `model`
that is not loaded answer `invalid_request`.

For the full API, read the hub's [reference](../judge/reference.md).

## Wide choices (laya 0.3.28 `predict_tournament`)

A row's options share the header budget, so past about 20 options every label
is cut: on `laya`, from 44 options to three tokens each and from 46 the
instructions to eight, and from 127 options the row would not fit at all. A
choice with more than 16 options can be answered like laya's
`predict_tournament` (#950) instead: set `choice_tournament: true` (off by
default, opt-in as upstream). Its options are cut, in the contract's key
order, into near-equal groups of at most 16, each asked as its own row (same
state and instructions) in the first
pass beside every other question of the request; the group winners then meet
in one final question. The answer is the final's: probabilities over the
finalists, 0 for every eliminated option, and `confidence` over the finalists,
as laya reports it, so a `confidence` threshold tuned on one-row answers does
not carry over. Upstream measured, on `laya` against one row: BANKING77 (77
labels) 0.43 → 0.61 accuracy, CLINC150 (150) 0.63 → 0.88, MASSIVE (60) 0.52
→ 0.57, with calibration error from 0.30–0.37 to 0.06–0.14.

The cost: ⌈n/16⌉ + 1 rows instead of one (every row counts in
`usage.input_tokens`) and one more pass after the first, which waits for it
(1.8–2.1x the latency upstream). The contract's 255 options take one round;
cancellation and the deadline are checked before the final's pass as before
any other. Choices of 16 options or fewer, scores and nouls are answered
exactly as before.

## Building

llama.cpp b11379 is compiled from source through `crates/llama-native` (see
judge-clef's README): `curl`, `tar`, `patch`, `cmake` and a C++17 compiler
are required, and no libclang; an offline build points
`III_LLAMA_CPP_TARBALL` at a copy of the source archive. Linux x86_64 builds
also need the Vulkan loader headers, the SPIR-V headers and `glslc` (Ubuntu:
`libvulkan-dev spirv-headers glslc`) to compile the Vulkan module. The build
copies the modules and libraries beside the binary, so `target/release` has
the published layout; the release catalog ships them as the artifact's
`companions`. Windows is not published yet.
