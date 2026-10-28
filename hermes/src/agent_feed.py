"""Owned event feeds: `hermes::agent-event` and `hermes::raw-event`.

The worker registers two trigger types at startup and delivers each frame to
the functions bound to the frame's session. They replace the old iii-stream
feeds (`agent::events` / `hermes::events`); the contract is shared by every
agent worker (P3-1 design, section 2):

- binding config `{session_id, metadata?}`: `session_id` is a required,
  non-empty string of at most 512 characters, `metadata` an optional object.
  Unknown keys, a bad `session_id` or non-object `metadata` reject the binding.
  At most `MAX_BINDINGS` bindings per trigger type; re-registering an existing
  binding id replaces it.
- delivery payload `{session_id, event_id, seq, epoch, source, event}`: `event`
  is the unchanged frame, `seq` is contiguous per (feed, session, epoch) from 0,
  `epoch` a per-process uuid, `event_id` = `<session_id>-<epoch>-<seq:08>`.
- delivery: one fire-and-forget (Void) trigger call per matching binding,
  sequential, forwarding the binding's namespace and metadata (config metadata
  wins); a failed delivery is logged and swallowed. No binding, no call.

On an iii 0.25 engine each feed is also an owner-delivered observable function
(`hermes::on-agent-event`, `hermes::on-raw-event`): an observer with args
`{session_id, metadata?}` (the binding config) gets the same payload through
`obs.call`, filed in the same binding table (same numbering, same limit). The
trigger types stay for 0.24 engines and existing consumers; invoking an `on-*`
function directly is refused with FORBIDDEN.

Frames are ephemeral: not stored, not replayed. History lives in the session
records (`hermes::status`, `hermes::sessions::list`) and the turn result.
"""

from __future__ import annotations

import logging
import threading
import uuid
import warnings
from dataclasses import dataclass
from typing import Any

from iii import InvocationError, ObserveHooks, ObserveRejection, TriggerAction
from iii.triggers import TriggerConfig, TriggerHandler

SOURCE = "hermes"
AGENT_EVENT_TYPE = "hermes::agent-event"
RAW_EVENT_TYPE = "hermes::raw-event"
# The observable functions (iii 0.25) next to the two trigger types.
AGENT_EVENT_OBSERVABLE = "hermes::on-agent-event"
RAW_EVENT_OBSERVABLE = "hermes::on-raw-event"
MAX_BINDINGS = 256
MAX_SESSION_ID_LEN = 512
CONFIG_KEYS = frozenset({"session_id", "metadata"})

AGENT_EVENT_DESCRIPTION = (
    "Normalized AgentEvent frames of one Hermes session (turn_end, agent_end). "
    "Config {session_id, metadata?}; payload {session_id, event_id, seq, epoch, source, event}. "
    "Ephemeral: order by (epoch, seq), dedup by event_id."
)
RAW_EVENT_DESCRIPTION = (
    "Raw Hermes frames of one session: the run `result` frame and inbound gateway deliveries. "
    "Config {session_id, metadata?}; payload {session_id, event_id, seq, epoch, source, event}. "
    "Ephemeral: order by (epoch, seq), dedup by event_id."
)

FEED_CONFIG_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_SESSION_ID_LEN,
            "description": "Only this session's frames are delivered.",
        },
        "metadata": {"type": "object", "description": "Echoed to the handler; wins over binding metadata."},
    },
    "required": ["session_id"],
    "additionalProperties": False,
}
# The observers' args: the binding config, validated by the engine (INVALID_ARGS).
FEED_ARGS_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_SESSION_ID_LEN,
            "description": "The session whose frames this observer receives.",
        },
        "metadata": {
            "type": "object",
            "description": "Accepted for parity with the trigger config; an observer gets its sidecar "
            "from its own `metadata` option.",
        },
    },
    "required": ["session_id"],
    "additionalProperties": False,
}
# Observer delivery outcomes that are not failures: nothing was sent.
_SKIPPED_CODES = frozenset({"FILTERS_NOT_MATCHED", "OBSERVER_NOT_ACTIVE"})
FEED_PAYLOAD_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {"type": "string"},
        "event_id": {"type": "string"},
        "seq": {"type": "integer"},
        "epoch": {"type": "string"},
        "source": {"type": "string"},
        "event": {"type": "object"},
    },
    "required": ["session_id", "event_id", "seq", "epoch", "source", "event"],
}

