"""
nanochat worker for iii-engine (v0.10.0 SDK).

Covers the full nanochat pipeline: tokenizer training, base pretraining,
supervised fine-tuning, RL fine-tuning, CORE/BPB/ChatCORE evaluation,
inference with tool use, and checkpoint management.

Every capability is a registered function + trigger. Pydantic type hints
on every handler for auto schema extraction. Async handlers for state I/O.
safe() wrapper on every handler for zero-crash guarantee.

Usage:
    python worker.py --no-autoload
    python worker.py --source sft --device cuda
"""

import argparse
import contextlib
import io
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

NANOCHAT_DIR = os.environ.get("NANOCHAT_DIR", str(Path(__file__).parent / "nanochat-upstream" / "nanochat"))

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
    async def wrapper(data):
        try:
            return await fn(data)
        except Exception as e:
            logger.error(f"Handler {fn.__name__} failed", {"error": str(e), "traceback": traceback.format_exc()})
            return {"error": str(e)}
    wrapper.__name__ = fn.__name__
    wrapper.__annotations__ = fn.__annotations__
    return wrapper


# ---------------------------------------------------------------------------
# Pydantic schemas
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

class ModelSampleInput(BaseModel):
    prompt: str = ""
    max_tokens: int = 256
    temperature: float = 0.8
    top_k: int = 50
    num_samples: int = 1

class TokenizeInput(BaseModel):
    text: str | list[str]

class DecodeInput(BaseModel):
    tokens: list[int]

class ExecuteCodeInput(BaseModel):
    code: str
    timeout: float = 5.0

class TrainTokenizerInput(BaseModel):
    max_chars: int = 2_000_000_000
    doc_cap: int = 10_000
    vocab_size: int = 32_768

class TrainBaseInput(BaseModel):
    depth: int = 20
    aspect_ratio: int = 64
    head_dim: int = 128
    max_seq_len: int = 2048
    window_pattern: str = "SSSL"
    target_param_data_ratio: float = 12.0
    num_iterations: int = -1
    device_batch_size: int = 32
    warmup_steps: int = 40
    warmdown_ratio: float = 0.65
    eval_every: int = 250
    save_every: int = -1
    device: str | None = None
    run_name: str = "base"
    model_tag: str | None = None
    fp8: bool = False

class TrainSFTInput(BaseModel):
    source: str = "base"
    model_tag: str | None = None
    step: int | None = None
    num_iterations: int = -1
    device_batch_size: int | None = None
    mmlu_epochs: int = 3
    gsm8k_epochs: int = 4
    eval_every: int = 200
    save_every: int = -1
    warmdown_ratio: float = 0.5
    device: str | None = None
    run_name: str = "sft"

class TrainRLInput(BaseModel):
    source: str = "sft"
    model_tag: str | None = None
    step: int | None = None
    num_epochs: int = 1
    examples_per_step: int = 16
    num_samples: int = 16
    max_new_tokens: int = 256
    temperature: float = 1.0
    top_k: int = 50
    device_batch_size: int = 8
    eval_every: int = 60
    save_every: int = 60
    device: str | None = None
    run_name: str = "rl"

class TrainStatusInput(BaseModel):
    run_id: str | None = None

class EvalCoreInput(BaseModel):
    max_per_task: int = -1

class EvalLossInput(BaseModel):
    split: str = "val"
    steps: int = 50
    device_batch_size: int = 4

class EvalChatInput(BaseModel):
    task_name: str | None = None
    temperature: float = 0.0
    max_new_tokens: int = 512
    num_samples: int = 1
    top_k: int = 50
    batch_size: int = 8
    max_problems: int | None = None

class CheckpointSaveInput(BaseModel):
    tag: str | None = None
    step: int | None = None

class CheckpointListInput(BaseModel):
    source: str = "sft"

class HealthOutput(BaseModel):
    status: str
    model_loaded: bool
    device: str | None = None
    source: str | None = None
    worker: str = "iii-nanochat"


# ---------------------------------------------------------------------------
# GPU state
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

    def load(self, source, device, model_tag=None, step=None):
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

    def snapshot(self):
        """Return a consistent snapshot of (model, tokenizer, engine, meta, source, device) under lock."""
        with self._lock:
            return self.model, self.tokenizer, self.engine, self.meta, self.source, self.device

    @property
    def ready(self):
        return self.engine is not None

