# judge-decider

[decider](https://github.com/Mapika/decider) provider for the
[`judge`](../judge/) hub, running **inside the worker**: no API key, no Python,
no llama-server. decider-4b v2 is a Qwen3.5-4B-Base fine-tune trained for
exactly this readout (JevBench v1.4.2 rank 1, 64.1): one forward pass per
decision, a softmax over the next-token logits of the option labels at the
model's fitted temperature, no decoding. This worker reads it in decider's own
prompt layout through llama.cpp (the
[`llama-cpp-2`](https://crates.io/crates/llama-cpp-2) crate) on the CPU, on
Metal (macOS) or on Vulkan (Linux x86_64: AMD, NVIDIA and Intel GPUs), picked
automatically at start.

On an RX 6900 XT (Vulkan), against the other judge providers on the same
machine:

| provider | labelled tickets: department / urgency / churn / refund | per ticket | `directory::search_functions` (22 capabilities): hit / top-1 / precision |
|---|---|---|---|
| `decider` (decider-4b-v2) | 1.00 / 0.79 / 0.96 / 0.96 | 0.30 s | 22 / 22 / 0.94 |
| `semif` (Qwen3.5-4B) | 0.88 / 0.62 / 1.00 / 1.00 | 0.29 s | 21 / 18 / 0.81 |
| `typesafe` (hosted Jev) | 0.96 / 0.75 / 0.96 / 1.00 | 0.25 s | 22 / 22 / 0.99 |

## Hardware selection

- **macOS**: Metal is linked in; Apple Silicon GPUs run the model.
- **Linux x86_64**: llama.cpp's backends are modules loaded at start from the
  binary's directory (`GGML_BACKEND_DL`). The Vulkan module loads wherever a
  Vulkan loader and driver exist (`libvulkan.so.1`); without them it is
  skipped and the CPU runs the model, so the same package works on GPU
  desktops, servers and containers. The CPU module is picked for the host
  (AVX2, AVX-512, AMX variants). The package ships `libllama`, `libggml`,
  `libggml-base`, the CPU variants and the Vulkan module (≈77 MB) beside the
  binary, which finds them through its `$ORIGIN` runpath.
- **Linux aarch64**: CPU, statically linked.

`gpu_layers: 0` keeps the model on the CPU even when a GPU is present. The
chosen device is logged at start as `selected inference device`.

## Install

```bash
iii trigger compose::add worker=judge-decider
```

The model loads only while decider is the judge hub's **default provider**
(`provider: decider` under **Settings → Workers → judge**): the worker follows
the hub's configuration and, when another provider becomes the default,
unregisters its functions and releases the model (VRAM included, about 6 GB);
it loads again when decider is selected. The first load downloads the pinned
GGUF (`mindchain/decider-4b-v2-GGUF` Q4_K_M, 2.7 GB, quantized by llama.cpp
from `Mapika/decider-4b` at tag `v2`) into the hf-hub cache (`$HF_HOME`,
default `~/.cache/huggingface`). The functions register only once the model
answers; until then, and while another provider is the default, the hub
reports `provider_unavailable`, also for calls that name
`"provider": "decider"`. To keep it loaded while another provider is the
default, turn on **Keep every local provider loaded** (`preload_all`) in the
judge settings. A hub build that does not expose `judge::configuration-id`
leaves the model loaded from the start. Air-gapped installs set
`III_DECIDER_GGUF` to a local GGUF file.

The repository's `main` is v2.1, which trades some of v2's accuracy on hard
decisions for sampled play and has no published GGUF; v2 is the JevBench
rank-1 revision.

Make it the hub's default, then call the hub:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
  "timeout_ms": 60000, "provider": "decider",
  "evaluations": [{
    "id": "ticket-4411",
    "state": {"subject": "Duplicate charge on invoice #4411",
              "body": "We were billed twice for March. Refund the duplicate today or we cancel."},
    "questions": {
      "department": {"type": "choice", "instructions": "Which team should handle it?", "criteria": {"billing": "invoices, payments, refunds", "technical": "bugs, outages"}},
      "urgency": {"type": "score", "instructions": "How urgent is it?", "criteria": ["not urgent", "soon", "critical deadline or blocking issue"]},
      "churn_risk": {"type": "noul", "instructions": "Does the user threaten to cancel or leave?"}
    }
  }]
}'
```

## How questions map to decider

The worker builds decider's plain state-first prompt as decider-ai 1.5.0's
`/v1/systemone` does (`decider/systemone.py`, `decider/prompt_fast.py`), one
independent prompt per decision:

```
Context:
<state>

Question: <instructions>
Options:
(A) <option>
(B) <option>
Answer: (
```

The state is a string as-is, or Python JSON with `_index` written into every
element of an 8+ element array. `Context:` and each question block are
tokenized separately, as decider does; up to ten options render as one
string, more use one label token each (`A`–`Z`, then the two-letter labels
that are single tokens), up to 255 options. The label logits go through a
softmax at v2's temperature 1.935.

| Contract | decider options | Answer |
|---|---|---|
| `noul` | `no` then `yes`, each followed by its `criteria` description after `: ` | `noul` = P(yes) |
| `choice` | one per criterion, `key: description` | argmax, probabilities, confidence |
| `score` | one yes/no prompt per level: `<instructions>\nProposed answer: <level>\nDoes the proposed answer fit?` | the levels' P(yes) normalized; Σ i·p_i |

Score levels are judged in isolation (`isolated_levels`), without their number
or their neighbours, as decider-4b v2 was trained. `confidence` is decider's
(TypeSafe's definitions): for a choice `(n·p_max − 1) / (n − 1)`, 0 for a
uniform distribution and 1 for all mass on one option; for a score
1 − the expected distance from the likeliest level over the mean distance of
the levels from the middle of the scale. A question without instructions asks
"Which answer fits the context?"; a noul needs instructions or a `true` /
`false` description. A prompt longer than `context_tokens` answers
`payload_too_large`: the state is never truncated.

Every prompt of one evaluation starts with the same `Context:` tokens. The
worker prefills them once, snapshots the sequence, restores it into up to
`parallel_questions` sequences and decodes the question blocks together in
one batch (`iii_llama_runtime::scorer`, shared with judge-semif).
`usage.input_tokens` counts the decoded tokens (the shared prefix once).

`tests/decider.rs` checks the prompts and token ids against decider's own
rows and the answers (probabilities and confidence) against the bf16 weights
computed in float32 (`tests/fixtures/make_decider_prompts.py`): an f16
conversion agrees to 0.0025, Q8_0 to 0.0074, and the shipped Q4_K_M to 0.12
on a near-flat 12-option question (0.02 elsewhere). For the closer Q8_0
(4.5 GB), convert `Mapika/decider-4b` at `v2` with llama.cpp's
`convert_hf_to_gguf.py --outtype q8_0 --no-mtp` and point `III_DECIDER_GGUF`
at it.

## Configuration

**Settings → Workers → judge-decider**:

| Field | Default | Applied |
|---|---|---|
| `model` | `decider-4b-v2` | next start |
| `threads` | min(8, logical cores) | next start |
| `gpu_layers` | all layers when a GPU backend and device exist | next start (`0` = CPU) |
| `context_tokens` | 16384 | next start |
| `parallel_questions` | 8 | next start (`1` = one question per decode) |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls |

## Building

llama.cpp is compiled from source through `crates/llama-runtime`: `cmake`, a C++ compiler and `libclang`
(for bindgen) are required; if libclang lives outside the default search path
set `LIBCLANG_PATH` (and `BINDGEN_EXTRA_CLANG_ARGS=-I<clang>/include` when its
builtin headers are not found). Linux x86_64 builds also need the Vulkan
loader headers, the SPIR-V headers and `glslc` (Ubuntu: `libvulkan-dev
spirv-headers glslc`) to compile the Vulkan module (only the module links
`libvulkan`, the binary does not). The build copies the modules and libraries
beside the binary, so `target/release` has the published layout; the release
catalog ships them as the artifact's `companions`. Windows is not published
yet.

For the full API, read the hub's [reference](../judge/reference.md).
