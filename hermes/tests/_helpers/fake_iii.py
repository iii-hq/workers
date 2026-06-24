"""In-memory stand-in for the engine bus.

`state::get/set/list` are backed by a dict keyed `scope/key`, `stream::set` is
recorded as a plain call, and `register_function` is captured so tests can
invoke handlers at the same boundary the engine uses. Mirrors the pi worker's
`fake-iii` helper.

Handlers await `trigger_async` (0.19.x forbids the sync `trigger` from the
event-loop thread); `trigger_async` here backs the same in-memory store. The
sync `trigger` is kept only as that backing implementation, not a handler
entrypoint.
"""

from __future__ import annotations

import copy
from typing import Any


class FakeIii:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.state: dict[str, Any] = {}
        self.registered: dict[str, Any] = {}
        self.triggers: list[dict[str, Any]] = []

    def trigger(self, request: dict[str, Any]) -> Any:
        fid = request["function_id"]
        # Clone like the wire would: later caller-side mutation must not rewrite
        # the recorded call or the stored value.
        payload = copy.deepcopy(request.get("payload", {}))
        self.calls.append({"function_id": fid, "payload": payload})
        scope, key, value = payload.get("scope"), payload.get("key"), payload.get("value")
        if fid == "state::set":
            self.state[f"{scope}/{key}"] = value
            return None
        if fid == "state::get":
            return copy.deepcopy(self.state.get(f"{scope}/{key}"))
        if fid == "state::list":
            return [copy.deepcopy(v) for k, v in self.state.items() if k.startswith(f"{scope}/")]
        return None

    async def trigger_async(self, request: dict[str, Any]) -> Any:
        # Handlers await `trigger_async` (0.19.x forbids the sync `trigger` from
        # the event-loop thread); back it with the same in-memory store.
        return self.trigger(request)

    def register_function(self, fid: str, handler: Any, **_kw: Any) -> None:
        self.registered[fid] = handler

    def register_trigger(self, spec: dict[str, Any]) -> None:
        self.triggers.append(spec)

    def stream_frames(self, stream_name: str) -> list[dict[str, Any]]:
        return [
            c["payload"]
            for c in self.calls
            if c["function_id"] == "stream::set" and c["payload"].get("stream_name") == stream_name
        ]


class FakeLogger:
    # Match logging.Logger-style calls: logger.error("...%s", a, b).
    def __init__(self) -> None:
        self.errors: list[tuple[str, tuple[Any, ...]]] = []
        self.infos: list[tuple[str, tuple[Any, ...]]] = []

    def error(self, msg: str, *args: Any) -> None:
        self.errors.append((msg, args))

    def info(self, msg: str, *args: Any) -> None:
        self.infos.append((msg, args))


def base_cfg(**overrides: Any) -> dict[str, Any]:
    cfg = {
        "engine_url": "ws://127.0.0.1:49134",
        "defaults": {"model": "", "cwd": "", "tools": ""},
        "events_stream": "agent::events",
        "raw_events_stream": "hermes::events",
        "iii_context": True,
        "hermes_executable": "",
        "inbound_api_path": "/hermes/inbound",
    }
    for k, v in overrides.items():
        if k == "defaults":
            cfg["defaults"].update(v)
        else:
            cfg[k] = v
    return cfg