gpu = GPUState()


# ---------------------------------------------------------------------------
# Async state helpers
# ---------------------------------------------------------------------------

async def state_get(scope, key):
    return await iii_client.trigger_async({"function_id": "state::get", "payload": {"scope": scope, "key": key}})

async def state_set(scope, key, value):
    return await iii_client.trigger_async({"function_id": "state::set", "payload": {"scope": scope, "key": key, "value": value}})

async def state_list(scope):
    return await iii_client.trigger_async({"function_id": "state::list", "payload": {"scope": scope}})


# ---------------------------------------------------------------------------
# Chat handlers
# ---------------------------------------------------------------------------

async def fn_chat_complete(data: ChatCompleteInput) -> ChatCompleteOutput:
    _ensure_nanochat()
    import torch
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    model, tokenizer, engine, _meta, source, _device = gpu.snapshot()
    inp = ChatCompleteInput.model_validate(data) if isinstance(data, dict) else data
    session_id = inp.session_id or str(uuid.uuid4())
    conversation = [{"role": m.role, "content": m.content} for m in inp.messages]

    if hasattr(tokenizer, "render_conversation"):
        tokens, _mask = tokenizer.render_conversation(conversation, max_tokens=model.config.sequence_len)
    else:
        tokens = tokenizer.render_for_completion(conversation)

    with torch.no_grad():
        results, _masks = engine.generate_batch(
            [tokens], num_samples=1,
            max_tokens=inp.max_tokens, temperature=inp.temperature, top_k=inp.top_k,
        )

    generated_ids = results[0]
    text = tokenizer.decode(generated_ids)
    if "<|assistant_end|>" in text:
        text = text[:text.index("<|assistant_end|>")]

    conversation.append({"role": "assistant", "content": text.strip()})
    await state_set("nanochat:sessions", session_id, {
        "messages": conversation, "model": source, "tokens_generated": len(generated_ids),
    })
    logger.info("Chat completion", {"session_id": session_id, "tokens": len(generated_ids)})
    return ChatCompleteOutput(content=text.strip(), tokens_generated=len(generated_ids), session_id=session_id).model_dump()


async def fn_chat_stream(data: ChatCompleteInput) -> ChatCompleteOutput:
    _ensure_nanochat()
    import torch
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    model, tokenizer, engine, _meta, source, _device = gpu.snapshot()
    inp = ChatCompleteInput.model_validate(data) if isinstance(data, dict) else data
    session_id = inp.session_id or str(uuid.uuid4())
    conversation = [{"role": m.role, "content": m.content} for m in inp.messages]

    if hasattr(tokenizer, "render_conversation"):
        tokens, _mask = tokenizer.render_conversation(conversation, max_tokens=model.config.sequence_len)
    else:
        tokens = tokenizer.render_for_completion(conversation)

    chunks = []
    with torch.no_grad():
        for token_col, _token_masks in engine.generate(
            [tokens], num_samples=1,
            max_tokens=inp.max_tokens, temperature=inp.temperature, top_k=inp.top_k,
        ):
            token_id = token_col[0].item()
            piece = tokenizer.decode([token_id])
            if "<|assistant_end|>" in piece:
                break
            chunks.append(piece)

    full_text = "".join(chunks)
    conversation.append({"role": "assistant", "content": full_text.strip()})
    await state_set("nanochat:sessions", session_id, {
        "messages": conversation, "model": source, "tokens_generated": len(chunks),
    })
    return ChatCompleteOutput(content=full_text.strip(), tokens_generated=len(chunks), session_id=session_id).model_dump()


async def fn_chat_history(data: ChatHistoryInput) -> dict:
    inp = ChatHistoryInput.model_validate(data) if isinstance(data, dict) else data
    if not inp.session_id:
        return {"sessions": await state_list("nanochat:sessions")}
    return {"session_id": inp.session_id, "data": await state_get("nanochat:sessions", inp.session_id)}


# ---------------------------------------------------------------------------
# Model handlers
# ---------------------------------------------------------------------------

