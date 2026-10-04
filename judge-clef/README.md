# judge-clef

[Clef-Flash](https://huggingface.co/Cloudflare/clef-flash) provider for the
[`judge`](../judge/) hub, running **inside the worker**: no API key, no Python,
no llama-server. Clef-Flash is Cloudflare's decision model
([announcement](https://blog.cloudflare.com/clef-decision-models)): a
Qwen3.5-9B backbone and a joint schema head of about 122M parameters. The head
reads the backbone's final hidden state of every prompt token and gives one
score per option of every question, so all the questions of an evaluation are
decided together in one forward pass, with no decoding. This worker runs both
in one graph through llama.cpp's own `clef` architecture:
[`crates/llama-native`](../crates/llama-native/) compiles llama.cpp b11379
from source and calls it through a C shim. The model runs on
the CPU, on Metal (macOS) or on Vulkan (Linux x86_64: AMD, NVIDIA and Intel
GPUs), picked automatically at start.

## Hardware selection

- **macOS**: Metal is linked in; Apple Silicon GPUs run the model.
- **Linux x86_64**: llama.cpp's backends are modules loaded at start from the
  binary's directory (`GGML_BACKEND_DL`). The Vulkan module loads wherever a
  Vulkan loader and driver exist (`libvulkan.so.1`); without them it is
  skipped and the CPU runs the model, so the same package works on GPU
  desktops, servers and containers. The CPU module is picked for the host
  (AVX2, AVX-512, AMX variants). The package ships `libllama`, `libggml`,
  `libggml-base`, the CPU variants and the Vulkan module beside the binary,
  which finds them through its `$ORIGIN` runpath.
- **Linux aarch64**: CPU, statically linked.

By default the whole model, joint schema head included, runs on the GPU when
there is one. `gpu_layers: 0` keeps it all on the CPU; `threads` sets
llama.cpp's CPU threads. The chosen device is logged when the model loads, as
`selected inference device`, and shown in the settings form.

## Install

```bash
iii trigger compose::add worker=judge-clef
```

The functions register at start; the model loads on demand. While clef is the
judge hub's **default provider** (`provider: clef` under **Settings → Workers →
judge**) it loads at once and stays loaded. Otherwise the first call that names
`"provider": "clef"` (or comes from a session that picked it) loads it, and it
is released (VRAM included, about 5.5 GiB) after 10 minutes without calls. A call
that cannot wait for the load answers `deadline` while the load goes on for the
next one.
The first load downloads the pinned checkpoint into the hf-hub cache
(`$HF_HOME`, default `~/.cache/huggingface`): first the tokenizer from
`Cloudflare/clef-flash`, then the 6.5 GB `Clef-Flash-Q4_K_M.gguf` from
`ggml-org/Clef-Flash-GGUF` (llama.cpp's `clef` conversion: the backbone and
the joint schema head in one file). To keep it loaded while
another provider is the default, turn on **Keep every local provider loaded**
(`preload_all`) in the judge settings. A hub build that does not expose
`judge::configuration-id` keeps the model loaded from the start.

Air-gapped installs point `III_CLEF_CHECKPOINT_DIR` (or `--checkpoint-dir`) at
a directory holding one checkpoint, loaded as the configured `model`:

| File | Content |
|---|---|
| `model.gguf` | the Clef GGUF (arch `clef`: backbone and joint schema head), from `ggml-org/Clef-Flash-GGUF` |
| `tokenizer.json` | the tokenizer, from `Cloudflare/clef-flash` |

The files may be symlinks, for example into an hf-hub snapshot. A GGUF of any
other architecture is refused at load: a `qwen35` conversion of the backbone
alone would load without the head. The tokenizer is the Hugging Face
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
own tokens. A softmax over each question's scores gives its probabilities, at
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
`context_window`.

`tests/clef.rs` checks the token counts and probabilities of 10 records (28
questions) against Cloudflare's own pipeline, `joint_schema_model.py` with the
bf16 backbone and head (`tests/fixtures/make_reference.py`). The shipped
Q4_K_M differs by:

| Device | max \|Δp\| | mean \|Δp\| | reference's top option |
|---|---|---|---|
| CPU | 0.065 | 0.0067 | 26 of 28 |
| Vulkan | 0.080 | 0.0065 | 26 of 28 |

Both misses are near-ties: orders/region, whose reference top two are 0.005
apart (the f32 Hugging Face backbone flips it too), and deploy_log/severity,
0.017 apart. The test's tolerance is 0.14 and it runs on the CPU
(`CLEF_GPU_LAYERS=all` runs it on the GPU). `Clef-Flash-Q8_0.gguf` is about 3x
closer (max 0.03, mean 0.003, 27 of 28) but a 9.7 GB file and about 2.5 GiB
more VRAM.

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

## Speed and memory (Q4_K_M, i9-14900K, RX 6900 XT 16 GiB)

An evaluation is one forward pass over one prompt, backbone and joint schema
head together, so its time follows the prompt's length, not its number of
questions. The evaluations of one request run one after another, and so do
those of concurrent requests. Per evaluation:

| Prompt | 356 tokens | 2k tokens | 8k tokens | 16k tokens |
|---|---|---|---|---|
| Vulkan | 0.25 s | 1.3 s | 6.7 s | 15.4 s |
| CPU, 16 threads | 5 s | 31 s | – | 280–360 s |

The CPU timings were taken while other work loaded the machine (8 threads are
about as fast). A prompt near 16k tokens cannot finish on the CPU within the
default `max_timeout_ms` of 300000 (5 minutes): use a GPU, or raise
`max_timeout_ms` and the request's `timeout_ms`.

Each pass runs in a llama.cpp context sized to its prompt (rounded up to 256
tokens), created for it and freed after it, so between evaluations only the
weights hold memory: 5.5 GiB of VRAM on Vulkan. A pass adds its compute
buffers for its duration: 5.7 GiB of VRAM in all for a 356-token prompt,
7.6 GiB for 8k tokens and 10.1 GiB for 16384, with 1 GiB of pinned host
memory. At 16k tokens llama.cpp also reallocates them as the pass starts, since
it sized them for a head with one question and one option: for about 30 ms
VRAM reaches 14.1 GiB and pinned memory 1.75 GiB, so the GPU needs that much
free. The worker's own memory peaks at 0.9 GiB, besides 0.6 GiB of the mapped
GGUF. Creating and freeing the context costs about 15 ms at 356 tokens and
0.1–0.2 s at 16k. On the CPU a 16k pass needs about 7 GiB of RAM besides the
mapped GGUF. Flash attention is on: without it the attention scores grow with
the square of the prompt, and on Vulkan llama.cpp moves them to the CPU (16k
tokens took 101–110 s instead of 15 s, with about 18 GiB of host compute
buffers).

### Why the window stops at 16384 tokens

The whole prompt is one llama.cpp batch, so the compute buffers grow with it,
and the GPU's kernel driver kills a job that runs too long. On the RX 6900 XT
(amdgpu, 2 s job timeout), 24576 tokens fit in VRAM (13.0 GB) but the pass lost
the device every time, and the worker process did not survive it. 32768 tokens need more VRAM than the card has. `context_tokens`
therefore stops at 16384, the reference's `max_length` and the default; on a
smaller GPU, or one other processes share, lower it. The worker also sets
`GGML_VK_MAX_NODES_PER_SUBMIT=10` unless the environment sets it: in a fresh
context ggml-vulkan submits every 100 graph nodes, which at 16k tokens comes
close to the 2 s timeout; 10 nodes per submit cost at most 0.5%.

### Cancellation

`judge-clef::cancel` and the deadline are checked while an evaluation waits for
the model and again just before its pass. A pass that has started cannot be
stopped (llama.cpp's Vulkan backend has no abort hook): the caller gets its
`cancelled` or `deadline` answer at once, but the pass holds the model until it
ends, up to about 15 s at 16k tokens on Vulkan and minutes on the CPU, and the
next evaluation waits for it.

## Configuration

**Settings → Workers → judge-clef**:

| Field | Default | Applied |
|---|---|---|
| `model` | `clef-flash` | next start |
| `threads` | min(8, logical cores) | next start (llama.cpp's CPU threads) |
| `gpu_layers` | the whole model when a GPU backend and device exist | next start (`0` = CPU) |
| `context_tokens` | 16384, the reference's `max_length` | next start (512 to 16384; 14.1 GiB of VRAM at peak at 16384, see above) |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls (raise it for long prompts on the CPU) |

## Building

`crates/llama-native`'s `build.rs` downloads llama.cpp b11379 (`1537a0a8`,
GitHub's source archive, checked against a pinned sha256), applies
`native/*.patch`, builds it with cmake and compiles `native/shim.cpp` against
it; its `src/lib.rs` declares the shim's C functions by hand. It needs `curl`,
`tar`, `patch`, `cmake` and a C++17 compiler, and no libclang. An offline
build points `III_LLAMA_CPP_TARBALL` at a copy of the archive, checked the
same way. The patch keeps llama.cpp from reserving a logits buffer the `clef`
pass never writes (15 GiB of host RAM at 16k tokens). Linux x86_64 builds also
need the Vulkan loader headers, the SPIR-V headers and `glslc` (Ubuntu:
`libvulkan-dev spirv-headers glslc`) to compile the Vulkan module (only the
module links `libvulkan`, the binary does not). The build copies the modules
and libraries beside the binary, so `target/release` has the published layout;
the release catalog ships them as the artifact's `companions`. Windows is not
published yet.

- Give this package its own `CARGO_TARGET_DIR`: it lays its llama.cpp beside
  the binaries as `libllama.so.0` and `libggml*.so`, the names the older
  llama.cpp of judge-decider, judge-semif and judge-laya uses too.
- `build.rs` adds `$ORIGIN` to the binary's runpath;
  `readelf -d target/release/judge-clef | grep RUNPATH` shows it.
- Run the real model from a release build: the debug build is far too slow at
  this size. The `#[ignore]` real-model tests read the checkpoint from
  `CLEF_CHECKPOINT_DIR`, in the layout above. The tiny test model
  (`tests/fixtures/tiny/model.gguf`) comes from
  `tests/fixtures/make_fixtures.py`.

## License

Apache-2.0. The Clef-Flash weights, joint schema head and tokenizer are
Cloudflare's (`Cloudflare/clef-flash`, Apache-2.0), built on Qwen3.5-9B
(Apache-2.0); the GGUF is ggml-org's conversion of them
(`ggml-org/Clef-Flash-GGUF`, Apache-2.0).

For the full API, read the hub's [reference](../judge/reference.md).