log = logging.getLogger("hermes.agent_feed")


def validate_feed_config(type_id: str, raw: Any) -> dict[str, Any]:
    """Validate a binding config; raise ValueError (rejects the binding)."""
    if not isinstance(raw, dict):
        raise ValueError(f"{type_id}: config must be an object with a session_id")
    unknown = sorted(str(k) for k in raw if k not in CONFIG_KEYS)
    if unknown:
        raise ValueError(f"{type_id}: unknown config key(s): {', '.join(unknown)}")
    session_id = raw.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        raise ValueError(f"{type_id}: config.session_id must be a non-empty string")
    if len(session_id) > MAX_SESSION_ID_LEN:
        raise ValueError(f"{type_id}: config.session_id exceeds {MAX_SESSION_ID_LEN} characters")
    config: dict[str, Any] = {"session_id": session_id}
    metadata = raw.get("metadata")
    if metadata is not None:
        if not isinstance(metadata, dict):
            raise ValueError(f"{type_id}: config.metadata must be an object")
        config["metadata"] = metadata
    return config


@dataclass(frozen=True)
class _Binding:
    id: str
    function_id: str
    namespace: str | None
    metadata: Any
    config: dict[str, Any]
    # Set for an observer of the observable function (iii 0.25), None for a trigger binding.
    observer: Any = None


def _observer_key(observer_id: str) -> str:
    """Observers share the binding table under a key no trigger binding id uses."""
    return f"observer:{observer_id}"


