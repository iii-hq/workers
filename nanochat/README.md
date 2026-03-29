# nanochat worker

A Python worker that brings [Karpathy's nanochat](https://github.com/karpathy/nanochat) — the minimal full-stack ChatGPT clone — onto the III engine. Train GPT models from scratch, fine-tune them, evaluate benchmarks, and serve chat completions, all as live iii functions that any connected worker can discover and call.

nanochat is ~7,000 lines of Python that trains a GPT-2 level model in ~2 hours on 8xH100 for ~$48. This worker wraps its entire pipeline (tokenizer, pretraining, SFT, evaluation, inference, tool use) into 13 registered functions with typed schemas and proper triggers.

## Why this exists

nanochat is a standalone Python script. You train a model, then serve it with FastAPI. Nothing else on the engine can talk to it.

This worker changes that. Once it connects to an iii engine, every capability becomes a function that any other worker — Rust, TypeScript, Python — can invoke via `trigger("nanochat.chat.complete", ...)`. Training runs report progress to iii state. Conversations persist across sessions. The model can be hot-swapped without restarting the worker.

## Prerequisites

- Python 3.10+
- iii-sdk 0.10.0+ (`pip install iii-sdk`)
- PyTorch 2.0+ (`pip install torch`)
- nanochat dependencies: `pip install tiktoken tokenizers rustbpe datasets pyarrow psutil`
- A running iii engine on `ws://localhost:49134` (or configure via `--engine-url`)
- For GPU inference/training: CUDA-capable GPU with sufficient VRAM

The nanochat source must be available locally. By default, the worker expects it at `./nanochat/` (symlink or copy from the nanochat repo). Override with `--nanochat-dir` or `NANOCHAT_DIR` env var.

## Quick start

```bash
# Clone nanochat
git clone https://github.com/karpathy/nanochat.git /tmp/nanochat

# Symlink into worker directory
ln -s /tmp/nanochat/nanochat ./nanochat

# Install dependencies
pip install iii-sdk torch tiktoken tokenizers rustbpe

# Start without a model (for testing registration and non-GPU functions)
python worker.py --no-autoload

# Start with a trained SFT model on CUDA
python worker.py --source sft --device cuda

# Start with a base model on MPS (Apple Silicon)
python worker.py --source base --device mps
```

## Functions

The worker registers 13 functions, each with an HTTP or queue trigger. Every handler uses Pydantic type hints for automatic request/response schema extraction — the engine knows the exact input/output shape of every function.

**nanochat.chat.complete** — `POST /nanochat/chat/completions`

Takes a list of messages (OpenAI-style `role`/`content` format), generates a completion using the loaded model. Supports `temperature`, `top_k`, and `max_tokens`. Persists the full conversation to iii state under `nanochat:sessions` with the returned `session_id`.

**nanochat.chat.stream** — `POST /nanochat/chat/stream`

Same as `chat.complete` but generates tokens one at a time internally. Currently returns the full text (not SSE streaming) — the token-by-token generation prevents the model from generating past `<|assistant_end|>` tokens, matching nanochat's original behavior.

**nanochat.chat.history** — `GET /nanochat/chat/history`

Reads conversation history from iii state. Pass `session_id` to get a specific session, or omit it to list all sessions.

**nanochat.model.load** — `POST /nanochat/model/load`

Loads a nanochat checkpoint into GPU memory. Accepts `source` ("base", "sft", or "rl"), optional `model_tag`, `step`, and `device`. After loading, writes model metadata to `nanochat:models` state scope. The loaded model is immediately available to all chat and eval functions.

**nanochat.model.status** — `GET /nanochat/model/status`

Returns current model state: whether a model is loaded, its source, device, architecture config (`n_layer`, `n_embd`, `vocab_size`, `sequence_len`), and total parameter count.

**nanochat.tokenizer.encode** — `POST /nanochat/tokenizer/encode`

Encodes text (string or list of strings) to BPE token IDs using nanochat's RustBPE tokenizer. Prepends BOS token automatically. Returns the token list and count.

**nanochat.tokenizer.decode** — `POST /nanochat/tokenizer/decode`

Decodes a list of token IDs back to text.

**nanochat.tools.execute** — `POST /nanochat/tools/execute`

Executes arbitrary Python code in a sandboxed environment. Returns stdout, stderr, success status, and any errors. This mirrors nanochat's built-in tool use (calculator, code execution) that models learn during SFT training.

**nanochat.eval.core** — `POST /nanochat/eval/core`

Runs the CORE benchmark (DCLM paper) on the loaded model. Results are stored to `nanochat:evals` state scope with timestamps.

**nanochat.eval.loss** — `POST /nanochat/eval/loss`

Evaluates bits-per-byte on the validation set. This is the vocab-size-invariant loss metric nanochat uses to compare models across different tokenizers.

**nanochat.train.sft** — Queue `nanochat-training`

Runs supervised fine-tuning. This is a long-running function designed to be triggered via queue (`TriggerAction.Enqueue(queue="nanochat-training")`). Reports step-by-step progress and loss values to `nanochat:training` state scope. Other workers can poll `nanochat.train.status` to monitor progress.

**nanochat.train.status** — `GET /nanochat/train/status`

Reads training run status from iii state. Pass `run_id` to get a specific run, or omit it to list all runs.

**nanochat.health** — `GET /nanochat/health`

Returns worker health, model loaded status, device, and source.

## State scopes

All persistent state goes through iii `state::get/set` primitives. The worker uses four scopes:

- **nanochat:sessions** — Conversation history keyed by session_id. Each entry contains the full message list, model source used, and token count.
- **nanochat:models** — Model metadata. The `current` key always reflects the loaded model's config.
- **nanochat:training** — Training run progress keyed by run_id. Contains status (running/complete/failed), step count, loss values, and device info.
- **nanochat:evals** — Evaluation results keyed by `core-{timestamp}` or `loss-{timestamp}`. Contains metric values and model source.

## SDK patterns used

This worker targets iii-sdk v0.10.0 and uses these patterns:

**Pydantic type hints for auto-schema.** Every handler is annotated with Pydantic input/output models. The SDK's `extract_request_format` and `extract_response_format` automatically convert these to JSON Schema, making every function self-documenting in the engine dashboard. Inside the handler, `Model.model_validate(data)` parses the raw dict the SDK delivers.

**Async handlers for state I/O.** All handlers that touch iii state use `async def` and `await iii_client.trigger_async(...)`. This avoids blocking the SDK's thread pool executor during state reads/writes. GPU-bound work (inference, training) still runs synchronously within the async handler since PyTorch operations release the GIL.

**safe() wrapper for crash prevention.** Every handler is wrapped with `safe()` which catches all exceptions and returns an error dict instead of raising. This is critical because unhandled exceptions in iii-sdk handlers can crash the WebSocket connection, causing all subsequent invocations to fail with "function_not_found" until the worker reconnects. The wrapper preserves `__annotations__` so the SDK's schema extraction still works.

**Service hierarchy.** Functions are organized under `nanochat` with sub-services (`nanochat.chat`, `nanochat.model`, etc.) using `parent_service_id`. This groups functions in the engine dashboard.

**Queue triggers for long-running work.** Training uses a queue trigger (`nanochat-training`) instead of HTTP, so callers don't block waiting for a multi-hour training run to complete.

**TelemetryOptions.** The worker passes `language="python"` and `project_name="nanochat"` to `InitOptions` for engine-level analytics.

## Testing

We tested this worker against a live iii engine (v0.10.0) on macOS (Darwin 25.2.0, Python 3.11). Here are the findings.

### Registration

13 functions and 13 triggers register successfully. The SDK queues WebSocket messages internally — no delays needed between `register_function` and `register_trigger` calls. We initially added `time.sleep(0.1)` between registrations to work around suspected message ordering issues, but the real cause was different (see "Crashes" below). The sleeps were removed.

### Function invocation

All 13 functions respond correctly when invoked via `iii.trigger(...)` from a separate Python worker process. The engine routes invocations by `function_id` and the response returns to the calling worker.

Functions that require a loaded model (`chat.complete`, `chat.stream`, `eval.core`, `eval.loss`) correctly return error messages when no model is loaded. Functions that need a trained tokenizer (`tokenizer.encode`, `tokenizer.decode`) return a `FileNotFoundError` when the tokenizer pickle doesn't exist — this is expected behavior before running nanochat's `tok_train.py`.

### Payload behavior

The iii-sdk v0.10.0 Python SDK has a quirk: `payload: None` causes invocations to time out. The engine appears to drop invocations with null payloads. Passing `payload: {}` (empty dict) works correctly. All our handlers guard against this with `Model.model_validate(data)` which handles both `{}` and populated dicts.

### Crash prevention

The most critical finding: **unhandled exceptions in iii-sdk handlers crash the worker's WebSocket connection.** When a handler raises, the SDK's internal `_handle_invoke` propagates it as a `_TraceContextError`, which corrupts the connection state. After the crash, the worker silently reconnects, but the re-registration happens asynchronously — during this window, all invocations fail with `function_not_found`.

The `safe()` wrapper solves this completely. With it, the worker survived 10/10 sequential invocations including intentional error cases (no model loaded, missing tokenizer file) without a single disconnect.

### Subprocess behavior

nanochat's original `execute_code()` uses `multiprocessing.Process` to sandbox code execution. This caused the worker's WebSocket to disconnect — `fork()` in a multi-threaded Python process (the iii-sdk runs asyncio on a daemon thread) corrupts shared state. We replaced this with in-process `exec()` using `contextlib.redirect_stdout/stderr`. For production use where untrusted code runs, a `subprocess.run` approach (which does `fork+exec`, not bare `fork`) would be safer.

### Async vs sync handlers

Sync handlers work fine but run in the SDK's `run_in_executor` thread pool. For handlers that call `state::get/set` (which itself goes through the WebSocket), async handlers with `trigger_async()` avoid a round-trip through the executor. We measured no latency difference in our testing, but under load the async path would avoid thread pool exhaustion.

### Test results (no model loaded)

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
