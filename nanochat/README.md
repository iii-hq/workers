# nanochat worker

A Python worker that brings [Karpathy's nanochat](https://github.com/karpathy/nanochat) onto the III engine. 20 functions covering the full LLM pipeline: tokenizer training, base pretraining, supervised fine-tuning, RL fine-tuning (GRPO), CORE/BPB/ChatCORE evaluation, inference with tool use, checkpoint management, and conversation persistence.

nanochat trains a GPT-2 level model in ~2 hours on 8xH100 for ~$48. This worker wraps the entire pipeline as iii functions that any connected worker (Rust, TypeScript, Python) can call. Training runs the actual nanochat scripts as subprocesses via a pre-forked launcher, so you get 100% fidelity to the original implementation. Inference, evaluation, and tokenization run in-process for speed.

## Prerequisites

- Python 3.10+
- PyTorch 2.0+
- iii-sdk 0.10.0+
- nanochat dependencies: tiktoken, tokenizers, rustbpe, pyarrow, wandb
- A running iii engine on `ws://localhost:49134`
- For training/inference: CUDA GPU recommended. CPU and MPS work but are slow.

## Quick start

```bash
git clone --recurse-submodules https://github.com/iii-hq/workers.git
cd workers/nanochat

pip install iii-sdk torch tiktoken tokenizers rustbpe pyarrow wandb pydantic
cd nanochat-upstream && pip install -e . && cd ..

# Start without loading a model
python worker.py --no-autoload

# Start with a trained SFT model
python worker.py --source sft --device cuda
```

The nanochat source is included as a git submodule at `nanochat-upstream/`. Training functions run the actual nanochat scripts (`scripts/base_train.py`, `scripts/chat_sft.py`, etc.) as subprocesses from this directory.

## Functions

20 functions, 20 triggers (all HTTP). Every handler uses Pydantic type hints for automatic request/response schema extraction.

**Chat**

- `nanochat.chat.complete` POST - Generate a chat completion. Takes OpenAI-style messages, returns content + session_id. Conversation persisted to iii state.
- `nanochat.chat.stream` POST - Same as complete but generates token-by-token internally.
- `nanochat.chat.history` GET - Read conversation history from iii state by session_id.

**Model**

- `nanochat.model.load` POST - Load a checkpoint into memory. Accepts source (base/sft/rl), model_tag, step, device.
- `nanochat.model.status` GET - Current model config: loaded, source, device, n_layer, n_embd, vocab_size, parameters.
- `nanochat.model.sample` POST - Generate raw text samples with configurable prompt, temperature, top_k, num_samples.

**Tokenizer**

- `nanochat.tokenizer.encode` POST - Text to BPE token IDs.
- `nanochat.tokenizer.decode` POST - Token IDs to text.

**Training** (runs actual nanochat scripts via pre-forked subprocess launcher)

- `nanochat.train.tokenizer` POST - Train BPE tokenizer from dataset. Runs `scripts/tok_train.py`.
- `nanochat.train.base` POST - Pretrain base GPT model. Runs `scripts/base_train.py` with full Muon optimizer, gradient accumulation, LR scheduling, FP8, checkpoint saving.
- `nanochat.train.sft` POST - Supervised fine-tuning with real task mixture (SmolTalk, MMLU, GSM8K, SpellingBee). Runs `scripts/chat_sft.py`.
- `nanochat.train.rl` POST - GRPO reinforcement learning on GSM8K. Runs `scripts/chat_rl.py`.
- `nanochat.train.status` GET - Training run progress from iii state.

**Evaluation** (imports and calls real nanochat eval functions)

- `nanochat.eval.core` POST - CORE benchmark (DCLM). Calls `base_eval.evaluate_core()`.
- `nanochat.eval.loss` POST - Bits-per-byte on validation set. Calls `loss_eval.evaluate_bpb()`.
- `nanochat.eval.chat` POST - ChatCORE evaluation (GSM8K, MMLU, ARC-Easy, ARC-Challenge, HumanEval, SpellingBee). Calls `chat_eval.run_chat_eval()`.

**Checkpoints**

- `nanochat.checkpoint.save` POST - Save current model to disk.
- `nanochat.checkpoint.list` GET - List available checkpoints by source.

**Health**

- `nanochat.health` GET - Worker health, model loaded status, device.
- `nanochat.tools.execute` POST - Execute Python code in-process (not sandboxed).

## State scopes

All state goes through iii `state::get/set`. Five scopes:

- **nanochat:sessions** - Conversation history keyed by session_id.
- **nanochat:models** - Model metadata. The `current` key reflects the loaded model.
- **nanochat:training** - Training run progress keyed by run_id. Updated with parsed metrics from subprocess stdout (step, loss, tok/sec, MFU, BPB, CORE scores).
- **nanochat:evals** - Evaluation results keyed by type and timestamp.
- **nanochat:checkpoints** - Checkpoint metadata.

## How training works

Training functions can't fork subprocesses from inside iii-sdk handlers (fork corrupts the WebSocket on macOS). The worker solves this with a pre-forked subprocess launcher:

1. Before connecting to the iii engine, the worker forks a child process using `multiprocessing` with explicit fork context.
2. The child process waits for job requests on a Pipe.
3. When a training function is triggered, it sends the script name and arguments to the child via the Pipe.
4. The child runs `subprocess.Popen` (safe because it was forked before the WebSocket existed).
5. The child captures all stdout and sends it back.
6. The handler parses stdout for metrics (step, loss, BPB, CORE, ChatCORE, reward) and writes them to iii state.

This gives 100% fidelity to nanochat's training scripts while keeping the iii worker alive.

## E2E test results

Tested on macOS (Apple Silicon, CPU) with iii engine v0.10.0 and Python 3.11. Trained a 2-layer, 1.9M parameter GPT model from scratch (5 steps on CPU), loaded the checkpoint, and ran inference through the worker.

```text
1. Load model   -> loaded=True, params=1,966,134, n_layer=2, n_embd=128
2. Sample        -> "<|bos|>Hello! if ifite Sther made Oite were are..."
3. Chat          -> completion with session tracking (26 tokens)
4. History       -> 1 session stored in iii state
5. Tokenizer     -> encode: 5 tokens, decode roundtrip OK
6. Tools         -> print(42) = 42
7. Model status  -> full config visible (device, layers, vocab, params)
8. Health        -> worker alive after all operations

8/8 passed
```

The generated text is gibberish because the model was only trained for 5 steps. With real GPU training (8xH100, ~2 hours), the model produces coherent chat responses, solves math problems with tool use, and scores competitively on CORE benchmarks.

## Calling from other workers

```python
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

## Known issues

**Null payloads time out.** iii-sdk v0.10.0 drops invocations with `payload: None`. Always pass `{}`.

**Handler exceptions crash WebSocket.** Unhandled exceptions corrupt the SDK's connection. Every handler is wrapped with `safe()` which logs server-side and returns `{"error": "..."}`.

**fork() from handler threads crashes WebSocket.** Both `subprocess.Popen` and `os.system` from inside `run_in_executor` or `asyncio.to_thread` corrupt the asyncio event loop on macOS. The pre-forked launcher solves this for training. `tools.execute` uses in-process `exec()`.

**torch.compile hangs on CPU.** nanochat's `base_train.py` calls `torch.compile(model)` which takes extremely long on CPU. Use GPU for real training.

## License

Apache-2.0