async def fn_model_load(data: ModelLoadInput) -> ModelStatusOutput:
    _ensure_nanochat()
    from nanochat.common import autodetect_device_type
    inp = ModelLoadInput.model_validate(data) if isinstance(data, dict) else data
    device = inp.device or autodetect_device_type()
    gpu.load(inp.source, device, model_tag=inp.model_tag, step=inp.step)
    await state_set("nanochat:models", "current", {
        "source": gpu.source, "model_tag": gpu.model_tag, "device": gpu.device,
        "config": gpu.meta.get("model_config", {}) if gpu.meta else {},
        "parameters": sum(p.numel() for p in gpu.model.parameters()),
    })
    logger.info("Model loaded", {"source": inp.source, "device": device})
    return await fn_model_status({})


async def fn_model_status(data: dict) -> ModelStatusOutput:
    if not gpu.ready:
        return ModelStatusOutput(loaded=False).model_dump()
    model, _tok, _eng, meta, source, device = gpu.snapshot()
    config = meta.get("model_config", {}) if meta else {}
    return ModelStatusOutput(
        loaded=True, source=source, model_tag=gpu.model_tag, device=device,
        n_layer=config.get("n_layer"), n_embd=config.get("n_embd"),
        vocab_size=config.get("vocab_size"), sequence_len=config.get("sequence_len"),
        parameters=sum(p.numel() for p in model.parameters()) if model else None,
    ).model_dump()


async def fn_model_sample(data: ModelSampleInput) -> dict:
    _ensure_nanochat()
    import torch
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    _model, tokenizer, engine, _meta, _source, _device = gpu.snapshot()
    inp = ModelSampleInput.model_validate(data) if isinstance(data, dict) else data
    bos = tokenizer.get_bos_token_id()
    tokens = [bos] + tokenizer.encode(inp.prompt) if inp.prompt else [bos]

    samples = []
    with torch.no_grad():
        results, _masks = engine.generate_batch(
            [tokens], num_samples=inp.num_samples,
            max_tokens=inp.max_tokens, temperature=inp.temperature, top_k=inp.top_k,
        )
    for result_ids in results:
        text = tokenizer.decode(result_ids)
        if "<|assistant_end|>" in text:
            text = text[:text.index("<|assistant_end|>")]
        samples.append(text)

    return {"samples": samples, "num_samples": len(samples)}


# ---------------------------------------------------------------------------
# Tokenizer handlers
# ---------------------------------------------------------------------------

async def fn_tokenizer_encode(data: TokenizeInput) -> dict:
    _ensure_nanochat()
    from nanochat.tokenizer import get_tokenizer
    inp = TokenizeInput.model_validate(data) if isinstance(data, dict) else data
    tokenizer = gpu.tokenizer or get_tokenizer()
    bos = tokenizer.get_bos_token_id()
    encoded = tokenizer.encode(inp.text, prepend=bos)
    count = sum(len(t) for t in encoded) if isinstance(inp.text, list) else len(encoded)
    return {"tokens": encoded, "count": count}


async def fn_tokenizer_decode(data: DecodeInput) -> dict:
    _ensure_nanochat()
    from nanochat.tokenizer import get_tokenizer
    inp = DecodeInput.model_validate(data) if isinstance(data, dict) else data
    tokenizer = gpu.tokenizer or get_tokenizer()
    return {"text": tokenizer.decode(inp.tokens)}


# ---------------------------------------------------------------------------
# Tools handler
# ---------------------------------------------------------------------------

async def fn_tools_execute(data: ExecuteCodeInput) -> dict:
    inp = ExecuteCodeInput.model_validate(data) if isinstance(data, dict) else data
    stdout_buf, stderr_buf = io.StringIO(), io.StringIO()
    try:
        with contextlib.redirect_stdout(stdout_buf), contextlib.redirect_stderr(stderr_buf):
            exec(inp.code, {"__builtins__": __builtins__}, {})
        return {"success": True, "stdout": stdout_buf.getvalue(), "stderr": stderr_buf.getvalue(), "error": None}
    except Exception as e:
        return {"success": False, "stdout": stdout_buf.getvalue(), "stderr": stderr_buf.getvalue(), "error": str(e)}


# ---------------------------------------------------------------------------
# Subprocess runner with real-time stdout parsing -> iii state
# ---------------------------------------------------------------------------

