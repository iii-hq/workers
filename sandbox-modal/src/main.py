"""Entry point for sandbox-modal. Registers the seven `sandbox::provider::modal::*`
functions on the iii engine, then waits on SIGTERM/SIGINT.
"""

from __future__ import annotations

import os
import signal
import threading
from typing import Any

from iii import InitOptions, register_worker

from .config import Config
from .handlers import (
    HandlerCtx,
    do_create,
    do_exec,
    do_expose_port,
    do_list,
    do_snapshot,
    do_stop,
)
from .modal_client import ModalClient


def main() -> None:
    engine_ws_url = os.environ.get("III_URL", "ws://localhost:49134")
    config_path = os.environ.get("SANDBOX_MODAL_CONFIG", "./config.yaml")
    config = Config.load(config_path)
    client = ModalClient(app_name="sandbox-modal")
    ctx = HandlerCtx(config=config, client=client)

    iii = register_worker(
        address=engine_ws_url,
        options=InitOptions(
            worker_name="sandbox-modal",
            otel={"enabled": True, "service_name": "sandbox-modal"},
        ),
    )

    print(f"sandbox-modal connected to engine: {engine_ws_url}")

    def reg(function_id: str, handler: Any) -> None:
        async def wrapped(payload: dict[str, Any]) -> dict[str, Any]:
            return await handler(ctx, payload or {})

        iii.register_function(function_id, wrapped)

    reg("sandbox::provider::modal::create", do_create)
    reg("sandbox::provider::modal::exec", do_exec)
    reg("sandbox::provider::modal::stop", do_stop)
    reg("sandbox::provider::modal::list", do_list)
    reg("sandbox::provider::modal::snapshot", do_snapshot)
    reg("sandbox::provider::modal::expose_port", do_expose_port)

    print("sandbox-modal registered, awaiting invocations")

    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    stop.wait()
    iii.shutdown()


if __name__ == "__main__":
    main()
