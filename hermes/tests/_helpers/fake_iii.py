"""In-memory stand-in for the engine bus.

`state::get/set/list` are backed by a dict keyed `scope/key`, and every call is
recorded (with its action, namespace and metadata, so the Void calls of the
owned event feeds can be asserted). `register_function` and the observer hooks
(`on_observer_added` / `on_observer_removed` / `on_observer_match`, by function
id) are captured so tests drive them at the same boundary the SDK does:
`observe_feed` adds an observer the way the engine announces one (the added
hook runs, then the match hook's observer keys file it), and a call of a
registered function runs its handler and plays iii's delivery, recording what
each matched observer's target receives. `observe` records the observers the
worker itself creates (its http route). Mirrors the pi worker's `fake-iii`
helper.

Handlers await `call_async`; the sync `call` is kept only as the backing store
for `state::*`, not a handler entrypoint.
"""

from __future__ import annotations

import asyncio
import copy
import inspect
from dataclasses import dataclass, field
from typing import Any

from iii import CallActionVoid


@dataclass
class FakeObserver:
    """What the worker's hooks see of an observer (`obs.id`, `obs.args`, ...)."""

    id: str
    function_id: str
    args: dict[str, Any]
    target: str
    metadata: dict[str, Any] | None = None
    keys: tuple[str, ...] = field(default=())


@dataclass
class _Call:
    """The parts of a call a match hook's `call=` key function reads."""

    input: Any


class FakeFunctionRef:
    def __init__(self, fake: FakeIii, function_id: str) -> None:
        self._fake = fake
        self.id = function_id

    def on_observer_added(self, hook: Any) -> Any:
        return self._fake.on_observer_added(self.id, hook)

    def on_observer_removed(self, hook: Any) -> Any:
        return self._fake.on_observer_removed(self.id, hook)

    def on_observer_match(self, hook: Any = None, *, observer: Any = None, call: Any = None) -> Any:
        return self._fake.on_observer_match(self.id, hook, observer=observer, call=call)


