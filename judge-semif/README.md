# judge-semif

[SemIf](https://github.com/TheoLeeCJ/SemIf) provider for the [`judge`](../judge/)
hub, running **inside the worker**: no API key, no Python, no llama-server.
SemIf reads runtime-defined decisions from a frozen open LLM: one forward pass,
a softmax over the next-token logits of the option letters `A`–`P`, no decoding.
Its published Qwen3.5-4B baseline agrees with Jev's own references on 0.845 of
TypeSafe's public subset (Jev 0.883). This worker runs the same prompt through
llama.cpp (the [`llama-cpp-2`](https://crates.io/crates/llama-cpp-2) crate) on
the CPU, on Metal (macOS) or on Vulkan (`--features vulkan`).

## Install

```bash
iii trigger compose::add worker=judge-semif
```

On the first start the worker downloads the pinned GGUF (`qwen3.5-4b`:
`bartowski/Qwen_Qwen3.5-4B-GGUF` Q4_K_M, 3.0 GB) into the hf-hub cache
(`$HF_HOME`, default `~/.cache/huggingface`). Its functions register only once
the model answers; until then the hub reports `provider_unavailable`.
Air-gapped installs set `III_SEMIF_GGUF` to a local GGUF file.

Route the hub to it per call with `"provider": "semif"`, or make it the default
under **Settings → Workers → judge**:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
  "timeout_ms": 60000, "provider": "semif",
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

## How questions map to SemIf

Each question becomes one SemIf decision: `{"evidence": state, "criterion":
instructions, "options": [...]}` in SemIf's frozen system prompt and Qwen chat
template (thinking disabled), tokenized by the GGUF vocabulary. Token ids match
SemIf's reference tokenizer (`tests/prompt.rs`), and on the CPU the option
probabilities match SemIf's own llama.cpp backend to four decimals.

| Contract | SemIf options | Answer |
|---|---|---|
| `noul` | A = `criteria.true` (or "Yes, the statement holds."), B = `criteria.false` | `noul` = P(A) |
| `choice` | one per criterion, `key: description` | argmax, probabilities, confidence |
| `score` | `level i: <level>` | Σ i·p_i, probabilities, confidence, legend |

`confidence` is 1 − normalized entropy of the option distribution; SemIf's
probabilities are conditional on the offered options and uncalibrated. At most
16 options per question. A prompt longer than `context_tokens` answers
`payload_too_large`: SemIf never truncates evidence.

Every question of one evaluation shares its state, and SemIf puts the evidence
first, so the worker prefills that prefix once, snapshots it, and restores it
before each question (SemIf's serial prefix reuse). `usage.input_tokens` counts
the decoded tokens (the shared prefix once).

## Speed (Qwen3.5-4B Q4_K_M, i9-14900K, RX 6900 XT)

| | one short decision (~140 tokens) | 16 questions on a 541-token state, fresh | with prefix reuse |
|---|---|---|---|
| CPU, 8 threads | 1.1–1.4 s | 6.1 s each | 0.66 s each |
| Vulkan (RX 6900 XT) | 67–140 ms | 227 ms each | 57 ms each + 0.29 s prefill |

GPU and prefix-reuse probabilities differ from fresh CPU scoring by up to 0.02,
as SemIf documents for its own fast paths.

## Configuration

**Settings → Workers → judge-semif**:

| Field | Default | Applied |
|---|---|---|
| `model` | `qwen3.5-4b` | next start |
| `threads` | min(8, logical cores) | next start |
| `gpu_layers` | all layers when a GPU backend and device exist | next start (`0` = CPU) |
| `context_tokens` | 16384 | next start |
| `max_request_bytes` | 8388608 | new calls |
| `max_timeout_ms` | 300000 | new calls |

## Building

llama.cpp is compiled from source: `cmake`, a C++ compiler and `libclang`
(for bindgen) are required; if libclang lives outside the default search path
set `LIBCLANG_PATH` (and `BINDGEN_EXTRA_CLANG_ARGS=-I<clang>/include` when its
builtin headers are not found). `--features vulkan` also needs the Vulkan
loader headers and `glslc`. Published binaries use default features (CPU on
Linux, Metal on macOS); Windows is not published yet.

For the full API, read the hub's [reference](../judge/reference.md).
