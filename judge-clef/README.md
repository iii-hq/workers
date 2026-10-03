# judge-clef

[Clef-Flash](https://huggingface.co/Cloudflare/clef-flash) provider for the
[`judge`](../judge/) hub, running **inside the worker**: no API key, no Python,
no llama-server. Clef-Flash is Cloudflare's decision model
([announcement](https://blog.cloudflare.com/clef-decision-models)): a
Qwen3.5-9B backbone and a joint schema head of about 122M parameters. The head
reads the backbone's final hidden state of every prompt token and gives one
logit per option of every question, so all the questions of an evaluation are
decided together in one forward pass, with no decoding. This worker runs the
backbone GGUF through llama.cpp (the
[`llama-cpp-2`](https://crates.io/crates/llama-cpp-2) crate, in the runtime
it shares with judge-decider, judge-semif and judge-laya) on the CPU, on
Metal (macOS) or on Vulkan (Linux x86_64: AMD, NVIDIA and Intel GPUs), picked
automatically at start; the joint schema head runs in candle on the CPU.

## Hardware selection

- **macOS**: Metal is linked in; Apple Silicon GPUs run the backbone.
- **Linux x86_64**: llama.cpp's backends are modules loaded at start from the
  binary's directory (`GGML_BACKEND_DL`). The Vulkan module loads wherever a
  Vulkan loader and driver exist (`libvulkan.so.1`); without them it is
  skipped and the CPU runs the backbone, so the same package works on GPU
  desktops, servers and containers. The CPU module is picked for the host
  (AVX2, AVX-512, AMX variants). The package ships `libllama`, `libggml`,
  `libggml-base`, the CPU variants and the Vulkan module beside the binary,
  which finds them through its `$ORIGIN` runpath.
- **Linux aarch64**: CPU, statically linked.

`gpu_layers: 0` keeps the backbone on the CPU even when a GPU is present. The
joint schema head always runs on the CPU, on `threads` threads
(`RAYON_NUM_THREADS` in the worker environment wins). The chosen device is
logged when the model loads, as `selected inference device`, and shown in the
settings form.

## Install

```bash
iii trigger compose::add worker=judge-clef
```

The functions register at start; the model loads on demand. While clef is the
judge hub's **default provider** (`provider: clef` under **Settings → Workers →
judge**) it loads at once and stays loaded. Otherwise the first call that names
`"provider": "clef"` (or comes from a session that picked it) loads it, and it
is released (VRAM included, about 6 GiB) after 10 minutes without calls. A call
that cannot wait for the load answers `deadline` while the load goes on for the
next one.
The first load downloads the pinned checkpoint into the hf-hub cache
(`$HF_HOME`, default `~/.cache/huggingface`): first the joint schema head and
the tokenizer from `Cloudflare/clef-flash`, then the backbone GGUF from
`bartowski/Cloudflare_clef-flash-GGUF` (a llama.cpp `qwen35` conversion that
keeps the untied `output.weight` the head reads). To keep it loaded while
another provider is the default, turn on **Keep every local provider loaded**
(`preload_all`) in the judge settings. A hub build that does not expose
`judge::configuration-id` keeps the model loaded from the start.

Air-gapped installs point `III_CLEF_CHECKPOINT_DIR` (or `--checkpoint-dir`) at
a directory holding one checkpoint, loaded as the configured `model`:

| File | Content |
|---|---|
| `backbone.gguf` | the Qwen3.5 backbone GGUF (arch `qwen35`, with `output.weight`) |
| `joint_head.safetensors` | the joint schema head, from `Cloudflare/clef-flash` |
| `joint_head_config.json` | its sizes, from `Cloudflare/clef-flash` |
| `tokenizer.json` | the tokenizer, from `Cloudflare/clef-flash` |

The files may be symlinks, for example into an hf-hub snapshot. Use a K-quant
or legacy-quant backbone (`Q4_K_M`, `Q8_0`, …): the reader of `output.weight`
(candle) rejects the IQ quantizations. The tokenizer is the Hugging Face
`tokenizer.json`, not the GGUF's vocabulary: llama.cpp's `qwen35`
pre-tokenizer skips the NFC normalization the model was trained with, which
changes the tokens of Hindi, Thai and decomposed text.

Make it the hub's default, then call the hub:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
  "timeout_ms": 60000, "provider": "clef",
  "evaluations": [{
    "id": "ticket-4411",
    "state": {"subject": "Duplicate charge on invoice #4411",
              "body": "We were billed twice for March. Refund the duplicate today or we cancel."},
    "questions": {
      "department": {"type": "choice", "instructions": "Which team should handle it?", "criteria": {"billing": "invoices, payments, refunds", "technical_support": "bugs, outages"}},
      "urgency": {"type": "score", "instructions": "How urgent is it?", "criteria": ["not urgent", "soon", "critical deadline or blocking issue"]},
      "churn_risk": {"type": "noul", "instructions": "Does the user threaten to cancel or leave?"}
    }
  }]
}'
```

## How questions map to Clef

The worker builds the reference prompt (`encode_record` in
`joint_schema_model.py`, published with the model; llama.cpp's Clef support
uses the same layout), one prompt per evaluation with every question in it:

```
<|im_start|>system
Read the complete state and schema. Decide every field jointly. Each answer must be exactly one of that field's allowed options.<|im_end|>
<|im_start|>user
STATE:
<state>

SCHEMA FIELDS:

FIELD 1
ID: <question id>
TYPE: <noul | choice | score>
INSTRUCTION: <instructions>
ALLOWED OPTIONS:
OPTION 1: {"description":"<description>","option_id":"<option id>"}
OPTION 2: …
END FIELD

FIELD 2
…
END FIELD

<|im_end|>
<|im_start|>assistant
<think>

</think>

JOINT SCHEMA DECISIONS:
```

The state is a string as-is, or compact JSON with every object's keys sorted,
as Python's `json.dumps(value, sort_keys=True, separators=(",", ":"),
ensure_ascii=False)` writes it; instructions and
descriptions that are objects or arrays render the same way. The key order of a
JSON state therefore does not matter to Clef. Every piece (the header, the
state, each field line, each option) is tokenized on its own, as the model was
trained. Questions appear in id order (the contract's `questions` map is
sorted). A question whose `instructions` are null or empty is asked by its id,
as the reference does.

| Contract | Clef options, in prompt order | Answer |
|---|---|---|
| `noul` | `true`, then `false`; the `criteria` descriptions replace the defaults ("The proposition is true or the answer is yes." / "… false or the answer is no.") | `noul` = P(true) |
| `choice` | one per criterion, sorted by option id; a null description is left out | argmax, probabilities, confidence |
| `score` | levels `"0"` to `"n-1"` in order, the level text as the description | the levels' probabilities, Σ i·p_i, confidence, legend |

The joint schema head reads the hidden states of each question's instruction
span and of each option's JSON span, plus the last token's state as a summary
of the whole prompt. The questions attend to each other, and each option gets
a lexical prior from the backbone's output embeddings (`output.weight`) of its
own tokens. A softmax over each question's logits gives its probabilities, at
temperature 1 as in the reference. `confidence` is TypeSafe's
(`judge_contract::confidence`), as for every local provider:
`(n·p_max − 1) / (n − 1)` for a choice, 1 − the expected distance from the
likeliest level over the levels' mean distance from the middle for a score.
The reference's `systemone()` reports p_max instead, so a threshold tuned on
the reference does not carry over unchanged. `usage.input_tokens` counts the
prompt tokens and `output_tokens` is always 0.

Only the state is ever cut. When the prompt exceeds `context_tokens`, the state
keeps its first tokens and loses the rest (cut between tokens, possibly inside
a JSON value; logged as `state truncated to the context window`); a schema
that alone exceeds the window answers `payload_too_large`. A question with an
empty id and no instructions answers `invalid_request`: it would give the head
an empty span to read. `judge-clef::models::list` reports the window as
`context_window` and 255 as `max_options`.

`tests/clef.rs` checks the token counts and probabilities of 10 records (28
questions) against Cloudflare's own pipeline, `joint_schema_model.py` with the
bf16 backbone and head (`tests/fixtures/make_reference.py`). The shipped
Q4_K_M differs by about 0.008 on average and at most 0.088 on the CPU, 0.15 on
Vulkan (deploy_log/rollback_needed). It picks the reference's top option on
27 of 28 questions on the CPU and all 28 on Vulkan; the CPU miss is
deploy_log/severity, whose reference top two are 0.018 apart. The test runs
on the CPU, where its tolerance was measured. Q8_0 is about 3x closer (mean
0.003, max 0.035) but a 9.5 GB file.

## Writing requests for Clef

Cloudflare publishes no prompting guide for Clef; it claims compatibility with
TypeSafe's Jev API, so TypeSafe's guidance applies, with these Clef specifics:

- **Name questions and options with words.** The model reads every question id
  and option id (Jev never sees the question key), and the option id's tokens
  also feed the head's lexical prior. Use descriptive snake_case ids such as
  `technical_support` or `churn_risk`, not `q1` or `opt_b`.
- **Always send instructions.** Ask one snap judgment per question and combine
  multi-factor decisions in code. Phrase a noul so that a high value means yes,
  state the exact condition, and avoid negations and double conditions. Point
  at state fields with backticked paths such as `` `ticket.messages[0].text` ``.
- **Put the decisive state first.** Prefer a JSON object with descriptive
  field names and keep only the context the questions need: a long state loses
  its end.
- **Ask related questions together, and only those.** Every question sees the
  others, so an extra speculative question can move the other answers;
  unrelated decisions belong in separate evaluations.
- **Choice:** list every option and add an `other` or `none_of_the_above`
  option. Cloudflare's results show Clef-Flash strong on tool and function
  selection (BFCL 98.8) and intent sets of up to about 80 options (BANKING77,
  77 intents: 90.9), and weak on very large option sets (CLINC150+OOS, 151
  options: 66.8) and on hallucination and faithfulness checks (RAGTruth 35.6).
- **Score:** describe situations, not degrees; one dimension per question; give
  rare extremes their own level.
- **Keep arithmetic out of the model.** Counting, dates and numeric
  comparisons belong in code.
- **Tune thresholds on your own data.** Cloudflare publishes no calibration
  figures for Clef.

## Speed and memory (Q4_K_M, i9-14900K, RX 6900 XT)

An evaluation is one forward pass over one prompt, so its time follows the
prompt's length, not its number of questions; the evaluations of one request
run one after another. Per evaluation, backbone and joint schema head
together:

| Prompt | 400 tokens | 2048 tokens | 8192 tokens | 16384 tokens |
|---|---|---|---|---|
| Vulkan | 0.4 s | 1.9 s | 7.5 s | 15.7 s |

The head is 9–16% of that. The CPU (8 threads) reads about 40 prompt tokens
per second: 10.4 s for 416 tokens, 51 s for 2048, 449 s for 16384. A prompt
over about 10k tokens therefore cannot finish on the CPU within the default
`max_timeout_ms` of 300000 (5 minutes): use a GPU, or raise `max_timeout_ms`
and the request's `timeout_ms`. From a warm page cache the model loads in
2.3 s on Vulkan and 1.2 s on the CPU.

On Vulkan the worker holds 6.0 GiB of VRAM at the default `context_tokens` of
16384 (backbone weights 4.9 GiB, KV cache 0.5 GiB, compute buffer 0.6 GiB;
6.1 GiB at peak) and 9.2 GiB at 65536, where the compute buffer grows to
2.2 GiB. The joint schema head stays in host memory.

Flash attention is off. The worker prefills a prompt in 512-token chunks, and
llama.cpp's CPU flash-attention kernel changes below 64 queries, so a short
last chunk drifted from a one-pass prefill. Without it chunked prefill is
bit-exact, Vulkan prefill is 13% faster at 16384 tokens and the gap to the
reference is unchanged on the CPU and lower on Vulkan (0.15 against 0.16). The
cost is VRAM: 79 MiB more at 16384 tokens, 1.7 GiB more at 65536.

## Configuration

**Settings → Workers → judge-clef**:

| Field | Default | Applied |
|---|---|---|
| `model` | `clef-flash` | next start |
| `threads` | min(8, logical cores) | next start (also the head's threads; `RAYON_NUM_THREADS` in the environment wins for the head) |
| `gpu_layers` | all layers when a GPU backend and device exist | next start (`0` = CPU; the head always runs on the CPU) |
| `context_tokens` | 16384, the reference's `max_length` | next start (512 to 65536, Cloudflare's stated window; 6.0 GiB of VRAM at 16384, 9.2 GiB at 65536) |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls (raise it for prompts over about 10k tokens on the CPU) |

## Building

llama.cpp is compiled from source through `crates/llama-runtime`: `cmake`, a
C++ compiler and `libclang` (for bindgen) are required; if libclang lives
outside the default search path set `LIBCLANG_PATH` (and
`BINDGEN_EXTRA_CLANG_ARGS=-I<clang>/include` when its builtin headers are not
found). Linux x86_64 builds also need the Vulkan loader headers, the SPIR-V
headers and `glslc` (Ubuntu: `libvulkan-dev spirv-headers glslc`) to compile
the Vulkan module (only the module links `libvulkan`, the binary does not).
The build copies the modules and libraries beside the binary, so
`target/release` has the published layout; the release catalog ships them as
the artifact's `companions`. Windows is not published yet.

- Give this package its own `CARGO_TARGET_DIR`: llama-cpp-sys-2 relinks its
  shared libraries in the profile directory and fails with "File exists" when
  two packages that build llama.cpp share one.
- `build.rs` adds `$ORIGIN` to the binary's runpath, because a dependency's
  build script cannot; `readelf -d target/release/judge-clef | grep RUNPATH`
  shows it.
- Run the real model from a release build: the debug build is far too slow at
  this size. The `#[ignore]` real-model tests read the checkpoint from
  `CLEF_CHECKPOINT_DIR`, in the layout above.

## License

Apache-2.0. The Clef-Flash weights, joint schema head and tokenizer are
Cloudflare's (`Cloudflare/clef-flash`, Apache-2.0), built on Qwen3.5-9B
(Apache-2.0); the backbone GGUF is bartowski's conversion of them
(`bartowski/Cloudflare_clef-flash-GGUF`, Apache-2.0).

For the full API, read the hub's [reference](../judge/reference.md).