class AgentFeed(TriggerHandler[dict[str, Any]]):
    """One owned trigger type and its observable function: binding table, per-session sequence, delivery."""

    def __init__(
        self, iii: Any, *, type_id: str, source: str, description: str, observable_id: str | None = None
    ) -> None:
        self._iii = iii
        self.type_id = type_id
        self.observable_id = observable_id
        self.source = source
        self.description = description
        self.epoch = str(uuid.uuid4())
        self._bindings: dict[str, _Binding] = {}
        self._seq_by_session: dict[str, int] = {}
        # Guards the tables only; never held across an await.
        self._lock = threading.Lock()

    def register(self) -> AgentFeed:
        # Deprecated in iii 0.25 and kept registered for 0.24 engines and existing bindings.
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            self._iii.register_trigger_type(
                {
                    "id": self.type_id,
                    "description": self.description,
                    "trigger_request_format": FEED_CONFIG_FORMAT,
                    "call_request_format": FEED_PAYLOAD_FORMAT,
                },
                self,
            )
        if self.observable_id is not None:
            # The iii 0.25 observable function next to the trigger type: owner-delivered,
            # its observers filed in the same binding table, so one emit reaches bindings
            # and observers alike. Events are routed in-process; invoking it is refused.
            self._iii.register_function(
                self.observable_id,
                self.refuse_invocation,
                description=(
                    f"{self.description} Observe with iii.observe({{function_id: '{self.observable_id}', "
                    "args: {session_id}}, target) (iii 0.25); invoking it is refused with FORBIDDEN."
                ),
                observe=ObserveHooks(
                    delivery="owner",
                    args_format=FEED_ARGS_FORMAT,
                    on_observer_added=self.on_observer_added,
                    on_observer_removed=self.on_observer_removed,
                ),
            )
        return self

    async def refuse_invocation(self, data: Any) -> None:
        """The observable function's handler: it is an event source, never invoked."""
        raise InvocationError(
            code="FORBIDDEN",
            message=(
                f"{self.observable_id} is an event source: observe it (iii.observe({{function_id: "
                f"'{self.observable_id}', args: {{session_id}}}}, target)); only the hermes worker emits its events"
            ),
            function_id=self.observable_id,
        )

    async def on_observer_added(self, obs: Any) -> None:
        name = self.observable_id or self.type_id
        try:
            feed_config = validate_feed_config(name, obs.args)
        except ValueError as exc:
            raise ObserveRejection("INVALID_ARGS", str(exc)) from exc
        key = _observer_key(obs.id)
        binding = _Binding(
            id=key, function_id=obs.target, namespace=None, metadata=None, config=feed_config, observer=obs
        )
        with self._lock:
            if key not in self._bindings and len(self._bindings) >= MAX_BINDINGS:
                raise ObserveRejection("BINDING_LIMIT", f"{name}: binding limit ({MAX_BINDINGS}) reached")
            self._bindings[key] = binding

    async def on_observer_removed(self, obs: Any) -> None:
        with self._lock:
            self._bindings.pop(_observer_key(obs.id), None)

    async def register_trigger(self, config: TriggerConfig[dict[str, Any]]) -> None:
        feed_config = validate_feed_config(self.type_id, config.config)
        binding = _Binding(
            id=config.id,
            function_id=config.function_id,
            namespace=config.namespace,
            metadata=config.metadata,
            config=feed_config,
        )
        with self._lock:
            if binding.id not in self._bindings and len(self._bindings) >= MAX_BINDINGS:
                raise ValueError(f"{self.type_id}: binding limit ({MAX_BINDINGS}) reached")
            self._bindings[binding.id] = binding

    async def unregister_trigger(self, config: TriggerConfig[dict[str, Any]]) -> None:
        with self._lock:
            self._bindings.pop(config.id, None)

    def binding_count(self) -> int:
        with self._lock:
            return len(self._bindings)

    async def emit(self, session_id: str, event: dict[str, Any]) -> int:
        """Deliver one frame to the session's bindings; return the delivered count."""
        with self._lock:
            seq = self._seq_by_session.get(session_id, 0)
            self._seq_by_session[session_id] = seq + 1
            targets = [b for b in self._bindings.values() if b.config["session_id"] == session_id]
        if not targets:
            return 0
        payload = {
            "session_id": session_id,
            "event_id": f"{session_id}-{self.epoch}-{seq:08d}",
            "seq": seq,
            "epoch": self.epoch,
            "source": self.source,
            "event": event,
        }
        delivered = 0
        for binding in targets:
            if binding.observer is not None:
                try:
                    await binding.observer.call_async(payload)
                    delivered += 1
                except InvocationError as exc:
                    if exc.code not in _SKIPPED_CODES:
                        log.warning("%s: delivery to %s failed: %s", self.observable_id, binding.function_id, exc)
                except Exception as exc:  # noqa: BLE001 - a gone observer never fails the turn
                    log.warning("%s: delivery to %s failed: %s", self.observable_id, binding.function_id, exc)
                continue
            request: dict[str, Any] = {
                "function_id": binding.function_id,
                "payload": payload,
                "action": TriggerAction.Void(),
            }
            if binding.namespace is not None:
                request["namespace"] = binding.namespace
            metadata = binding.config.get("metadata", binding.metadata)
            if metadata is not None:
                request["metadata"] = metadata
            try:
                await self._iii.trigger_async(request)
                delivered += 1
            except Exception as exc:  # noqa: BLE001 - a gone consumer never fails the turn
                log.warning("%s: delivery to %s failed: %s", self.type_id, binding.function_id, exc)
        return delivered


@dataclass(frozen=True)
class Feeds:
    agent: AgentFeed
    raw: AgentFeed


def create_feeds(iii: Any) -> Feeds:
    """Register `hermes::agent-event` and `hermes::raw-event` on `iii`, each with its
    observable function (`hermes::on-agent-event`, `hermes::on-raw-event`)."""
    agent = AgentFeed(
        iii,
        type_id=AGENT_EVENT_TYPE,
        source=SOURCE,
        description=AGENT_EVENT_DESCRIPTION,
        observable_id=AGENT_EVENT_OBSERVABLE,
    )
    raw = AgentFeed(
        iii,
        type_id=RAW_EVENT_TYPE,
        source=SOURCE,
        description=RAW_EVENT_DESCRIPTION,
        observable_id=RAW_EVENT_OBSERVABLE,
    )
    return Feeds(agent=agent.register(), raw=raw.register())
