# nanochat worker

A Python worker that brings [Karpathy's nanochat](https://github.com/karpathy/nanochat) (the minimal full-stack ChatGPT clone) onto the III engine. Train GPT models from scratch, fine-tune them, evaluate benchmarks, and serve chat completions, all as live iii functions that any connected worker can discover and call.

nanochat is ~7,000 lines of Python that trains a GPT-2 level model in ~2 hours on 8xH100 for ~$48. This worker wraps its entire pipeline (tokenizer, pretraining, SFT, evaluation, inference, tool use) into 13 registered functions with typed schemas and proper triggers.

## Why this exists

nanochat is a standalone Python script. You train a model, then serve it with FastAPI. Nothing else on the engine can talk to it.

This worker changes that. Once it connects to an iii engine, every capability becomes a function that any other worker (Rust, TypeScript, Python) can invoke via `trigger("nanochat.chat.complete", ...)`. Training runs report progress to iii state. Conversations persist across sessions. The model can be hot-swapped without restarting the worker.

## Prerequisites

- Python 3.10+
- iii-sdk 0.10.0+ (`pip install iii-sdk`)
- PyTorch 2.0+ (`pip install torch`)
- nanochat dependencies: `pip install tiktoken tokenizers rustbpe datasets pyarrow psutil`
- A running iii engine on `ws://localhost:49134` (or configure via `--engine-url`)
- For GPU inference/training: CUDA-capable GPU with sufficient VRAM

The nanochat source is included as a git submodule. If you cloned without `--recurse-submodules`, run `git submodule update --init`. To use a different nanochat checkout, set `NANOCHAT_DIR` or pass `--nanochat-dir`.

## Quick start

```bash
# Clone the workers repo with the nanochat submodule
git clone --recurse-submodules https://github.com/iii-hq/workers.git
cd workers/nanochat

# Install dependencies
pip install iii-sdk torch tiktoken tokenizers rustbpe

# Install nanochat's own dependencies
pip install -r nanochat-upstream/pyproject.toml  # or: cd nanochat-upstream && pip install -e .

# Start without a model (for testing registration and non-GPU functions)
python worker.py --no-autoload

# Start with a trained SFT model on CUDA
python worker.py --source sft --device cuda

# Start with a base model on MPS (Apple Silicon)
python worker.py --source base --device mps
```

The nanochat source is included as a git submodule at `nanochat-upstream/` pointing to [karpathy/nanochat](https://github.com/karpathy/nanochat). Training functions run the actual nanochat scripts as subprocesses from this directory, so you get 100% fidelity to the original implementation.

## Functions

The worker registers 20 functions, each with an HTTP or queue trigger. Every handler uses Pydantic type hints for automatic request/response schema extraction, so the engine knows the exact input/output shape of every function.

**nanochat.chat.complete**:`POST /nanochat/chat/completions`

Takes a list of messages (OpenAI-style `role`/`content` format), generates a completion using the loaded model. Supports `temperature`, `top_k`, and `max_tokens`. Persists the full conversation to iii state under `nanochat:sessions` with the returned `session_id`.

**nanochat.chat.stream**:`POST /nanochat/chat/stream`

Same as `chat.complete` but generates tokens one at a time internally. Currently returns the full text (not SSE streaming):the token-by-token generation prevents the model from generating past `<|assistant_end|>` tokens, matching nanochat's original behavior.

**nanochat.chat.history**:`GET /nanochat/chat/history`

Reads conversation history from iii state. Pass `session_id` to get a specific session, or omit it to list all sessions.

**nanochat.model.load**:`POST /nanochat/model/load`

Loads a nanochat checkpoint into GPU memory. Accepts `source` ("base", "sft", or "rl"), optional `model_tag`, `step`, and `device`. After loading, writes model metadata to `nanochat:models` state scope. The loaded model is immediately available to all chat and eval functions.

**nanochat.model.status**:`GET /nanochat/model/status`

Returns current model state: whether a model is loaded, its source, device, architecture config (`n_layer`, `n_embd`, `vocab_size`, `sequence_len`), and total parameter count.

**nanochat.tokenizer.encode**:`POST /nanochat/tokenizer/encode`

Encodes text (string or list of strings) to BPE token IDs using nanochat's RustBPE tokenizer. Prepends BOS token automatically. Returns the token list and count.

**nanochat.tokenizer.decode**:`POST /nanochat/tokenizer/decode`

Decodes a list of token IDs back to text.

**nanochat.tools.execute**:`POST /nanochat/tools/execute`

Executes arbitrary Python code in a sandboxed environment. Returns stdout, stderr, success status, and any errors. This mirrors nanochat's built-in tool use (calculator, code execution) that models learn during SFT training.

**nanochat.eval.core**:`POST /nanochat/eval/core`

Runs the CORE benchmark (DCLM paper) on the loaded model. Results are stored to `nanochat:evals` state scope with timestamps.

**nanochat.eval.loss**:`POST /nanochat/eval/loss`

Evaluates bits-per-byte on the validation set. This is the vocab-size-invariant loss metric nanochat uses to compare models across different tokenizers.

**nanochat.train.sft**:Queue `nanochat-training`

Runs supervised fine-tuning. This is a long-running function designed to be triggered via queue (`TriggerAction.Enqueue(queue="nanochat-training")`). Reports step-by-step progress and loss values to `nanochat:training` state scope. Other workers can poll `nanochat.train.status` to monitor progress.

**nanochat.train.status**:`GET /nanochat/train/status`

Reads training run status from iii state. Pass `run_id` to get a specific run, or omit it to list all runs.

**nanochat.health**:`GET /nanochat/health`

Returns worker health, model loaded status, device, and source.

## State scopes

All persistent state goes through iii `state::get/set` primitives. The worker uses four scopes:

- **nanochat:sessions**:Conversation history keyed by session_id. Each entry contains the full message list, model source used, and token count.
- **nanochat:models**:Model metadata. The `current` key always reflects the loaded model's config.
- **nanochat:training**:Training run progress keyed by run_id. Contains status (running/complete/failed), step count, loss values, and device info.
- **nanochat:evals**:Evaluation results keyed by `core-{timestamp}` or `loss-{timestamp}`. Contains metric values and model source.

## Testing

Tested against a live iii engine (v0.10.0) on macOS with Python 3.11. All 13 functions and 13 triggers register on connect. Functions that need a loaded model return clear error messages when none is loaded:the worker stays alive through all error cases.

```
OK   nanochat.health              {"status": "ok", "model_loaded": false}
OK   nanochat.model.status        {"loaded": false}
OK   nanochat.chat.history        {"sessions": []}
OK   nanochat.train.status        {"runs": []}
OK   nanochat.tools.execute       {"success": true, "stdout": "3628800\n"}
WARN nanochat.tokenizer.encode    {"error": "tokenizer.pkl not found"}
WARN nanochat.tokenizer.decode    {"error": "tokenizer.pkl not found"}
WARN nanochat.chat.complete       {"error": "No model loaded"}
WARN nanochat.eval.core           {"error": "No model loaded"}
OK   nanochat.health              {"status": "ok"}  (still alive after errors)

10/10 responded, 0 crashes
```

The WARN results are expected:`tokenizer.encode`/`decode` need a trained tokenizer (run `tok_train.py` first or load a model), and `chat.complete`/`eval.core` need a loaded model via `nanochat.model.load`.

### Known issues

**Null payloads time out.** The iii-sdk v0.10.0 Python SDK drops invocations with `payload: None`. Always pass `payload: {}` for functions that don't need input.

**Unhandled handler exceptions crash the WebSocket.** If a handler raises without catching, the SDK's connection state corrupts and all subsequent calls fail with `function_not_found` until the worker reconnects. Every handler in this worker is wrapped with `safe()` to prevent this.

**`multiprocessing.Process` breaks the connection.** nanochat's original code execution sandbox uses `multiprocessing.Process`, but `fork()` in a multi-threaded Python process corrupts the SDK's asyncio event loop. We use in-process `exec()` with stdout/stderr capture instead.

## Calling from other workers

Any worker on the same engine can invoke nanochat functions:

```python
# Python
from iii import register_worker
iii = register_worker("ws://localhost:49134")

result = iii.trigger({
    "function_id": "nanochat.chat.complete",
    "payload": {
        "messages": [{"role": "user", "content": "What is the capital of France?"}],
        "temperature": 0.8,
    }
})
print(result["content"])
```

```typescript
// TypeScript
import { registerWorker } from 'iii-sdk'
const iii = registerWorker('ws://localhost:49134')

const result = await iii.trigger({
  function_id: 'nanochat.chat.complete',
  payload: {
    messages: [{ role: 'user', content: 'What is the capital of France?' }],
    temperature: 0.8,
  },
})
```

```rust
// Rust
let result = iii.trigger("nanochat.chat.complete", json!({
    "messages": [{"role": "user", "content": "What is the capital of France?"}],
    "temperature": 0.8
})).await?;
```

## License

Apache-2.0
