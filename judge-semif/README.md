# judge-semif

[SemIf](https://github.com/TheoLeeCJ/SemIf) provider for the [`judge`](../judge/)
hub, running **inside the worker**: no API key, no Python, no llama-server.
SemIf reads runtime-defined decisions from a frozen open LLM: one forward pass,
a softmax over the next-token logits of the option letters `A`–`P`, no decoding.
Its published Qwen3.5-4B baseline agrees with Jev's own references on 0.845 of
TypeSafe's public subset (Jev 0.883). This worker runs the same prompt through
llama.cpp (the [`llama-cpp-2`](https://crates.io/crates/llama-cpp-2) crate) on
the CPU, on Metal (macOS) or on Vulkan (`--features vulkan`: AMD, NVIDIA and
Intel GPUs).

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
first. The worker prefills the prompts' longest common token run once,
snapshots it, restores it into up to `parallel_questions` sequences and decodes
their suffixes together in one batch (SemIf's parallel suffixes). The prefix is
found on tokens, not re-tokenized text: a state ending in `{}` merges with the
following `}` two tokens back, which SemIf's own `_state_prefix` rejects.
`usage.input_tokens` counts the decoded tokens (the shared prefix once).

## Speed (Qwen3.5-4B Q4_K_M, i9-14900K, RX 6900 XT)

Per request, 16 questions about one state; `parallel_questions` 8 unless noted:

| | incident (541-token state) | directory block (4.6k-token state, 64 functions) |
|---|---|---|
| CPU, 8 threads | 3.8 s (6.9 s with parallel 1) | 6.6 s |
| Vulkan | 0.79 s (1.18 s with parallel 1) | 3.4 s |

Before the token-level prefix, the directory block decoded every question in
full: 75k tokens and 35.8 s on Vulkan, against 6k tokens and 3.4 s now.
Parallel suffixes pay off on short states; long states are dominated by the
one-time prefill. On the same card llama.cpp's ROCm backend measured within
10% of Vulkan either way, so the worker ships Vulkan alone. A single short
decision takes 67–140 ms on the GPU and 1.1–1.4 s on the CPU.

GPU, reuse and batching change probabilities by up to 0.02 against fresh CPU
scoring, as SemIf documents for its own fast paths.

## Configuration

**Settings → Workers → judge-semif**:

| Field | Default | Applied |
|---|---|---|
| `model` | `qwen3.5-4b` | next start |
| `threads` | min(8, logical cores) | next start |
| `gpu_layers` | all layers when a GPU backend and device exist | next start (`0` = CPU) |
| `context_tokens` | 16384 | next start |
| `parallel_questions` | 8 | next start (`1` = one question per decode) |
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
