"""iii Aura worker — on-device multimodal voice + vision, wired through iii primitives.

Functions:
    aura::session::open    — create a session, returns session_id
    aura::ingest::turn     — receive audio+image via channel, run inference, stream TTS back
    aura::interrupt        — cancel in-flight generation for a session
"""

from __future__ import annotations

import asyncio
import base64
import concurrent.futures
import concurrent.futures.thread as _cft
import json
import os
import re
import signal
import threading
import time
import uuid
from typing import Any

import numpy as np

from iii import ChannelReader, InitOptions, Logger, TriggerAction, register_worker  # type: ignore[import-not-found]

from . import tts as tts_module
from . import inference

SENTENCE_SPLIT_RE = re.compile(r"(?<=[.!?])\s+")

logger = Logger(service_name="iii-aura")

_interrupts: dict[str, asyncio.Event] = {}
_tts: tts_module.TTSBackend | None = None


def _patch_localhost(obj: Any, attr: str = "_url") -> None:
    """Replace localhost with 127.0.0.1 to bypass DNS when litert_lm poisons the executor."""
    if hasattr(obj, attr):
        setattr(obj, attr, getattr(obj, attr).replace("://localhost", "://127.0.0.1"))


def _clean_model_tokens(s: str) -> str:
    return s.replace('<|"|>', "").strip()


class _SafeExecutor(concurrent.futures.ThreadPoolExecutor):
    """ThreadPoolExecutor that resets the global _shutdown flag before each submit.

    litert_lm's C++ init sets concurrent.futures.thread._shutdown = True,
    which poisons ALL executors. This subclass ensures it's always cleared.
    """

    def submit(self, fn, /, *args, **kwargs):
        _cft._shutdown = False
        return super().submit(fn, *args, **kwargs)


_executor: _SafeExecutor | None = None

iii = None


def _split_sentences(text: str) -> list[str]:
    parts = SENTENCE_SPLIT_RE.split(text.strip())
    return [s.strip() for s in parts if s.strip()]


# ---------------------------------------------------------------------------
# Functions registered with the iii engine
# ---------------------------------------------------------------------------

async def _session_open(data: dict[str, Any]) -> dict[str, Any]:
    session_id = str(uuid.uuid4())
    _interrupts[session_id] = asyncio.Event()

    await iii.trigger_async({
        "function_id": "state::set",
        "payload": {
            "scope": "aura",
            "key": session_id,
            "value": {"session_id": session_id, "created_at": time.time()},
        },
    })

    logger.info("Session opened", {"session_id": session_id})
    return {"session_id": session_id}


