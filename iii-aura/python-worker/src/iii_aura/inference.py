"""Gemma 4 E2B model loading and tool definition."""

import atexit
import os

import litert_lm

HF_REPO = "litert-community/gemma-4-E2B-it-litert-lm"
HF_FILENAME = "gemma-4-E2B-it.litertlm"

SYSTEM_PROMPT = (
    "You are a friendly, conversational AI assistant. The user is talking to you "
    "through a microphone and showing you their camera. "
    "You MUST always use the respond_to_user tool to reply. "
    "First transcribe exactly what the user said, then write your response."
)


def resolve_model_path() -> str:
    path = os.environ.get("MODEL_PATH", "")
    if path:
        return path
    from huggingface_hub import hf_hub_download

    print(f"Downloading {HF_REPO}/{HF_FILENAME} (first run only)...")
    return hf_hub_download(repo_id=HF_REPO, filename=HF_FILENAME)


engine = None
tool_result: dict[str, str] = {}


def respond_to_user(transcription: str, response: str) -> str:
    """Respond to the user's voice message.

    Args:
        transcription: Exact transcription of what the user said in the audio.
        response: Your conversational response to the user. Keep it to 1-4 short sentences.
    """
    tool_result["transcription"] = transcription
    tool_result["response"] = response
    return "OK"


def load():
    """Load the Gemma engine (call once at startup)."""
    global engine
    model_path = resolve_model_path()
    print(f"Loading Gemma 4 E2B from {model_path}...")
    engine = litert_lm.Engine(
        model_path,
        backend=litert_lm.Backend.GPU,
        vision_backend=litert_lm.Backend.GPU,
        audio_backend=litert_lm.Backend.CPU,
    )
    engine.__enter__()
    atexit.register(unload)
    print("Gemma engine loaded.")


def unload():
    """Release the Gemma engine (called automatically at exit)."""
    global engine
    if engine is not None:
        try:
            engine.__exit__(None, None, None)
        except Exception:
            pass
        engine = None
