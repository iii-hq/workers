"""In-memory stand-in for the engine bus.

`state::get/set/list` are backed by a dict keyed `scope/key`, every trigger call
is recorded (with its action, namespace and metadata, so Void deliveries of the
owned event feeds can be asserted), and `register_function` /
`register_trigger_type` are captured so tests can invoke handlers at the same
boundary the engine uses. `bind` / `unbind` drive a captured trigger type's
handler the way the engine does when a consumer registers a binding. Mirrors
the pi worker's `fake-iii` helper.

Handlers await `trigger_async` (0.19.x forbids the sync `trigger` from the
event-loop thread); `trigger_async` here backs the same in-memory store. The
sync `trigger` is kept only as that backing implementation, not a handler
entrypoint.
"""

from __future__ import annotations

import asyncio
import copy
from typing import Any

from iii import TriggerActionVoid
from iii.triggers import TriggerConfig


class FakeIii:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.state: dict[str, Any] = {}
        self.registered: dict[str, Any] = {}
        self.triggers: list[dict[str, Any]] = []
        self.trigger_types: dict[str, dict[str, Any]] = {}
        # function ids whose invocation raises (simulates a gone consumer)
        self.failing_functions: set[str] = set()
        self._binding_seq = 0

    def trigger(self, request: dict[str, Any]) -> Any:
        fid = request["function_id"]
        # Clone like the wire would: later caller-side mutation must not rewrite
        # the recorded call or the stored value.
        payload = copy.deepcopy(request.get("payload", {}))
        call: dict[str, Any] = {"function_id": fid, "payload": payload}
        for extra in ("action", "namespace", "metadata"):
            if extra in request:
                call[extra] = copy.deepcopy(request[extra])
        self.calls.append(call)
        if fid in self.failing_functions:
            raise RuntimeError(f"function {fid} is gone")
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

    def register_trigger_type(self, trigger_type: dict[str, Any], handler: Any) -> None:
        self.trigger_types[trigger_type["id"]] = {"info": trigger_type, "handler": handler}

    # -- engine side of a binding -------------------------------------------

    async def bind_async(
        self,
        type_id: str,
        config: Any,
        *,
        function_id: str | None = None,
        binding_id: str | None = None,
        namespace: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> str:
        fid = function_id or consumer_fn(type_id)
        self._binding_seq += 1
        bid = binding_id or f"binding-{self._binding_seq}"
        handler = self.trigger_types[type_id]["handler"]
        await handler.register_trigger(
            TriggerConfig(id=bid, function_id=fid, config=config, metadata=metadata, namespace=namespace)
        )
        return bid

    def bind(self, type_id: str, session_id: str | None = None, **kw: Any) -> str:
        config = kw.pop("config", {"session_id": session_id})
        return asyncio.run(self.bind_async(type_id, config, **kw))

    def unbind(self, type_id: str, binding_id: str) -> None:
        handler = self.trigger_types[type_id]["handler"]
        asyncio.run(handler.unregister_trigger(TriggerConfig(id=binding_id, function_id="", config=None)))

    def void_calls(self, function_id: str | None = None) -> list[dict[str, Any]]:
        return [
            c
            for c in self.calls
            if isinstance(c.get("action"), TriggerActionVoid)
            and (function_id is None or c["function_id"] == function_id)
        ]

    def feed_frames(self, type_id: str, function_id: str | None = None) -> list[dict[str, Any]]:
        """Delivery payloads received by the default consumer bound via `bind`."""
        return [c["payload"] for c in self.void_calls(function_id or consumer_fn(type_id))]

    def feed_events(self, type_id: str) -> list[dict[str, Any]]:
        return [p["event"] for p in self.feed_frames(type_id)]


def consumer_fn(type_id: str) -> str:
    return f"consumer::{type_id.replace('::', '-')}"


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
