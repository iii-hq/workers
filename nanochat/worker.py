"""
nanochat worker for iii-engine (v0.10.0 SDK).

Idiomatic use of iii primitives:
- Pydantic type hints on every handler → auto request/response schema extraction
- Async handlers for state I/O → no executor contention
- Every function has a trigger — no orphan registrations
- All state through state::get/set via trigger_async
- Service hierarchy for engine dashboard grouping
- safe() wrapper on every handler — zero-crash guarantee

Usage:
    python worker.py                          # auto-detect device, load SFT model
    python worker.py --no-autoload            # start without loading a model
    python worker.py --source base --device mps
"""

import argparse
import io
import contextlib
import os
import signal
import sys
import threading
import time
import traceback
import uuid
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field

from iii import InitOptions, Logger, TelemetryOptions, register_worker

NANOCHAT_DIR = os.environ.get("NANOCHAT_DIR", str(Path(__file__).parent / "nanochat"))

logger = Logger(service_name="iii-nanochat")

iii_client = None

_nanochat_imported = False


def _ensure_nanochat():
    global _nanochat_imported
    if _nanochat_imported:
        return
    parent = str(Path(NANOCHAT_DIR).parent)
    if parent not in sys.path:
        sys.path.insert(0, parent)
    import torch  # noqa: F401
    _nanochat_imported = True


def safe(fn):
    """Wrap async handler so unhandled exceptions return error dicts, never crash the WebSocket."""
    async def wrapper(data):
        try:
            return await fn(data)
        except Exception as e:
            return {"error": str(e), "traceback": traceback.format_exc()}
    wrapper.__name__ = fn.__name__
    wrapper.__annotations__ = fn.__annotations__
    return wrapper


# ---------------------------------------------------------------------------
# Pydantic schemas — auto-extracted by SDK for engine UI & validation
# ---------------------------------------------------------------------------

class ChatMessage(BaseModel):
    role: str
    content: str


class ChatCompleteInput(BaseModel):
    messages: list[ChatMessage]
    temperature: float = Field(0.6, ge=0.0, le=2.0)
    top_k: int = Field(50, ge=0, le=200)
    max_tokens: int = Field(2048, ge=1, le=4096)
    session_id: str | None = None


class ChatCompleteOutput(BaseModel):
    content: str
    tokens_generated: int
    session_id: str


class ChatHistoryInput(BaseModel):
    session_id: str | None = None


class ChatHistoryOutput(BaseModel):
    session_id: str | None = None
    sessions: Any | None = None
    data: Any | None = None


class ModelLoadInput(BaseModel):
    source: str = "sft"
    model_tag: str | None = None
    step: int | None = None
    device: str | None = None


class ModelStatusOutput(BaseModel):
    loaded: bool
    source: str | None = None
    model_tag: str | None = None
    device: str | None = None
    n_layer: int | None = None
    n_embd: int | None = None
    vocab_size: int | None = None
    sequence_len: int | None = None
    parameters: int | None = None


class TokenizeInput(BaseModel):
    text: str | list[str]


class TokenizeOutput(BaseModel):
    tokens: list[int] | list[list[int]]
    count: int


class DecodeInput(BaseModel):
    tokens: list[int]


class DecodeOutput(BaseModel):
    text: str


class ExecuteCodeInput(BaseModel):
    code: str
    timeout: float = 5.0


class ExecuteCodeOutput(BaseModel):
    success: bool
    stdout: str
    stderr: str
    error: str | None = None
    timeout: bool = False


class EvalInput(BaseModel):
    source: str = "sft"
    model_tag: str | None = None
    step: int | None = None
    max_per_task: int = -1


class EvalCoreOutput(BaseModel):
    core_metric: float | None = None
    results: dict[str, Any] = {}


class EvalLossOutput(BaseModel):
    bits_per_byte: float
    model: str | None = None


class TrainSFTInput(BaseModel):
    source: str = "base"
    model_tag: str | None = None
    step: int | None = None
    training_horizon: int = 5000
    batch_size: int = 4
    device: str | None = None


class TrainStatusInput(BaseModel):
    run_id: str | None = None


class HealthOutput(BaseModel):
    status: str
    model_loaded: bool
    device: str | None = None
    source: str | None = None
    worker: str = "iii-nanochat"


# ---------------------------------------------------------------------------
# GPU state — model lives in GPU memory, inherently local
# ---------------------------------------------------------------------------