def _nanochat_repo_dir() -> str:
    """Root of the nanochat repo (contains scripts/, tasks/, nanochat/)."""
    return str(Path(NANOCHAT_DIR).parent)


def _parse_training_line(line: str) -> dict | None:
    """Parse nanochat stdout into structured metrics. Returns None for non-metric lines."""
    import re

    # base_train / chat_sft step line:
    # "step 00100/05000 (2.00%) | loss: 4.123456 | lrm: 0.50 | dt: 123.45ms | tok/sec: 123,456 | bf16_mfu: 0.45"
    m = re.match(r"step\s+(\d+)(?:/(\d+))?\s+\((\d+\.\d+)%\)\s*\|(.+)", line)
    if m:
        metrics = {"step": int(m.group(1)), "pct": float(m.group(3))}
        if m.group(2):
            metrics["total_steps"] = int(m.group(2))
        for pair in m.group(4).split("|"):
            pair = pair.strip()
            kv = pair.split(":")
            if len(kv) == 2:
                key = kv[0].strip().replace(" ", "_")
                val = kv[1].strip().replace(",", "").rstrip("ms").rstrip("m")
                try:
                    metrics[key] = float(val)
                except ValueError:
                    metrics[key] = val
        return metrics

    # Validation BPB: "Step 00250 | Validation bpb: 1.234567"
    m = re.match(r"Step\s+(\d+)\s+\|\s+Validation bpb:\s+(\S+)", line)
    if m:
        return {"step": int(m.group(1)), "val_bpb": float(m.group(2)), "event": "eval_bpb"}

    # CORE metric: "Step 00250 | CORE metric: 0.1234"
    m = re.match(r"Step\s+(\d+)\s+\|\s+CORE metric:\s+(\S+)", line)
    if m:
        return {"step": int(m.group(1)), "core_metric": float(m.group(2)), "event": "eval_core"}

    # ChatCORE: "Step 00200 | ChatCORE: 0.1234 | ChatCORE_cat: 0.2345"
    m = re.match(r"Step\s+(\d+)\s+\|\s+ChatCORE:\s+(\S+)\s+\|\s+ChatCORE_cat:\s+(\S+)", line)
    if m:
        return {"step": int(m.group(1)), "chatcore": float(m.group(2)), "chatcore_cat": float(m.group(3)), "event": "eval_chatcore"}

    # RL step: "Step 10/100 | Average reward: 0.5 | Average sequence length: 128.00"
    m = re.match(r"Step\s+(\d+)/(\d+)\s+\|\s+Average reward:\s+(\S+)\s+\|\s+Average sequence length:\s+(\S+)", line)
    if m:
        return {"step": int(m.group(1)), "total_steps": int(m.group(2)), "avg_reward": float(m.group(3)), "avg_seq_len": float(m.group(4))}

    # RL pass@k: "Step 10 | pass@1: 0.25, pass@16: 0.75"
    m = re.match(r"Step\s+(\d+)\s+\|\s+(pass@.+)", line)
    if m:
        metrics = {"step": int(m.group(1)), "event": "eval_passk"}
        for pair in m.group(2).split(","):
            kv = pair.strip().split(":")
            if len(kv) == 2:
                metrics[kv[0].strip()] = float(kv[1].strip())
        return metrics

    return None


def _run_subprocess_blocking(module: str, args: list[str], run_id: str, train_type: str,
                             base_state: dict, on_metrics) -> dict:
    """Blocking subprocess runner. Called from a thread via asyncio.to_thread."""
    import subprocess

    cmd = [sys.executable, "-m", module] + args

    proc = subprocess.Popen(
        cmd, cwd=_nanochat_repo_dir(),
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, bufsize=1,
    )

    last_metrics = {}
    output_tail = []

    for line in proc.stdout:
        line = line.rstrip()
        output_tail.append(line)
        if len(output_tail) > 200:
            output_tail = output_tail[-100:]

        metrics = _parse_training_line(line)
        if metrics:
            last_metrics.update(metrics)
            on_metrics(run_id, {**base_state, **last_metrics}, metrics)

    proc.wait()

    status = "complete" if proc.returncode == 0 else "failed"
    return {
        "status": status, "returncode": proc.returncode,
        "last_metrics": last_metrics,
        "output_tail": "\n".join(output_tail[-50:]),
    }