class FakeIii:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.state: dict[str, Any] = {}
        self.registered: dict[str, Any] = {}
        self.registrations: dict[str, dict[str, Any]] = {}
        self.added_hooks: dict[str, Any] = {}
        self.removed_hooks: dict[str, Any] = {}
        self.match_keys: dict[str, tuple[Any, Any]] = {}
        # Active observers per function id, as the SDK holds them.
        self.observers: dict[str, list[FakeObserver]] = {}
        # Observers the worker created with `observe` (source, target, options).
        self.observed: list[dict[str, Any]] = []
        # iii's delivery: one entry per matched observer per call.
        self.deliveries: list[dict[str, Any]] = []
        # function ids whose call raises (simulates a refused or failed call)
        self.failing_functions: set[str] = set()
        self._observer_seq = 0

    # -- calls ---------------------------------------------------------------

    def call(self, request: dict[str, Any]) -> Any:
        fid = request["function_id"]
        # Clone like the wire would: later caller-side mutation must not rewrite
        # the recorded call or the stored value.
        payload = copy.deepcopy(request.get("payload", {}))
        record: dict[str, Any] = {"function_id": fid, "payload": payload}
        for extra in ("action", "namespace", "metadata"):
            if extra in request:
                record[extra] = copy.deepcopy(request[extra])
        self.calls.append(record)
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

    async def call_async(self, request: dict[str, Any]) -> Any:
        result = self.call(request)
        fid = request["function_id"]
        handler = self.registered.get(fid)
        if handler is None:
            return result
        payload = copy.deepcopy(request.get("payload", {}))
        output = handler(payload)
        if inspect.isawaitable(output):
            output = await output
        for obs in self._matching(fid, payload):
            self.deliveries.append(
                {
                    "observer": obs.id,
                    "function_id": fid,
                    "target": obs.target,
                    "payload": copy.deepcopy(output),
                    "metadata": obs.metadata,
                }
            )
        return None if isinstance(request.get("action"), CallActionVoid) else output

    def void_calls(self, function_id: str | None = None) -> list[dict[str, Any]]:
        return [
            c
            for c in self.calls
            if isinstance(c.get("action"), CallActionVoid) and (function_id is None or c["function_id"] == function_id)
        ]

    # -- registration and hooks ----------------------------------------------

    def register_function(self, fid: str, handler: Any, **kw: Any) -> FakeFunctionRef:
        self.registered[fid] = handler
        self.registrations[fid] = kw
        return FakeFunctionRef(self, fid)

    def on_observer_added(self, fid: str, hook: Any) -> Any:
        self.added_hooks[fid] = hook
        return hook

    def on_observer_removed(self, fid: str, hook: Any) -> Any:
        self.removed_hooks[fid] = hook
        return hook

    def on_observer_match(self, fid: str, hook: Any = None, *, observer: Any = None, call: Any = None) -> Any:
        assert hook is None, "the feeds match by keys"
        self.match_keys[fid] = (observer, call)
        return self.match_keys[fid]

    def observe(self, source: dict[str, Any], target: str, options: dict[str, Any] | None = None) -> dict[str, Any]:
        record = {"source": source, "target": target, "options": options}
        self.observed.append(record)
        return record

    # -- the engine announcing observers -------------------------------------

    async def observe_feed_async(
        self,
        fid: str,
        session_id: str | None = None,
        *,
        args: dict[str, Any] | None = None,
        target: str | None = None,
        observer_id: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> str:
        """Add an observer of `fid`: run its added hook (a raise rejects it),
        then file it under its keys. Returns the observer id."""
        self._observer_seq += 1
        obs = FakeObserver(
            id=observer_id or f"observer-{self._observer_seq}",
            function_id=fid,
            args=args if args is not None else {"session_id": session_id},
            target=target or consumer_fn(fid),
            metadata=metadata,
        )
        hook = self.added_hooks.get(fid)
        if hook is not None:
            out = hook(obs)
            if inspect.isawaitable(out):
                await out
        observer_keys, _ = self.match_keys.get(fid, (None, None))
        if observer_keys is not None:
            obs.keys = tuple(_as_keys(observer_keys(obs)))
        active = self.observers.setdefault(fid, [])
        active[:] = [o for o in active if o.id != obs.id] + [obs]
        return obs.id

    def observe_feed(self, fid: str, session_id: str | None = None, **kw: Any) -> str:
        return asyncio.run(self.observe_feed_async(fid, session_id, **kw))

    def unobserve(self, fid: str, observer_id: str) -> None:
        active = self.observers.get(fid, [])
        gone = [o for o in active if o.id == observer_id]
        self.observers[fid] = [o for o in active if o.id != observer_id]
        hook = self.removed_hooks.get(fid)
        for obs in gone:
            if hook is not None:
                out = hook(obs)
                if inspect.isawaitable(out):
                    asyncio.run(out)

    def is_observed(self, fid: str, call: dict[str, Any] | None = None) -> bool:
        active = self.observers.get(fid, [])
        if not active:
            return False
        if call is None or fid not in self.match_keys:
            return True
        return bool(self._matching(fid, call.get("input")))

    def _matching(self, fid: str, input_: Any) -> list[FakeObserver]:
        active = self.observers.get(fid, [])
        if fid not in self.match_keys:
            return list(active)
        _, call_keys = self.match_keys[fid]
        keys = set(_as_keys(call_keys(_Call(input=input_))))
        return [o for o in active if keys.intersection(o.keys)]

    # -- what targets received -----------------------------------------------

    def feed_frames(self, fid: str, target: str | None = None) -> list[dict[str, Any]]:
        """Events delivered to `target` (default: the consumer `observe_feed` uses)."""
        target = target or consumer_fn(fid)
        return [d["payload"] for d in self.deliveries if d["function_id"] == fid and d["target"] == target]

    def feed_events(self, fid: str) -> list[dict[str, Any]]:
        return [p["event"] for p in self.feed_frames(fid)]


def _as_keys(keys: Any) -> tuple[str, ...]:
    return (keys,) if isinstance(keys, str) else tuple(keys)


def consumer_fn(fid: str) -> str:
    return f"consumer::{fid.replace('::', '-')}"


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