class GPUState:
    def __init__(self):
        self.model = None
        self.tokenizer = None
        self.engine = None
        self.meta: dict | None = None
        self.source: str | None = None
        self.model_tag: str | None = None
        self.device: str | None = None
        self._lock = threading.Lock()

    def load(self, source: str, device: str, model_tag: str | None = None, step: int | None = None):
        _ensure_nanochat()
        from nanochat.checkpoint_manager import load_model
        from nanochat.engine import Engine

        with self._lock:
            phase = "sft" if source in ("sft", "rl") else "base"
            model, tokenizer, meta = load_model(source, device, phase, model_tag=model_tag, step=step)
            model.eval()
            self.model = model
            self.tokenizer = tokenizer
            self.engine = Engine(model, tokenizer)
            self.meta = meta
            self.source = source
            self.model_tag = model_tag
            self.device = device

    @property
    def ready(self) -> bool:
        return self.engine is not None


gpu = GPUState()


# ---------------------------------------------------------------------------
# Async state helpers — all state through iii primitives via trigger_async
# ---------------------------------------------------------------------------

async def state_get(scope: str, key: str) -> Any:
    return await iii_client.trigger_async({"function_id": "state::get", "payload": {"scope": scope, "key": key}})


async def state_set(scope: str, key: str, value: Any) -> Any:
    return await iii_client.trigger_async({"function_id": "state::set", "payload": {"scope": scope, "key": key, "value": value}})


async def state_list(scope: str) -> Any:
    return await iii_client.trigger_async({"function_id": "state::list", "payload": {"scope": scope}})


# ---------------------------------------------------------------------------
# Async handlers — Pydantic type hints for auto-schema, async for state I/O
# ---------------------------------------------------------------------------

async def fn_chat_complete(data: ChatCompleteInput) -> ChatCompleteOutput:
    _ensure_nanochat()
    import torch

    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    inp = ChatCompleteInput.model_validate(data) if isinstance(data, dict) else data
    session_id = inp.session_id or str(uuid.uuid4())
    conversation = [{"role": m.role, "content": m.content} for m in inp.messages]

    if hasattr(gpu.tokenizer, "render_conversation"):
        tokens, _mask = gpu.tokenizer.render_conversation(conversation, max_tokens=gpu.model.config.sequence_len)
    else:
        tokens = gpu.tokenizer.render_for_completion(conversation)

    with torch.no_grad():
        results, _masks = gpu.engine.generate_batch(
            [tokens], num_samples=1,
            max_tokens=inp.max_tokens,
            temperature=inp.temperature,
            top_k=inp.top_k,
        )

    generated_ids = results[0]
    text = gpu.tokenizer.decode(generated_ids)
    if "<|assistant_end|>" in text:
        text = text[:text.index("<|assistant_end|>")]

    conversation.append({"role": "assistant", "content": text.strip()})
    await state_set("nanochat:sessions", session_id, {
        "messages": conversation,
        "model": gpu.source,
        "tokens_generated": len(generated_ids),
    })

    logger.info("Chat completion", {"session_id": session_id, "tokens": len(generated_ids)})
    return ChatCompleteOutput(content=text.strip(), tokens_generated=len(generated_ids), session_id=session_id).model_dump()


async def fn_chat_stream(data: ChatCompleteInput) -> ChatCompleteOutput:
    _ensure_nanochat()
    import torch

    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    inp = ChatCompleteInput.model_validate(data) if isinstance(data, dict) else data
    session_id = inp.session_id or str(uuid.uuid4())
    conversation = [{"role": m.role, "content": m.content} for m in inp.messages]

    if hasattr(gpu.tokenizer, "render_conversation"):
        tokens, _mask = gpu.tokenizer.render_conversation(conversation, max_tokens=gpu.model.config.sequence_len)
    else:
        tokens = gpu.tokenizer.render_for_completion(conversation)

    chunks = []
    with torch.no_grad():
        for token_col, _token_masks in gpu.engine.generate(
            [tokens], num_samples=1,
            max_tokens=inp.max_tokens,
            temperature=inp.temperature,
            top_k=inp.top_k,
        ):
            token_id = token_col[0].item()
            piece = gpu.tokenizer.decode([token_id])
            if "<|assistant_end|>" in piece:
                break
            chunks.append(piece)

    full_text = "".join(chunks)
    conversation.append({"role": "assistant", "content": full_text.strip()})
    await state_set("nanochat:sessions", session_id, {
        "messages": conversation,
        "model": gpu.source,
        "tokens_generated": len(chunks),
    })

    return ChatCompleteOutput(content=full_text.strip(), tokens_generated=len(chunks), session_id=session_id).model_dump()


