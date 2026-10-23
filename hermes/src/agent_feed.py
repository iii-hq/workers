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

Frames are ephemeral: not stored, not replayed. History lives in the session
records (`hermes::status`, `hermes::sessions::list`) and the turn result.
"""

from __future__ import annotations

import logging
import threading
import uuid
from dataclasses import dataclass
from typing import Any

from iii import TriggerAction
from iii.triggers import TriggerConfig, TriggerHandler

SOURCE = "hermes"
AGENT_EVENT_TYPE = "hermes::agent-event"
RAW_EVENT_TYPE = "hermes::raw-event"
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


class AgentFeed(TriggerHandler[dict[str, Any]]):
    """One owned trigger type: binding table, per-session sequence, delivery."""

    def __init__(self, iii: Any, *, type_id: str, source: str, description: str) -> None:
        self._iii = iii
        self.type_id = type_id
        self.source = source
        self.description = description
        self.epoch = str(uuid.uuid4())
        self._bindings: dict[str, _Binding] = {}
        self._seq_by_session: dict[str, int] = {}
        # Guards the tables only; never held across an await.
        self._lock = threading.Lock()

    def register(self) -> AgentFeed:
        self._iii.register_trigger_type(
            {
                "id": self.type_id,
                "description": self.description,
                "trigger_request_format": FEED_CONFIG_FORMAT,
                "call_request_format": FEED_PAYLOAD_FORMAT,
            },
            self,
        )
        return self

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
    """Register `hermes::agent-event` and `hermes::raw-event` on `iii`."""
    agent = AgentFeed(iii, type_id=AGENT_EVENT_TYPE, source=SOURCE, description=AGENT_EVENT_DESCRIPTION)
    raw = AgentFeed(iii, type_id=RAW_EVENT_TYPE, source=SOURCE, description=RAW_EVENT_DESCRIPTION)
    return Feeds(agent=agent.register(), raw=raw.register())