async def _run_training(module: str, args: list[str], run_id: str, train_type: str, extra_state: dict | None = None) -> dict:
    """Run a nanochat training script in a thread, parse stdout, push metrics to iii state in real-time."""
    import asyncio

    base_state = {"status": "running", "type": train_type, **(extra_state or {})}
    await state_set("nanochat:training", run_id, base_state)
    logger.info(f"Running: {module}", {"run_id": run_id, "type": train_type})

    def on_metrics(rid, state, metrics):
        iii_client.trigger({"function_id": "state::set", "payload": {
            "scope": "nanochat:training", "key": rid, "value": state,
        }})
        event = metrics.get("event")
        if event:
            iii_client.trigger({"function_id": "state::set", "payload": {
                "scope": "nanochat:evals",
                "key": f"{train_type}-{event}-{metrics.get('step', 0)}",
                "value": {"type": event, "run_id": rid, **metrics},
            }})

    result = await asyncio.to_thread(
        _run_subprocess_blocking, module, args, run_id, train_type, base_state, on_metrics,
    )

    final_state = {
        **base_state, **result["last_metrics"],
        "status": result["status"], "returncode": result["returncode"],
        "output_tail": result["output_tail"],
    }
    await state_set("nanochat:training", run_id, final_state)
    logger.info(f"{train_type} training {result['status']}", {"run_id": run_id, "returncode": result["returncode"]})

    return {"status": result["status"], "run_id": run_id, "returncode": result["returncode"], **result["last_metrics"]}


# ---------------------------------------------------------------------------
# Training handlers (all queued, run actual nanochat scripts with live state)
# ---------------------------------------------------------------------------

async def fn_train_tokenizer(data: TrainTokenizerInput) -> dict:
    inp = TrainTokenizerInput.model_validate(data) if isinstance(data, dict) else data
    run_id = str(uuid.uuid4())[:8]

    args = [
        "--max-chars", str(inp.max_chars),
        "--doc-cap", str(inp.doc_cap),
        "--vocab-size", str(inp.vocab_size),
    ]

    return await _run_training("scripts.tok_train", args, run_id, "tokenizer",
                               {"vocab_size": inp.vocab_size})


async def fn_train_base(data: TrainBaseInput) -> dict:
    inp = TrainBaseInput.model_validate(data) if isinstance(data, dict) else data
    run_id = str(uuid.uuid4())[:8]

    args = [
        "--run", inp.run_name,
        "--depth", str(inp.depth),
        "--aspect-ratio", str(inp.aspect_ratio),
        "--head-dim", str(inp.head_dim),
        "--max-seq-len", str(inp.max_seq_len),
        "--window-pattern", inp.window_pattern,
        "--target-param-data-ratio", str(inp.target_param_data_ratio),
        "--device-batch-size", str(inp.device_batch_size),
        "--warmup-steps", str(inp.warmup_steps),
        "--warmdown-ratio", str(inp.warmdown_ratio),
        "--eval-every", str(inp.eval_every),
    ]
    if inp.num_iterations > 0:
        args += ["--num-iterations", str(inp.num_iterations)]
    if inp.save_every > 0:
        args += ["--save-every", str(inp.save_every)]
    if inp.device:
        args += ["--device-type", inp.device]
    if inp.model_tag:
        args += ["--model-tag", inp.model_tag]
    if inp.fp8:
        args += ["--fp8"]

    return await _run_training("scripts.base_train", args, run_id, "base",
                               {"depth": inp.depth, "model_tag": inp.model_tag or f"d{inp.depth}"})


async def fn_train_sft(data: TrainSFTInput) -> dict:
    inp = TrainSFTInput.model_validate(data) if isinstance(data, dict) else data
    run_id = str(uuid.uuid4())[:8]

    args = [
        "--run", inp.run_name,
        "--mmlu-epochs", str(inp.mmlu_epochs),
        "--gsm8k-epochs", str(inp.gsm8k_epochs),
        "--eval-every", str(inp.eval_every),
        "--warmdown-ratio", str(inp.warmdown_ratio),
    ]
    if inp.num_iterations > 0:
        args += ["--num-iterations", str(inp.num_iterations)]
    if inp.device_batch_size:
        args += ["--device-batch-size", str(inp.device_batch_size)]
    if inp.save_every > 0:
        args += ["--save-every", str(inp.save_every)]
    if inp.device:
        args += ["--device-type", inp.device]
    if inp.model_tag:
        args += ["--model-tag", inp.model_tag]
    if inp.step:
        args += ["--model-step", str(inp.step)]

    return await _run_training("scripts.chat_sft", args, run_id, "sft",
                               {"source": inp.source})


