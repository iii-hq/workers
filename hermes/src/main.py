from __future__ import annotations

import logging
import os
import signal
import threading
from pathlib import Path
from typing import Any

import yaml
from iii import InitOptions, register_worker

from .handlers import create_handlers
from .hooks import use_api

DEFAULTS: dict[str, Any] = {
    "engine_url": "ws://127.0.0.1:49134",
    "defaults": {"model": "", "cwd": ""},
    "events_stream": "agent::events",
    "raw_events_stream": "hermes::events",
    "iii_context": True,
    "hermes_executable": "",
    "inbound_api_path": "/hermes/inbound",
}

RUN_REQUEST_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {"type": "string", "description": "iii session id; reuse to continue a Hermes conversation"},
        "prompt": {"type": "string", "description": "The user prompt for this turn"},
        "messages": {"type": "array", "description": "Alternative to prompt; the last user entry becomes the prompt"},
        "model": {"type": "string", "description": "HERMES_INFERENCE_MODEL value; empty = Hermes default"},
        "cwd": {"type": "string", "description": "Working directory the turn runs in"},
        "iii_context": {"type": "boolean", "description": "Prepend the iii runtime discovery context"},
    },
}
SEND_REQUEST_FORMAT = {
    "type": "object",
    "properties": {
        "platform": {"type": "string", "description": "Gateway platform, e.g. telegram, discord, slack"},
        "message": {"type": "string", "description": "Message body to deliver"},
    },
    "required": ["platform", "message"],
}
SESSION_ID_FORMAT = {"type": "object", "properties": {"session_id": {"type": "string"}}}

RUN_RESPONSE_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {"type": "string"},
        "result": {"type": "string"},
        "is_error": {"type": "boolean"},
        "stop_reason": {"type": "string"},
        "busy": {"type": "boolean"},
        "reason": {"type": "string"},
    },
}
START_RESPONSE_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {"type": "string"},
        "started": {"type": "boolean"},
        "busy": {"type": "boolean"},
        "reason": {"type": "string"},
    },
}
SEND_RESPONSE_FORMAT = {
    "type": "object",
    "properties": {
        "platform": {"type": "string"},
        "sent": {"type": "boolean"},
        "detail": {"type": "string"},
    },
}
SESSIONS_RESPONSE_FORMAT = {
    "type": "object",
    "properties": {
        "sessions": {"type": "array", "items": {"type": "object"}},
        "hermes_sessions_raw": {"type": "string"},
    },
}
STATUS_RESPONSE_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {"type": "string"},
        "live": {"type": "boolean"},
        "record": {"type": ["object", "null"]},
    },
}
STOP_RESPONSE_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {"type": "string"},
        "stopped": {"type": "boolean"},
        "reason": {"type": "string"},
    },
}


def load_config() -> dict[str, Any]:
    cfg = {**DEFAULTS, "defaults": {**DEFAULTS["defaults"]}}
    path = Path(os.environ.get("HERMES_WORKER_CONFIG", "config.yaml"))
    try:
        raw = yaml.safe_load(path.read_text()) or {}
    except FileNotFoundError:
        raw = {}
    cfg.update({k: v for k, v in raw.items() if k != "defaults"})
    cfg["defaults"].update(raw.get("defaults") or {})
    return cfg


def main() -> None:
    cfg = load_config()
    url = os.environ.get("III_URL") or cfg["engine_url"]
    iii = register_worker(
        address=url,
        options=InitOptions(worker_name="hermes", otel={"enabled": True, "service_name": "hermes"}),
    )
    logging.basicConfig(level=logging.INFO)
    logger = logging.getLogger("hermes")
    handlers = create_handlers(iii, load_config, logger)

    iii.register_function(
        "hermes::run",
        handlers["run"],
        description=(
            "Run one Hermes turn and wait. Accepts `prompt` or a `messages` array; returns "
            "{session_id, result}. Carries the iii runtime context by default. Final-text only "
            "(Hermes one-shot exposes no per-tool event stream)."
        ),
        request_format=RUN_REQUEST_FORMAT,
        response_format=RUN_RESPONSE_FORMAT,
    )
    iii.register_function(
        "hermes::start",
        handlers["start"],
        description=(
            "Start a Hermes turn and return immediately; watch agent::events "
            "(group_id = session_id) for the result. Returns {session_id, started}."
        ),
        request_format=RUN_REQUEST_FORMAT,
        response_format=START_RESPONSE_FORMAT,
    )
    iii.register_function(
        "run::start_and_wait",
        handlers["run"],
        description="Alias for hermes::run under the shared agent entrypoint.",
        request_format=RUN_REQUEST_FORMAT,
        response_format=RUN_RESPONSE_FORMAT,
    )
    iii.register_function(
        "hermes::send",
        handlers["send"],
        description=(
            "Deliver a message to a Hermes gateway platform (telegram, discord, slack, "
            "and 27+ others). Outbound omnichannel."
        ),
        request_format=SEND_REQUEST_FORMAT,
        response_format=SEND_RESPONSE_FORMAT,
    )
    iii.register_function(
        "hermes::sessions::list",
        handlers["sessions_list"],
        description="List Hermes sessions this worker has run, plus the raw `hermes sessions list` output.",
        request_format={"type": "object", "properties": {}},
        response_format=SESSIONS_RESPONSE_FORMAT,
    )
    iii.register_function(
        "hermes::status",
        handlers["status"],
        description="Point-in-time status of a Hermes session: live flag and stored record.",
        request_format=SESSION_ID_FORMAT,
        response_format=STATUS_RESPONSE_FORMAT,
    )
    iii.register_function(
        "hermes::stop",
        handlers["stop"],
        description="Interrupt a live Hermes run for a session.",
        request_format=SESSION_ID_FORMAT,
        response_format=STOP_RESPONSE_FORMAT,
    )

    # Inbound sink: the Hermes gateway delivers platform/webhook events here over
    # iii-http; the worker republishes them for iii workers to react to.
    use_api(
        iii,
        {
            "function_id": "hermes::inbound",
            "api_path": cfg["inbound_api_path"],
            "http_method": "POST",
            "description": "Inbound Hermes gateway delivery sink (platform messages + webhooks).",
            "request_format": {
                "type": "object",
                "properties": {"body": {"type": "object"}, "headers": {"type": "object"}},
            },
            "response_format": {
                "type": "object",
                "properties": {
                    "statusCode": {"type": "integer"},
                    "body": {"type": "object"},
                    "headers": {"type": "object"},
                },
            },
        },
        handlers["inbound"],
    )

    print(f"hermes worker connected to {url}")

    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    stop.wait()
    iii.shutdown()


if __name__ == "__main__":
    main()
