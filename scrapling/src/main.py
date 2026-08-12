from __future__ import annotations

import logging
import os
import signal
import threading
from pathlib import Path
from typing import Any

import yaml
from iii import InitOptions, register_worker

from . import guidance, sessions, storage
from .handlers import create_handlers
from .schemas import FUNCTIONS

DEFAULTS: dict[str, Any] = {
    "engine_url": "ws://127.0.0.1:49134",
    "defaults": {
        "impersonate": "chrome",
        "headless": True,
        "network_idle": False,
        "proxy": "",
        "include_html": False,
    },
    "max_bulk_concurrency": 5,
    "adaptive_storage_path": storage.DEFAULT_PATH,
    "max_sessions": 8,
    "session_idle_timeout_s": 900,
}


def load_config() -> dict[str, Any]:
    cfg = {**DEFAULTS, "defaults": {**DEFAULTS["defaults"]}}
    path = Path(os.environ.get("SCRAPLING_WORKER_CONFIG", "config.yaml"))
    try:
        raw = yaml.safe_load(path.read_text()) or {}
    except FileNotFoundError:
        raw = {}
    cfg.update({k: v for k, v in raw.items() if k != "defaults"})
    cfg["defaults"].update(raw.get("defaults") or {})
    return cfg


def _print_connected(state: str, url: str) -> None:
    # Fires on every (re)connect via the SDK connection-state listener, so the
    # "connected" line is evidence of a live socket, not startup aspiration (MOT-3969).
    if state == "connected":
        # flush: this line is timing evidence; don't let it sit in a pipe buffer
        print(f"scrapling worker connected to {url} ({len(FUNCTIONS)} functions)", flush=True)


def main() -> None:
    cfg = load_config()
    url = os.environ.get("III_URL") or cfg["engine_url"]
    logging.basicConfig(level=logging.INFO)

    storage.configure(cfg.get("adaptive_storage_path"))
    sessions.setup(
        max_sessions=int(cfg.get("max_sessions", 8)),
        idle_timeout=cfg.get("session_idle_timeout_s") or None,
    )

    # namespace: register under III_NAMESPACE when set (the SDK also resolves the
    # env; passed explicitly for visibility). Absent -> the engine's default.
    iii = register_worker(
        address=url,
        options=InitOptions(
            worker_name="scrapling",
            namespace=os.environ.get("III_NAMESPACE") or None,
        ),
    )
    handlers = create_handlers(load_config, iii=iii)

    for spec in FUNCTIONS:
        iii.register_function(
            spec["id"],
            handlers[spec["handler"]],
            description=spec["description"],
            request_format=spec["request"],
            response_format=spec["response"],
        )

    guidance.setup(iii)

    iii.add_connection_state_listener(lambda state: _print_connected(state, url))

    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    stop.wait()
    sessions.registry().close_all()  # tear down live browsers; iii.shutdown() won't
    iii.shutdown()


if __name__ == "__main__":
    main()