async def fn_train_rl(data: TrainRLInput) -> dict:
    inp = TrainRLInput.model_validate(data) if isinstance(data, dict) else data
    run_id = str(uuid.uuid4())[:8]

    args = [
        "--run", inp.run_name,
        "--num-epochs", str(inp.num_epochs),
        "--examples-per-step", str(inp.examples_per_step),
        "--num-samples", str(inp.num_samples),
        "--max-new-tokens", str(inp.max_new_tokens),
        "--temperature", str(inp.temperature),
        "--top-k", str(inp.top_k),
        "--device-batch-size", str(inp.device_batch_size),
        "--eval-every", str(inp.eval_every),
        "--save-every", str(inp.save_every),
    ]
    if inp.device:
        args += ["--device-type", inp.device]
    if inp.model_tag:
        args += ["--model-tag", inp.model_tag]
    if inp.step:
        args += ["--model-step", str(inp.step)]

    return await _run_training("scripts.chat_rl", args, run_id, "rl")


async def fn_train_status(data: TrainStatusInput) -> dict:
    inp = TrainStatusInput.model_validate(data) if isinstance(data, dict) else data
    if inp.run_id:
        return await state_get("nanochat:training", inp.run_id) or {"error": "run not found"}
    return {"runs": await state_list("nanochat:training")}


# ---------------------------------------------------------------------------
# Evaluation handlers (import and call real nanochat functions)
# ---------------------------------------------------------------------------

async def fn_eval_core(data: EvalCoreInput) -> dict:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")
    _ensure_nanochat()

    model, tokenizer, _engine, _meta, source, device = gpu.snapshot()
    inp = EvalCoreInput.model_validate(data) if isinstance(data, dict) else data

    scripts_dir = os.path.join(_nanochat_repo_dir(), "scripts")
    if scripts_dir not in sys.path:
        sys.path.insert(0, scripts_dir)
    from base_eval import evaluate_core

    dev = model.get_device() if hasattr(model, "get_device") else device
    result = evaluate_core(model, tokenizer, dev, max_per_task=inp.max_per_task)

    await state_set("nanochat:evals", f"core-{int(time.time())}", {
        "type": "core", "core_metric": result["core_metric"],
        "results": result["results"], "model": source,
    })

    return {
        "core_metric": result["core_metric"],
        "results": result.get("results", {}),
        "centered_results": result.get("centered_results", {}),
    }


async def fn_eval_loss(data: EvalLossInput) -> dict:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")
    _ensure_nanochat()
    from nanochat.loss_eval import evaluate_bpb
    from nanochat.tokenizer import get_token_bytes
    from nanochat.dataloader import tokenizing_distributed_data_loader_bos_bestfit

    model, tokenizer, _engine, _meta, source, device = gpu.snapshot()
    inp = EvalLossInput.model_validate(data) if isinstance(data, dict) else data
    token_bytes = get_token_bytes(device)
    B, T = inp.device_batch_size, model.config.sequence_len
    batches = tokenizing_distributed_data_loader_bos_bestfit(tokenizer, B, T, inp.split, device=device)
    bpb = evaluate_bpb(model, batches, steps=inp.steps, token_bytes=token_bytes)

    await state_set("nanochat:evals", f"loss-{int(time.time())}", {
        "type": "bpb", "bpb": bpb, "split": inp.split, "model": source,
    })
    return {"bits_per_byte": bpb, "split": inp.split, "model": source}


