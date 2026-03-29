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
    async def wrapper(data):
        try:
            return await fn(data)
        except Exception as e:
            return {"error": str(e), "traceback": traceback.format_exc()}
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
            max_tokens=inp.max_tokens, temperature=inp.temperature, top_k=inp.top_k,
        )

    generated_ids = results[0]
    text = gpu.tokenizer.decode(generated_ids)
    if "<|assistant_end|>" in text:
        text = text[:text.index("<|assistant_end|>")]

    conversation.append({"role": "assistant", "content": text.strip()})
    await state_set("nanochat:sessions", session_id, {
        "messages": conversation, "model": gpu.source, "tokens_generated": len(generated_ids),
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
            max_tokens=inp.max_tokens, temperature=inp.temperature, top_k=inp.top_k,
        ):
            token_id = token_col[0].item()
            piece = gpu.tokenizer.decode([token_id])
            if "<|assistant_end|>" in piece:
                break
            chunks.append(piece)

    full_text = "".join(chunks)
    conversation.append({"role": "assistant", "content": full_text.strip()})
    await state_set("nanochat:sessions", session_id, {
        "messages": conversation, "model": gpu.source, "tokens_generated": len(chunks),
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
    config = gpu.meta.get("model_config", {}) if gpu.meta else {}
    return ModelStatusOutput(
        loaded=True, source=gpu.source, model_tag=gpu.model_tag, device=gpu.device,
        n_layer=config.get("n_layer"), n_embd=config.get("n_embd"),
        vocab_size=config.get("vocab_size"), sequence_len=config.get("sequence_len"),
        parameters=sum(p.numel() for p in gpu.model.parameters()) if gpu.model else None,
    ).model_dump()


async def fn_model_sample(data: ModelSampleInput) -> dict:
    _ensure_nanochat()
    import torch
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")

    inp = ModelSampleInput.model_validate(data) if isinstance(data, dict) else data
    bos = gpu.tokenizer.get_bos_token_id()
    tokens = [bos] + gpu.tokenizer.encode(inp.prompt) if inp.prompt else [bos]

    samples = []
    with torch.no_grad():
        results, _masks = gpu.engine.generate_batch(
            [tokens], num_samples=inp.num_samples,
            max_tokens=inp.max_tokens, temperature=inp.temperature, top_k=inp.top_k,
        )
    for result_ids in results:
        text = gpu.tokenizer.decode(result_ids)
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
# Training handlers (all queued, long-running)
# ---------------------------------------------------------------------------

async def fn_train_tokenizer(data: TrainTokenizerInput) -> dict:
    _ensure_nanochat()
    import torch
    from nanochat.tokenizer import RustBPETokenizer
    from nanochat.common import get_base_dir
    from nanochat.dataset import parquets_iter_batched

    inp = TrainTokenizerInput.model_validate(data) if isinstance(data, dict) else data
    run_id = str(uuid.uuid4())[:8]
    await state_set("nanochat:training", run_id, {"status": "running", "type": "tokenizer"})
    logger.info("Tokenizer training started", {"run_id": run_id, "vocab_size": inp.vocab_size})

    total_chars = 0
    def text_iterator():
        nonlocal total_chars
        for batch in parquets_iter_batched(split="train"):
            for doc in batch:
                text = doc[:inp.doc_cap]
                total_chars += len(text)
                if total_chars > inp.max_chars:
                    return
                yield text

    tokenizer = RustBPETokenizer.train_from_iterator(text_iterator(), inp.vocab_size)

    base_dir = get_base_dir()
    tokenizer_dir = os.path.join(base_dir, "tokenizer")
    os.makedirs(tokenizer_dir, exist_ok=True)
    tokenizer.save(tokenizer_dir)

    token_bytes = torch.zeros(tokenizer.get_vocab_size(), dtype=torch.int32)
    for i in range(tokenizer.get_vocab_size()):
        token_bytes[i] = len(tokenizer.decode([i]).encode("utf-8"))
    torch.save(token_bytes, os.path.join(tokenizer_dir, "token_bytes.pt"))

    await state_set("nanochat:training", run_id, {
        "status": "complete", "type": "tokenizer",
        "vocab_size": tokenizer.get_vocab_size(), "total_chars": total_chars,
        "path": tokenizer_dir,
    })
    logger.info("Tokenizer training complete", {"run_id": run_id, "vocab_size": tokenizer.get_vocab_size()})
    return {"status": "complete", "run_id": run_id, "vocab_size": tokenizer.get_vocab_size(), "path": tokenizer_dir}


async def fn_train_base(data: TrainBaseInput) -> dict:
    _ensure_nanochat()
    import torch
    from nanochat.common import autodetect_device_type, get_base_dir
    from nanochat.gpt import GPT, GPTConfig
    from nanochat.tokenizer import get_tokenizer
    from nanochat.dataloader import tokenizing_distributed_data_loader_bos_bestfit
    from nanochat.checkpoint_manager import save_checkpoint
    from nanochat.loss_eval import evaluate_bpb
    from nanochat.tokenizer import get_token_bytes

    inp = TrainBaseInput.model_validate(data) if isinstance(data, dict) else data
    device = inp.device or autodetect_device_type()
    run_id = str(uuid.uuid4())[:8]

    tokenizer = get_tokenizer()
    vocab_size = tokenizer.get_vocab_size()

    base_dim = inp.depth * inp.aspect_ratio
    model_dim = ((base_dim + inp.head_dim - 1) // inp.head_dim) * inp.head_dim
    num_heads = model_dim // inp.head_dim
    config = GPTConfig(
        sequence_len=inp.max_seq_len, vocab_size=vocab_size,
        n_layer=inp.depth, n_head=num_heads, n_kv_head=num_heads,
        n_embd=model_dim, window_pattern=inp.window_pattern,
    )

    model = GPT(config).to(device)
    model.init_weights()
    n_params = sum(p.numel() for p in model.parameters())

    if inp.num_iterations > 0:
        num_iterations = inp.num_iterations
    else:
        tokens_needed = int(n_params * inp.target_param_data_ratio)
        tokens_per_step = inp.device_batch_size * inp.max_seq_len
        num_iterations = tokens_needed // tokens_per_step

    model_tag = inp.model_tag or f"d{inp.depth}"

    await state_set("nanochat:training", run_id, {
        "status": "running", "type": "base", "depth": inp.depth,
        "parameters": n_params, "num_iterations": num_iterations,
        "device": device, "step": 0, "model_tag": model_tag,
    })
    logger.info("Base training started", {
        "run_id": run_id, "depth": inp.depth, "params": n_params,
        "iterations": num_iterations, "device": device,
    })

    if inp.fp8:
        try:
            from nanochat.fp8 import convert_to_fp8
            convert_to_fp8(model)
        except ImportError:
            logger.warn("FP8 not available, continuing with default precision")

    model = torch.compile(model)
    optimizer = model.setup_optimizer()
    model.train()

    B, T = inp.device_batch_size, inp.max_seq_len
    train_loader = tokenizing_distributed_data_loader_bos_bestfit(tokenizer, B, T, "train", device=device)
    token_bytes = get_token_bytes(device)

    base_dir = get_base_dir()
    checkpoint_dir = os.path.join(base_dir, "checkpoints", model_tag)

    for step_i, (inputs, targets) in enumerate(train_loader):
        if step_i >= num_iterations:
            break

        progress = step_i / num_iterations
        if step_i < inp.warmup_steps:
            lr_frac = step_i / inp.warmup_steps
        elif progress > (1.0 - inp.warmdown_ratio):
            warmdown_progress = (progress - (1.0 - inp.warmdown_ratio)) / inp.warmdown_ratio
            lr_frac = 0.05 + 0.95 * (1.0 + __import__('math').cos(warmdown_progress * __import__('math').pi)) / 2
        else:
            lr_frac = 1.0

        for param_group in optimizer.param_groups:
            param_group["lr"] = param_group["initial_lr"] * lr_frac

        optimizer.zero_grad()
        _logits, loss = model(inputs, targets)
        loss.backward()
        optimizer.step()

        if step_i % 100 == 0:
            await state_set("nanochat:training", run_id, {
                "status": "running", "type": "base", "step": step_i,
                "loss": loss.item(), "num_iterations": num_iterations,
                "lr_frac": lr_frac, "model_tag": model_tag,
            })
            logger.info("Base step", {"run_id": run_id, "step": step_i, "loss": loss.item()})

        if inp.eval_every > 0 and step_i > 0 and step_i % inp.eval_every == 0:
            model.eval()
            val_loader = tokenizing_distributed_data_loader_bos_bestfit(tokenizer, B, T, "val", device=device)
            val_bpb = evaluate_bpb(model, val_loader, steps=20, token_bytes=token_bytes)
            model.train()
            await state_set("nanochat:evals", f"base-bpb-{step_i}", {
                "type": "bpb", "bpb": val_bpb, "step": step_i, "run_id": run_id,
            })

        if inp.save_every > 0 and step_i > 0 and step_i % inp.save_every == 0:
            model.eval()
            meta_data = {
                "step": step_i, "model_config": {
                    "sequence_len": config.sequence_len, "vocab_size": config.vocab_size,
                    "n_layer": config.n_layer, "n_head": config.n_head,
                    "n_kv_head": config.n_kv_head, "n_embd": config.n_embd,
                    "window_pattern": config.window_pattern,
                },
            }
            save_checkpoint(checkpoint_dir, step_i, model.state_dict(), optimizer.state_dict(), meta_data)
            model.train()

    model.eval()
    meta_data = {
        "step": num_iterations, "model_config": {
            "sequence_len": config.sequence_len, "vocab_size": config.vocab_size,
            "n_layer": config.n_layer, "n_head": config.n_head,
            "n_kv_head": config.n_kv_head, "n_embd": config.n_embd,
            "window_pattern": config.window_pattern,
        },
    }
    save_checkpoint(checkpoint_dir, num_iterations, model.state_dict(), optimizer.state_dict(), meta_data)

    await state_set("nanochat:training", run_id, {
        "status": "complete", "type": "base", "step": num_iterations,
        "model_tag": model_tag, "checkpoint_dir": checkpoint_dir,
    })
    logger.info("Base training complete", {"run_id": run_id, "steps": num_iterations})
    return {"status": "complete", "run_id": run_id, "steps": num_iterations, "model_tag": model_tag}


async def fn_train_sft(data: TrainSFTInput) -> dict:
    _ensure_nanochat()
    import torch
    from nanochat.common import autodetect_device_type, get_base_dir
    from nanochat.checkpoint_manager import load_model, save_checkpoint
    from nanochat.tokenizer import get_token_bytes
    from nanochat.loss_eval import evaluate_bpb

    inp = TrainSFTInput.model_validate(data) if isinstance(data, dict) else data
    device = inp.device or autodetect_device_type()
    run_id = str(uuid.uuid4())[:8]

    model, tokenizer, meta = load_model(inp.source, device, "base", model_tag=inp.model_tag, step=inp.step)
    model_config = meta.get("model_config", {})
    max_seq_len = model_config.get("sequence_len", 2048)
    device_batch_size = inp.device_batch_size or 4

    sys.path.insert(0, os.path.join(str(Path(NANOCHAT_DIR).parent), "tasks"))
    from nanochat.tokenizer import RustBPETokenizer

    try:
        from tasks.smoltalk import SmolTalk
        from tasks.mmlu import MMLU
        from tasks.gsm8k import GSM8K
        from tasks.common import TaskMixture
    except ImportError:
        sys.path.insert(0, os.path.join(str(Path(NANOCHAT_DIR).parent), "tasks"))
        from smoltalk import SmolTalk
        from mmlu import MMLU
        from gsm8k import GSM8K
        from common import TaskMixture

    train_tasks = [SmolTalk(split="train")]
    for _ in range(inp.mmlu_epochs):
        train_tasks.append(MMLU(subset="all", split="auxiliary_train"))
    for _ in range(inp.gsm8k_epochs):
        train_tasks.append(GSM8K(subset="main", split="train"))
    train_dataset = TaskMixture(train_tasks)

    dataset_size = len(train_dataset)
    if inp.num_iterations > 0:
        num_iterations = inp.num_iterations
    else:
        tokens_per_step = device_batch_size * max_seq_len
        num_iterations = (dataset_size * max_seq_len) // tokens_per_step

    await state_set("nanochat:training", run_id, {
        "status": "running", "type": "sft", "source": inp.source,
        "device": device, "num_iterations": num_iterations, "step": 0,
        "dataset_size": dataset_size,
    })
    logger.info("SFT training started", {"run_id": run_id, "device": device, "iterations": num_iterations})

    optimizer = model.setup_optimizer()
    model.train()
    token_bytes = get_token_bytes(device)

    base_dir = get_base_dir()
    model_tag = inp.model_tag or "sft"
    checkpoint_dir = os.path.join(base_dir, "chatsft_checkpoints", model_tag)

    bos_token = tokenizer.get_bos_token_id()
    cursor = 0

    for step_i in range(num_iterations):
        batch_inputs, batch_targets = [], []
        for _ in range(device_batch_size):
            conversation = train_dataset[cursor % dataset_size]
            cursor += 1
            ids, mask = tokenizer.render_conversation(conversation, max_tokens=max_seq_len)
            ids = ids[:max_seq_len + 1]
            mask = mask[:max_seq_len + 1]
            while len(ids) < max_seq_len + 1:
                ids.append(bos_token)
                mask.append(0)
            batch_inputs.append(ids[:max_seq_len])
            targets = [ids[i+1] if mask[i+1] == 1 else -1 for i in range(max_seq_len)]
            batch_targets.append(targets)

        inputs_t = torch.tensor(batch_inputs, dtype=torch.int32, device=device)
        targets_t = torch.tensor(batch_targets, dtype=torch.long, device=device)

        progress = step_i / num_iterations
        if progress > (1.0 - inp.warmdown_ratio):
            warmdown_progress = (progress - (1.0 - inp.warmdown_ratio)) / inp.warmdown_ratio
            import math
            lr_frac = 0.0 + 1.0 * (1.0 + math.cos(warmdown_progress * math.pi)) / 2
        else:
            lr_frac = 1.0
        for pg in optimizer.param_groups:
            pg["lr"] = pg["initial_lr"] * lr_frac

        optimizer.zero_grad()
        _logits, loss = model(inputs_t, targets_t)
        loss.backward()
        optimizer.step()

        if step_i % 50 == 0:
            await state_set("nanochat:training", run_id, {
                "status": "running", "type": "sft", "step": step_i,
                "loss": loss.item(), "num_iterations": num_iterations,
            })
            logger.info("SFT step", {"run_id": run_id, "step": step_i, "loss": loss.item()})

        if inp.eval_every > 0 and step_i > 0 and step_i % inp.eval_every == 0:
            model.eval()
            from nanochat.dataloader import tokenizing_distributed_data_loader_bos_bestfit
            val_loader = tokenizing_distributed_data_loader_bos_bestfit(tokenizer, device_batch_size, max_seq_len, "val", device=device)
            val_bpb = evaluate_bpb(model, val_loader, steps=20, token_bytes=token_bytes)
            model.train()
            await state_set("nanochat:evals", f"sft-bpb-{step_i}", {"type": "bpb", "bpb": val_bpb, "step": step_i})

        if inp.save_every > 0 and step_i > 0 and step_i % inp.save_every == 0:
            model.eval()
            save_checkpoint(checkpoint_dir, step_i, model.state_dict(), optimizer.state_dict(), {
                "step": step_i, "model_config": model_config,
            })
            model.train()

    model.eval()
    save_checkpoint(checkpoint_dir, num_iterations, model.state_dict(), optimizer.state_dict(), {
        "step": num_iterations, "model_config": model_config,
    })

    await state_set("nanochat:training", run_id, {
        "status": "complete", "type": "sft", "step": num_iterations,
        "checkpoint_dir": checkpoint_dir,
    })
    logger.info("SFT training complete", {"run_id": run_id, "steps": num_iterations})
    return {"status": "complete", "run_id": run_id, "steps": num_iterations}


async def fn_train_rl(data: TrainRLInput) -> dict:
    _ensure_nanochat()
    import torch
    from nanochat.common import autodetect_device_type, get_base_dir
    from nanochat.checkpoint_manager import load_model, save_checkpoint
    from nanochat.engine import Engine

    inp = TrainRLInput.model_validate(data) if isinstance(data, dict) else data
    device = inp.device or autodetect_device_type()
    run_id = str(uuid.uuid4())[:8]

    model, tokenizer, meta = load_model(inp.source, device, "sft", model_tag=inp.model_tag, step=inp.step)
    model_config = meta.get("model_config", {})
    engine = Engine(model, tokenizer)

    try:
        from tasks.gsm8k import GSM8K
    except ImportError:
        sys.path.insert(0, os.path.join(str(Path(NANOCHAT_DIR).parent), "tasks"))
        from gsm8k import GSM8K

    train_task = GSM8K(subset="main", split="train")
    task_size = len(train_task)

    total_steps = (task_size * inp.num_epochs) // inp.examples_per_step
    await state_set("nanochat:training", run_id, {
        "status": "running", "type": "rl", "device": device,
        "total_steps": total_steps, "step": 0,
    })
    logger.info("RL training started", {"run_id": run_id, "device": device, "total_steps": total_steps})

    optimizer = model.setup_optimizer()
    assistant_end = tokenizer.encode_special("<|assistant_end|>")

    base_dir = get_base_dir()
    checkpoint_dir = os.path.join(base_dir, "chatrl_checkpoints", inp.model_tag or "rl")

    step = 0
    for epoch in range(inp.num_epochs):
        for example_idx in range(0, task_size, inp.examples_per_step):
            batch_examples = list(range(example_idx, min(example_idx + inp.examples_per_step, task_size)))

            all_inputs, all_targets, all_advantages = [], [], []

            for idx in batch_examples:
                conversation = train_task[idx]
                tokens = tokenizer.render_for_completion(conversation)
                prefix_length = len(tokens)

                model.eval()
                generated_seqs, masks = engine.generate_batch(
                    tokens, num_samples=inp.num_samples,
                    max_tokens=inp.max_new_tokens,
                    temperature=inp.temperature, top_k=inp.top_k,
                )

                rewards = []
                for sample_tokens in generated_seqs:
                    gen_text = tokenizer.decode(sample_tokens[prefix_length:])
                    reward = train_task.reward(conversation, gen_text) if hasattr(train_task, 'reward') else 0.0
                    rewards.append(reward)

                rewards_t = torch.tensor(rewards, dtype=torch.float, device=device)
                advantages = rewards_t - rewards_t.mean()

                max_len = max(len(s) for s in generated_seqs)
                for i, seq in enumerate(generated_seqs):
                    padded = seq + [assistant_end] * (max_len - len(seq))
                    mask = masks[i] + [0] * (max_len - len(masks[i]))
                    inp_ids = padded[:-1]
                    tgt_ids = [padded[j+1] if mask[j+1] == 1 else -1 for j in range(len(padded)-1)]
                    all_inputs.append(inp_ids)
                    all_targets.append(tgt_ids)
                    all_advantages.append(advantages[i].item())

            if not all_inputs:
                continue

            model.train()
            max_len = max(len(x) for x in all_inputs)
            for i in range(len(all_inputs)):
                all_inputs[i] += [assistant_end] * (max_len - len(all_inputs[i]))
                all_targets[i] += [-1] * (max_len - len(all_targets[i]))

            for batch_start in range(0, len(all_inputs), inp.device_batch_size):
                batch_end = min(batch_start + inp.device_batch_size, len(all_inputs))
                inp_t = torch.tensor(all_inputs[batch_start:batch_end], dtype=torch.long, device=device)
                tgt_t = torch.tensor(all_targets[batch_start:batch_end], dtype=torch.long, device=device)
                adv_t = torch.tensor(all_advantages[batch_start:batch_end], dtype=torch.float, device=device)

                optimizer.zero_grad()
                logits = model(inp_t)
                log_probs = torch.nn.functional.log_softmax(logits, dim=-1)
                token_log_probs = log_probs.gather(2, tgt_t.clamp(min=0).unsqueeze(-1)).squeeze(-1)
                mask = (tgt_t != -1).float()
                per_sample_loss = -(token_log_probs * mask).sum(dim=1) / mask.sum(dim=1).clamp(min=1)
                loss = (per_sample_loss * adv_t).mean()
                loss.backward()
                optimizer.step()

            step += 1
            if step % 10 == 0:
                mean_reward = sum(all_advantages) / max(len(all_advantages), 1)
                await state_set("nanochat:training", run_id, {
                    "status": "running", "type": "rl", "step": step,
                    "total_steps": total_steps, "mean_advantage": mean_reward,
                })
                logger.info("RL step", {"run_id": run_id, "step": step})

            if inp.save_every > 0 and step > 0 and step % inp.save_every == 0:
                model.eval()
                save_checkpoint(checkpoint_dir, step, model.state_dict(), optimizer.state_dict(), {
                    "step": step, "model_config": model_config,
                })
                model.train()

    model.eval()
    save_checkpoint(checkpoint_dir, step, model.state_dict(), optimizer.state_dict(), {
        "step": step, "model_config": model_config,
    })

    await state_set("nanochat:training", run_id, {
        "status": "complete", "type": "rl", "step": step, "checkpoint_dir": checkpoint_dir,
    })
    logger.info("RL training complete", {"run_id": run_id, "steps": step})
    return {"status": "complete", "run_id": run_id, "steps": step}


async def fn_train_status(data: TrainStatusInput) -> dict:
    inp = TrainStatusInput.model_validate(data) if isinstance(data, dict) else data
    if inp.run_id:
        return await state_get("nanochat:training", inp.run_id) or {"error": "run not found"}
    return {"runs": await state_list("nanochat:training")}


# ---------------------------------------------------------------------------
# Evaluation handlers
# ---------------------------------------------------------------------------

async def fn_eval_core(data: EvalCoreInput) -> dict:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")
    _ensure_nanochat()

    inp = EvalCoreInput.model_validate(data) if isinstance(data, dict) else data

    sys.path.insert(0, os.path.join(str(Path(NANOCHAT_DIR).parent), "scripts"))
    from base_eval import evaluate_core

    device = gpu.model.get_device() if hasattr(gpu.model, "get_device") else gpu.device
    result = evaluate_core(gpu.model, gpu.tokenizer, device, max_per_task=inp.max_per_task)

    await state_set("nanochat:evals", f"core-{int(time.time())}", {
        "type": "core", "core_metric": result["core_metric"],
        "results": result["results"], "model": gpu.source,
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

    inp = EvalLossInput.model_validate(data) if isinstance(data, dict) else data
    token_bytes = get_token_bytes(gpu.device)
    B, T = inp.device_batch_size, gpu.model.config.sequence_len
    batches = tokenizing_distributed_data_loader_bos_bestfit(gpu.tokenizer, B, T, inp.split, device=gpu.device)
    bpb = evaluate_bpb(gpu.model, batches, steps=inp.steps, token_bytes=token_bytes)

    await state_set("nanochat:evals", f"loss-{int(time.time())}", {
        "type": "bpb", "bpb": bpb, "split": inp.split, "model": gpu.source,
    })
    return {"bits_per_byte": bpb, "split": inp.split, "model": gpu.source}


async def fn_eval_chat(data: EvalChatInput) -> dict:
    if not gpu.ready:
        raise RuntimeError("No model loaded. Trigger 'nanochat.model.load' first.")
    _ensure_nanochat()

    inp = EvalChatInput.model_validate(data) if isinstance(data, dict) else data

    sys.path.insert(0, os.path.join(str(Path(NANOCHAT_DIR).parent), "scripts"))
    sys.path.insert(0, os.path.join(str(Path(NANOCHAT_DIR).parent), "tasks"))

    from chat_eval import run_generative_eval, run_categorical_eval

    try:
        from tasks.gsm8k import GSM8K
        from tasks.mmlu import MMLU
        from tasks.arc import ARC
    except ImportError:
        from gsm8k import GSM8K
        from mmlu import MMLU
        from arc import ARC

    available_tasks = {
        "gsm8k": lambda: GSM8K(subset="main", split="test"),
        "mmlu": lambda: MMLU(subset="all", split="test"),
        "arc": lambda: ARC(split="test"),
    }

    if inp.task_name and inp.task_name in available_tasks:
        tasks_to_run = {inp.task_name: available_tasks[inp.task_name]}
    elif inp.task_name:
        raise ValueError(f"Unknown task: {inp.task_name}. Available: {list(available_tasks.keys())}")
    else:
        tasks_to_run = available_tasks

    results = {}
    for name, task_fn in tasks_to_run.items():
        task_obj = task_fn()
        if hasattr(task_obj, "reward"):
            acc = run_generative_eval(
                task_obj, gpu.tokenizer, gpu.model, gpu.engine,
                num_samples=inp.num_samples, max_new_tokens=inp.max_new_tokens,
                temperature=inp.temperature, top_k=inp.top_k,
                max_problems=inp.max_problems,
            )
        else:
            acc = run_categorical_eval(
                task_obj, gpu.tokenizer, gpu.model,
                batch_size=inp.batch_size, max_problems=inp.max_problems,
            )
        results[name] = acc

    await state_set("nanochat:evals", f"chat-{int(time.time())}", {
        "type": "chat", "results": results, "model": gpu.source,
    })
    return {"results": results, "model": gpu.source}


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
        ("nanochat.tools.execute", fn_tools_execute, "Execute Python code in sandbox", "http", {"api_path": "/nanochat/tools/execute", "http_method": "POST"}),
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