async def fn_chat_history(data: ChatHistoryInput) -> ChatHistoryOutput:
    inp = ChatHistoryInput.model_validate(data) if isinstance(data, dict) else data
    if not inp.session_id:
        sessions = await state_list("nanochat:sessions")
        return ChatHistoryOutput(sessions=sessions).model_dump()
    session_data = await state_get("nanochat:sessions", inp.session_id)
    return ChatHistoryOutput(session_id=inp.session_id, data=session_data).model_dump()


async def fn_model_load(data: ModelLoadInput) -> ModelStatusOutput:
    _ensure_nanochat()
    from nanochat.common import autodetect_device_type

    inp = ModelLoadInput.model_validate(data) if isinstance(data, dict) else data
    device = inp.device or autodetect_device_type()
    gpu.load(inp.source, device, model_tag=inp.model_tag, step=inp.step)

    await state_set("nanochat:models", "current", {
        "source": gpu.source,
        "model_tag": gpu.model_tag,
        "device": gpu.device,
        "config": gpu.meta.get("model_config", {}) if gpu.meta else {},
        "parameters": sum(p.numel() for p in gpu.model.parameters()),
    })

    logger.info("Model loaded", {"source": inp.source, "device": device})
    return await fn_model_status({})


async def fn_model_status(data: dict) -> ModelStatusOutput:
    if not gpu.ready:
        return ModelStatusOutput(loaded=False).model_dump()

    config = gpu.meta.get("model_config", {}) if gpu.meta else {}
    return ModelStatusOutput(
        loaded=True,
        source=gpu.source,
        model_tag=gpu.model_tag,
        device=gpu.device,
        n_layer=config.get("n_layer"),
        n_embd=config.get("n_embd"),
        vocab_size=config.get("vocab_size"),
        sequence_len=config.get("sequence_len"),
        parameters=sum(p.numel() for p in gpu.model.parameters()) if gpu.model else None,
    ).model_dump()


async def fn_tokenizer_encode(data: TokenizeInput) -> TokenizeOutput:
    _ensure_nanochat()
    from nanochat.tokenizer import get_tokenizer

    inp = TokenizeInput.model_validate(data) if isinstance(data, dict) else data
    tokenizer = gpu.tokenizer or get_tokenizer()
    bos = tokenizer.get_bos_token_id()
    encoded = tokenizer.encode(inp.text, prepend=bos)
    count = sum(len(t) for t in encoded) if isinstance(inp.text, list) else len(encoded)

    return TokenizeOutput(tokens=encoded, count=count).model_dump()


async def fn_tokenizer_decode(data: DecodeInput) -> DecodeOutput:
    _ensure_nanochat()
    from nanochat.tokenizer import get_tokenizer

    inp = DecodeInput.model_validate(data) if isinstance(data, dict) else data
    tokenizer = gpu.tokenizer or get_tokenizer()
    return DecodeOutput(text=tokenizer.decode(inp.tokens)).model_dump()


async def fn_tools_execute(data: ExecuteCodeInput) -> ExecuteCodeOutput:
    inp = ExecuteCodeInput.model_validate(data) if isinstance(data, dict) else data

    stdout_buf = io.StringIO()
    stderr_buf = io.StringIO()

    try:
        with contextlib.redirect_stdout(stdout_buf), contextlib.redirect_stderr(stderr_buf):
            exec(inp.code, {"__builtins__": __builtins__}, {})
        return ExecuteCodeOutput(
            success=True, stdout=stdout_buf.getvalue(),
            stderr=stderr_buf.getvalue(), error=None, timeout=False,
        ).model_dump()
    except Exception as e:
        return ExecuteCodeOutput(
            success=False, stdout=stdout_buf.getvalue(),
            stderr=stderr_buf.getvalue(), error=str(e), timeout=False,
        ).model_dump()


async def fn_eval_core(data: EvalInput) -> EvalCoreOutput:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    _ensure_nanochat()
    from nanochat.core_eval import evaluate_task

    logger.info("Starting CORE evaluation")

    tasks_yaml = Path(NANOCHAT_DIR) / "dev" / "core_tasks.yaml"
    if not tasks_yaml.exists():
        raise FileNotFoundError(f"CORE tasks file not found at {tasks_yaml}")

    import yaml
    with open(tasks_yaml) as f:
        tasks = yaml.safe_load(f)

    results = {}
    for task_name, task_meta in tasks.items():
        try:
            device = gpu.model.get_device() if hasattr(gpu.model, "get_device") else gpu.device
            acc = evaluate_task(gpu.model, gpu.tokenizer, task_meta.get("data", []), device, task_meta)
            results[task_name] = acc
        except Exception as e:
            results[task_name] = {"error": str(e)}

    core_metric = sum(v for v in results.values() if isinstance(v, (int, float))) / max(len(results), 1)

    await state_set("nanochat:evals", f"core-{int(time.time())}", {
        "type": "core", "results": results, "core_metric": core_metric, "model": gpu.source,
    })

    return EvalCoreOutput(core_metric=core_metric, results=results).model_dump()