async def fn_eval_chat(data: EvalChatInput) -> dict:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")
    _ensure_nanochat()

    model, tokenizer, engine, _meta, source, _device = gpu.snapshot()
    inp = EvalChatInput.model_validate(data) if isinstance(data, dict) else data

    scripts_dir = os.path.join(_nanochat_repo_dir(), "scripts")
    tasks_dir = os.path.join(_nanochat_repo_dir(), "tasks")
    if scripts_dir not in sys.path:
        sys.path.insert(0, scripts_dir)
    if tasks_dir not in sys.path:
        sys.path.insert(0, tasks_dir)

    from chat_eval import run_chat_eval

    available_tasks = ["GSM8K", "MMLU", "ARC-Easy", "ARC-Challenge", "HumanEval", "SpellingBee"]

    if inp.task_name:
        task_names = [inp.task_name]
    else:
        task_names = available_tasks

    results = {}
    for task_name in task_names:
        try:
            acc = run_chat_eval(
                task_name, model, tokenizer, engine,
                batch_size=inp.batch_size, num_samples=inp.num_samples,
                max_new_tokens=inp.max_new_tokens, temperature=inp.temperature,
                top_k=inp.top_k, max_problems=inp.max_problems,
            )
            results[task_name] = acc
        except Exception as e:
            results[task_name] = {"error": str(e)}

    await state_set("nanochat:evals", f"chat-{int(time.time())}", {
        "type": "chat", "results": results, "model": source,
    })
    return {"results": results, "model": source}


# ---------------------------------------------------------------------------
# Checkpoint handlers
# ---------------------------------------------------------------------------

async def fn_checkpoint_save(data: CheckpointSaveInput) -> dict:
    if not gpu.ready:
        raise RuntimeError("No model loaded.")
    _ensure_nanochat()
    from nanochat.checkpoint_manager import save_checkpoint
    from nanochat.common import get_base_dir

    inp = CheckpointSaveInput.model_validate(data) if isinstance(data, dict) else data
    tag = inp.tag or gpu.model_tag or "manual"
    step = inp.step or int(time.time())

    base_dir = get_base_dir()
    phase_dir = {"base": "checkpoints", "sft": "chatsft_checkpoints", "rl": "chatrl_checkpoints"}.get(gpu.source, "checkpoints")
    checkpoint_dir = os.path.join(base_dir, phase_dir, tag)

    model_config = gpu.meta.get("model_config", {}) if gpu.meta else {}
    save_checkpoint(checkpoint_dir, step, gpu.model.state_dict(), None, {
        "step": step, "model_config": model_config,
    })

    await state_set("nanochat:checkpoints", f"{tag}-{step}", {
        "tag": tag, "step": step, "source": gpu.source, "path": checkpoint_dir,
    })
    logger.info("Checkpoint saved", {"tag": tag, "step": step})
    return {"tag": tag, "step": step, "path": checkpoint_dir}


async def fn_checkpoint_list(data: CheckpointListInput) -> dict:
    _ensure_nanochat()
    from nanochat.common import get_base_dir

    inp = CheckpointListInput.model_validate(data) if isinstance(data, dict) else data
    base_dir = get_base_dir()
    phase_dir = {"base": "checkpoints", "sft": "chatsft_checkpoints", "rl": "chatrl_checkpoints"}.get(inp.source, "checkpoints")
    search_dir = os.path.join(base_dir, phase_dir)

    checkpoints = []
    if os.path.exists(search_dir):
        for tag_dir in sorted(os.listdir(search_dir)):
            tag_path = os.path.join(search_dir, tag_dir)
            if os.path.isdir(tag_path):
                steps = sorted([
                    int(f.split("_")[1].split(".")[0])
                    for f in os.listdir(tag_path) if f.startswith("model_") and f.endswith(".pt")
                ])
                checkpoints.append({"tag": tag_dir, "steps": steps, "path": tag_path})

    return {"source": inp.source, "checkpoints": checkpoints}


# ---------------------------------------------------------------------------
# Health
# ---------------------------------------------------------------------------

async def fn_health(data: dict) -> HealthOutput:
    return HealthOutput(
        status="ok", model_loaded=gpu.ready, device=gpu.device, source=gpu.source,
    ).model_dump()


# ---------------------------------------------------------------------------
# Registration
# ---------------------------------------------------------------------------