async def _ingest_turn(data: dict[str, Any]) -> dict[str, Any]:
    session_id = data["session_id"]
    reader: ChannelReader = data["reader"]

    interrupted = _interrupts.get(session_id)
    if interrupted is None:
        interrupted = asyncio.Event()
        _interrupts[session_id] = interrupted
    interrupted.clear()

    _patch_localhost(reader)

    # Read audio + metadata from channel
    messages: list[str] = []
    reader.on_message(lambda msg: messages.append(msg))
    raw = await reader.read_all()

    image_b64: str | None = None
    if messages:
        try:
            image_b64 = json.loads(messages[0]).get("image")
        except json.JSONDecodeError:
            pass

    audio_b64 = base64.b64encode(raw).decode() if raw else None

    # Build multimodal content for Gemma 4 E2B
    content: list[dict[str, str]] = []
    if audio_b64:
        content.append({"type": "audio", "blob": audio_b64})
    if image_b64:
        content.append({"type": "image", "blob": image_b64})

    if audio_b64 and image_b64:
        content.append({"type": "text", "text": (
            "The user just spoke to you (audio) while showing their camera (image). "
            "Respond to what they said, referencing what you see if relevant."
        )})
    elif audio_b64:
        content.append({"type": "text", "text": "The user just spoke to you. Respond to what they said."})
    elif image_b64:
        content.append({"type": "text", "text": "The user is showing you their camera. Describe what you see."})
    else:
        return {"error": "no_input"}

    # LLM inference on the GPU executor thread
    loop = asyncio.get_running_loop()
    t0 = time.time()
    inference.tool_result.clear()

    def _infer():
        # Fresh conversation per turn — avoids context overflow from multimodal tokens
        conv = inference.engine.create_conversation(
            messages=[{"role": "system", "content": inference.SYSTEM_PROMPT}],
            tools=[inference.respond_to_user],
        )
        conv.__enter__()
        try:
            return conv.send_message({"role": "user", "content": content})
        finally:
            conv.__exit__(None, None, None)

    response = await loop.run_in_executor(_executor, _infer)
    llm_time = time.time() - t0

    if inference.tool_result:
        transcription = _clean_model_tokens(inference.tool_result.get("transcription", ""))
        text_response = _clean_model_tokens(inference.tool_result.get("response", ""))
    else:
        transcription = None
        # Defensive read — if the tool call didn't fire and the response
        # shape is unexpected, fall back to an empty string so the turn
        # completes instead of raising and aborting the whole session.
        text_response = ""
        try:
            content_items = response.get("content") if isinstance(response, dict) else None
            if isinstance(content_items, list) and content_items:
                first = content_items[0]
                if isinstance(first, dict) and isinstance(first.get("text"), str):
                    text_response = first["text"]
        except (KeyError, IndexError, TypeError) as exc:
            logger.warn(
                "unexpected LLM response shape, returning empty text",
                {"error": str(exc)},
            )

    logger.info("LLM complete", {"llm_time": round(llm_time, 2), "text": text_response[:80]})

    if interrupted.is_set():
        return {"interrupted": True}

    # Send transcript to browser
    await iii.trigger_async({
        "function_id": "ui::aura::transcript",
        "payload": {
            "text": text_response,
            "transcription": transcription,
            "llm_time": round(llm_time, 2),
        },
        "action": TriggerAction.Void(),
    })

    if interrupted.is_set():
        return {"interrupted": True}

    # Stream TTS audio back via channel
    sentences = _split_sentences(text_response) or [text_response]
    playback_channel = await iii.create_channel_async()

    if hasattr(playback_channel, 'writer'):
        _patch_localhost(playback_channel.writer)

    await iii.trigger_async({
        "function_id": "ui::aura::playback",
        "payload": {
            "reader": playback_channel.reader_ref.model_dump(),
            "sample_rate": _tts.sample_rate,
            "sentence_count": len(sentences),
        },
        "action": TriggerAction.Void(),
    })

    tts_start = time.time()
    for i, sentence in enumerate(sentences):
        if interrupted.is_set():
            logger.info(f"Interrupted during TTS (sentence {i + 1}/{len(sentences)})")
            break

        pcm = await loop.run_in_executor(_executor, lambda s=sentence: _tts.generate(s))
        if interrupted.is_set():
            break

        pcm_int16 = (pcm * 32767).clip(-32768, 32767).astype(np.int16)
        await playback_channel.writer.write(pcm_int16.tobytes())
        await playback_channel.writer.send_message_async(json.dumps({
            "type": "audio_chunk", "index": i,
        }))

    tts_time = time.time() - tts_start
    logger.info("TTS complete", {"tts_time": round(tts_time, 2), "sentences": len(sentences)})

    await playback_channel.writer.send_message_async(json.dumps({
        "type": "audio_end", "tts_time": round(tts_time, 2),
    }))
    await playback_channel.writer.close_async()

    return {"llm_time": round(llm_time, 2), "tts_time": round(tts_time, 2)}


async def _interrupt(data: dict[str, Any]) -> None:
    session_id = data.get("session_id", "")
    ev = _interrupts.get(session_id)
    if ev:
        ev.set()
        logger.info("Interrupt signalled", {"session_id": session_id})


async def _session_close(data: dict[str, Any]) -> dict[str, Any]:
    session_id = data.get("session_id", "")
    ev = _interrupts.pop(session_id, None)
    if ev:
        ev.set()
    logger.info("Session closed", {"session_id": session_id})
    return {"closed": True}


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main():
    global iii, _tts, _executor

    # Load LLM + TTS models
    print("[aura] Loading models...")
    inference.load()
    _tts = tts_module.load()
    print("[aura] Models loaded.")

    # Single-worker executor for GPU calls — resets the global _shutdown flag
    # that litert_lm's C++ init sets on concurrent.futures
    _cft._shutdown = False
    _executor = _SafeExecutor(max_workers=1)

    engine_ws_url = os.environ.get("III_URL", "ws://127.0.0.1:49134")
    print(f"[aura] Connecting to {engine_ws_url}")
    iii = register_worker(
        address=engine_ws_url,
        options=InitOptions(
            worker_name="iii-aura",
            otel={"enabled": True, "service_name": "iii-aura"},
        ),
    )
    print("[aura] Connected to engine!")

    iii.register_function("aura::session::open", _session_open, description="Open an Aura session")
    iii.register_function("aura::ingest::turn", _ingest_turn, description="Ingest a voice+vision turn")
    iii.register_function("aura::interrupt", _interrupt, description="Interrupt in-flight generation")
    iii.register_function("aura::session::close", _session_close, description="Close a session and free resources")

    iii.register_trigger({
        "type": "http",
        "function_id": "aura::session::open",
        "config": {"api_path": "/aura/session", "http_method": "POST"},
    })

    iii.on_functions_available(
        lambda fns: logger.info(f"Aura worker ready — {len(fns)} functions available")
    )

    logger.info("iii-aura worker started")

    # Block main thread until a signal arrives — without this the process
    # would exit right after registration and the async handlers above
    # would never get to run.
    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    stop.wait()
    logger.info("iii-aura shutting down")
    try:
        iii.shutdown()
    except Exception:
        pass


if __name__ == "__main__":
    main()