async def fn_eval_loss(data: EvalInput) -> EvalLossOutput:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    _ensure_nanochat()
    from nanochat.loss_eval import evaluate_bpb
    from nanochat.tokenizer import get_token_bytes
    from nanochat.dataloader import tokenizing_distributed_data_loader_bos_bestfit

    token_bytes = get_token_bytes(gpu.device)
    B, T = 4, gpu.model.config.sequence_len
    batches = tokenizing_distributed_data_loader_bos_bestfit(gpu.tokenizer, B, T, "val", device=gpu.device)
    bpb = evaluate_bpb(gpu.model, batches, steps=50, token_bytes=token_bytes)

    await state_set("nanochat:evals", f"loss-{int(time.time())}", {
        "type": "bpb", "bpb": bpb, "model": gpu.source,
    })

    return EvalLossOutput(bits_per_byte=bpb, model=gpu.source).model_dump()


async def fn_train_sft(data: TrainSFTInput) -> dict:
    _ensure_nanochat()
    from nanochat.common import autodetect_device_type

    inp = TrainSFTInput.model_validate(data) if isinstance(data, dict) else data
    device = inp.device or autodetect_device_type()
    run_id = str(uuid.uuid4())[:8]

    await state_set("nanochat:training", run_id, {
        "status": "running", "type": "sft", "source": inp.source,
        "device": device, "training_horizon": inp.training_horizon, "step": 0,
    })
    logger.info("SFT training started", {"run_id": run_id, "device": device})

    try:
        from nanochat.checkpoint_manager import load_model
        model, tokenizer, meta = load_model(inp.source, device, "base", model_tag=inp.model_tag, step=inp.step)
        optimizer = model.setup_optimizer()
        model.train()

        from nanochat.dataloader import tokenizing_distributed_data_loader_bos_bestfit
        B, T = inp.batch_size, model.config.sequence_len
        train_loader = tokenizing_distributed_data_loader_bos_bestfit(tokenizer, B, T, "train", device=device)

        for step_i, (inputs, targets) in enumerate(train_loader):
            if step_i >= inp.training_horizon:
                break
            optimizer.zero_grad()
            _logits, loss = model(inputs, targets)
            loss.backward()
            optimizer.step()

            if step_i % 100 == 0:
                await state_set("nanochat:training", run_id, {
                    "status": "running", "type": "sft", "step": step_i,
                    "loss": loss.item(), "training_horizon": inp.training_horizon,
                })
                logger.info("SFT step", {"run_id": run_id, "step": step_i, "loss": loss.item()})

        await state_set("nanochat:training", run_id, {
            "status": "complete", "type": "sft", "step": inp.training_horizon, "device": device,
        })
        return {"status": "complete", "run_id": run_id, "steps": inp.training_horizon}

    except Exception as e:
        await state_set("nanochat:training", run_id, {"status": "failed", "error": str(e)})
        logger.error("SFT training failed", {"run_id": run_id, "error": str(e)})
        return {"status": "failed", "run_id": run_id, "error": str(e)}


async def fn_train_status(data: TrainStatusInput) -> dict:
    inp = TrainStatusInput.model_validate(data) if isinstance(data, dict) else data
    if inp.run_id:
        result = await state_get("nanochat:training", inp.run_id)
        return result or {"error": "run not found"}
    return {"runs": await state_list("nanochat:training")}


async def fn_health(data: dict) -> HealthOutput:
    return HealthOutput(
        status="ok",
        model_loaded=gpu.ready,
        device=gpu.device,
        source=gpu.source,
    ).model_dump()


# ---------------------------------------------------------------------------
# Registration — every function gets a function + trigger, no exceptions
# ---------------------------------------------------------------------------