def register_all(iii):
    iii.register_service({"id": "nanochat", "name": "nanochat", "description": "Full nanochat pipeline on iii-engine"})
    iii.register_service({"id": "nanochat.chat", "name": "Chat", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.model", "name": "Model", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.tokenizer", "name": "Tokenizer", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.tools", "name": "Tools", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.eval", "name": "Evaluation", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.train", "name": "Training", "parent_service_id": "nanochat"})
    iii.register_service({"id": "nanochat.checkpoint", "name": "Checkpoints", "parent_service_id": "nanochat"})

    functions = [
        # Chat
        ("nanochat.chat.complete", fn_chat_complete, "Generate chat completion", "http", {"api_path": "/nanochat/chat/completions", "http_method": "POST"}),
        ("nanochat.chat.stream", fn_chat_stream, "Generate chat completion token-by-token", "http", {"api_path": "/nanochat/chat/stream", "http_method": "POST"}),
        ("nanochat.chat.history", fn_chat_history, "Get conversation history from state", "http", {"api_path": "/nanochat/chat/history", "http_method": "GET"}),
        # Model
        ("nanochat.model.load", fn_model_load, "Load checkpoint into GPU memory", "http", {"api_path": "/nanochat/model/load", "http_method": "POST"}),
        ("nanochat.model.status", fn_model_status, "Get loaded model status and config", "http", {"api_path": "/nanochat/model/status", "http_method": "GET"}),
        ("nanochat.model.sample", fn_model_sample, "Generate raw text samples from loaded model", "http", {"api_path": "/nanochat/model/sample", "http_method": "POST"}),
        # Tokenizer
        ("nanochat.tokenizer.encode", fn_tokenizer_encode, "Encode text to BPE token IDs", "http", {"api_path": "/nanochat/tokenizer/encode", "http_method": "POST"}),
        ("nanochat.tokenizer.decode", fn_tokenizer_decode, "Decode token IDs to text", "http", {"api_path": "/nanochat/tokenizer/decode", "http_method": "POST"}),
        # Tools
        ("nanochat.tools.execute", fn_tools_execute, "Execute Python code (in-process, not sandboxed)", "http", {"api_path": "/nanochat/tools/execute", "http_method": "POST"}),
        # Training (all queued)
        ("nanochat.train.tokenizer", fn_train_tokenizer, "Train BPE tokenizer from dataset", "queue", {"queue_name": "nanochat-training"}),
        ("nanochat.train.base", fn_train_base, "Pretrain base GPT model from scratch", "queue", {"queue_name": "nanochat-training"}),
        ("nanochat.train.sft", fn_train_sft, "Supervised fine-tuning with task mixture", "queue", {"queue_name": "nanochat-training"}),
        ("nanochat.train.rl", fn_train_rl, "RL fine-tuning with GRPO on GSM8K", "queue", {"queue_name": "nanochat-training"}),
        ("nanochat.train.status", fn_train_status, "Check training run status", "http", {"api_path": "/nanochat/train/status", "http_method": "GET"}),
        # Evaluation
        ("nanochat.eval.core", fn_eval_core, "Run CORE benchmark (DCLM)", "http", {"api_path": "/nanochat/eval/core", "http_method": "POST"}),
        ("nanochat.eval.loss", fn_eval_loss, "Evaluate bits-per-byte on validation set", "http", {"api_path": "/nanochat/eval/loss", "http_method": "POST"}),
        ("nanochat.eval.chat", fn_eval_chat, "Run ChatCORE evaluation (GSM8K, MMLU, ARC)", "http", {"api_path": "/nanochat/eval/chat", "http_method": "POST"}),
        # Checkpoints
        ("nanochat.checkpoint.save", fn_checkpoint_save, "Save current model to disk", "http", {"api_path": "/nanochat/checkpoint/save", "http_method": "POST"}),
        ("nanochat.checkpoint.list", fn_checkpoint_list, "List available checkpoints", "http", {"api_path": "/nanochat/checkpoint/list", "http_method": "GET"}),
        # Health
        ("nanochat.health", fn_health, "Worker health check", "http", {"api_path": "/nanochat/health", "http_method": "GET"}),
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
            invocation_timeout_ms=600000,
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

    n_funcs = 20
    print(f"[nanochat] connected to {args.engine_url}")
    print(f"[nanochat] model: {'loaded (' + gpu.source + ' on ' + gpu.device + ')' if gpu.ready else 'none'}")
    print(f"[nanochat] {n_funcs} functions, {n_funcs} triggers (16 HTTP + 4 queue)")

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