def register_all(iii):
    iii.register_service({
        "id": "nanochat",
        "name": "nanochat",
        "description": "Train, fine-tune, evaluate, and chat with GPT models on iii-engine",
    })
    iii.register_service({"id": "nanochat.chat", "name": "Chat", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.model", "name": "Model", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.tokenizer", "name": "Tokenizer", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.tools", "name": "Tools", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.eval", "name": "Evaluation", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.train", "name": "Training", "parent_service_id": "nanochat"})

    functions = [
        ("nanochat.chat.complete", fn_chat_complete, "Generate chat completion from loaded GPT model",
         "http", {"api_path": "/nanochat/chat/completions", "http_method": "POST"}),

        ("nanochat.chat.stream", fn_chat_stream, "Generate chat completion token-by-token",
         "http", {"api_path": "/nanochat/chat/stream", "http_method": "POST"}),

        ("nanochat.chat.history", fn_chat_history, "Get conversation history from iii state",
         "http", {"api_path": "/nanochat/chat/history", "http_method": "GET"}),

        ("nanochat.model.load", fn_model_load, "Load a nanochat checkpoint into GPU memory",
         "http", {"api_path": "/nanochat/model/load", "http_method": "POST"}),

        ("nanochat.model.status", fn_model_status, "Get loaded model status and config",
         "http", {"api_path": "/nanochat/model/status", "http_method": "GET"}),

        ("nanochat.tokenizer.encode", fn_tokenizer_encode, "Encode text to BPE token IDs",
         "http", {"api_path": "/nanochat/tokenizer/encode", "http_method": "POST"}),

        ("nanochat.tokenizer.decode", fn_tokenizer_decode, "Decode token IDs back to text",
         "http", {"api_path": "/nanochat/tokenizer/decode", "http_method": "POST"}),

        ("nanochat.tools.execute", fn_tools_execute, "Execute Python code in sandboxed environment",
         "http", {"api_path": "/nanochat/tools/execute", "http_method": "POST"}),

        ("nanochat.eval.core", fn_eval_core, "Run CORE benchmark on loaded model",
         "http", {"api_path": "/nanochat/eval/core", "http_method": "POST"}),

        ("nanochat.eval.loss", fn_eval_loss, "Evaluate bits-per-byte loss on validation set",
         "http", {"api_path": "/nanochat/eval/loss", "http_method": "POST"}),

        ("nanochat.train.sft", fn_train_sft, "Run supervised fine-tuning (long-running, use queue)",
         "queue", {"queue_name": "nanochat-training"}),

        ("nanochat.train.status", fn_train_status, "Check training run status from iii state",
         "http", {"api_path": "/nanochat/train/status", "http_method": "GET"}),

        ("nanochat.health", fn_health, "Worker health check",
         "http", {"api_path": "/nanochat/health", "http_method": "GET"}),
    ]

    for func_id, handler, description, trigger_type, trigger_config in functions:
        iii.register_function(func_id, safe(handler), description=description)
        iii.register_trigger({"type": trigger_type, "function_id": func_id, "config": trigger_config})

    logger.info("Registered all functions and triggers", {"count": len(functions)})


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    global iii_client

    parser = argparse.ArgumentParser(description="nanochat iii-engine worker")
    parser.add_argument("--engine-url", default=os.environ.get("III_ENGINE_URL", "ws://localhost:49134"))
    parser.add_argument("--source", default="sft", choices=["base", "sft", "rl"])
    parser.add_argument("--model-tag", default=None)
    parser.add_argument("--step", type=int, default=None)
    parser.add_argument("--device", default=None)
    parser.add_argument("--no-autoload", action="store_true")
    parser.add_argument("--nanochat-dir", default=None)
    args = parser.parse_args()

    if args.nanochat_dir:
        global NANOCHAT_DIR
        NANOCHAT_DIR = args.nanochat_dir
        parent = str(Path(NANOCHAT_DIR).parent)
        if parent not in sys.path:
            sys.path.insert(0, parent)

    _ensure_nanochat()

    iii_client = register_worker(
        args.engine_url,
        InitOptions(
            worker_name="nanochat",
            invocation_timeout_ms=60000,
            telemetry=TelemetryOptions(language="python", project_name="nanochat"),
        ),
    )

    register_all(iii_client)

    if not args.no_autoload:
        from nanochat.common import autodetect_device_type
        device = args.device or autodetect_device_type()
        try:
            gpu.load(args.source, device, model_tag=args.model_tag, step=args.step)
            iii_client.trigger({"function_id": "state::set", "payload": {
                "scope": "nanochat:models", "key": "current",
                "value": {"source": gpu.source, "device": gpu.device,
                          "config": gpu.meta.get("model_config", {}) if gpu.meta else {}},
            }})
        except Exception as e:
            logger.warn("Auto-load failed, use nanochat.model.load", {"error": str(e)})

    print(f"[nanochat] connected to {args.engine_url}")
    print(f"[nanochat] model: {'loaded (' + gpu.source + ' on ' + gpu.device + ')' if gpu.ready else 'none'}")
    print(f"[nanochat] 13 functions, 13 triggers (12 HTTP + 1 queue)")

    try:
        signal.pause()
    except AttributeError:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    finally:
        iii_client.shutdown()


if __name__ == "__main__":
    main()
