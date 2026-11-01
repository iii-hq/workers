"""Owned event feeds: `hermes::on-agent-event` and `hermes::on-raw-event`.

The worker registers two private functions at startup. Each handler returns
the frame envelope it is called with, which is what observers observe; the
worker emits a frame by calling the function. The contract is shared by every
agent worker (P3-1 design, section 2):

- observer args `{session_id}`: a required, non-empty string of at most 512
  characters and nothing else (`FEED_ARGS_FORMAT`; the engine refuses other
  args with `INVALID_ARGS`). At most `MAX_OBSERVERS` observers per function;
  one more is rejected with `OBSERVER_LIMIT`. Metadata a consumer wants back
  goes in its observe options and reaches its target as the metadata sidecar.
- event `{session_id, event_id, seq, epoch, source, event}`: `event` is the
  unchanged frame, `seq` is contiguous per (feed, session, epoch) from 0,
  `epoch` a per-process uuid, `event_id` = `<session_id>-<epoch>-<seq:08>`.
- delivery: one fire-and-forget (Void) call of the function per frame. A match
  hook files each observer under its `session_id` and a frame matches the
  observers of its own, so iii delivers it only to them. A frame of a session
  nobody observes is never built, though its `seq` still advances. A failed
  call is logged and swallowed.

Frames are ephemeral: not stored, not replayed. History lives in the session
records (`hermes::status`, `hermes::sessions::list`) and the turn result.
"""

from __future__ import annotations

import logging
import threading
import uuid
from dataclasses import dataclass
from typing import Any

from iii import CallAction, ObserveRejection

SOURCE = "hermes"
ON_AGENT_EVENT_FN = "hermes::on-agent-event"
ON_RAW_EVENT_FN = "hermes::on-raw-event"
MAX_OBSERVERS = 256
MAX_SESSION_ID_LEN = 512
OBSERVER_LIMIT = "OBSERVER_LIMIT"

AGENT_EVENT_DESCRIPTION = (
    "Normalized AgentEvent frames of one Hermes session (turn_end, agent_end). "
    "Args {session_id}; event {session_id, event_id, seq, epoch, source, event}. "
    "Ephemeral: order by (epoch, seq), dedup by event_id."
)
RAW_EVENT_DESCRIPTION = (
    "Raw Hermes frames of one session: the run `result` frame and inbound gateway deliveries. "
    "Args {session_id}; event {session_id, event_id, seq, epoch, source, event}. "
    "Ephemeral: order by (epoch, seq), dedup by event_id."
)

FEED_ARGS_FORMAT = {
    "type": "object",
    "properties": {
        "session_id": {
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_SESSION_ID_LEN,
            "description": "Only this session's frames are delivered.",
        },
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


def observer_keys(obs: Any) -> tuple[str, ...]:
    """The session an observer is filed under: its `session_id` arg."""
    session_id = (obs.args or {}).get("session_id")
    return (session_id,) if isinstance(session_id, str) else ()


def call_keys(call: Any) -> tuple[str, ...]:
    """The session a frame matches: the `session_id` of the call's input."""
    payload = call.input if isinstance(call.input, dict) else {}
    session_id = payload.get("session_id")
    return (session_id,) if isinstance(session_id, str) else ()


async def _identity(payload: dict[str, Any]) -> dict[str, Any]:
    """The feed function's handler: observers observe the envelope it returns.
    Async, so a local call runs on the SDK loop rather than a thread of its own."""
    return payload


class AgentFeed:
    """One owned feed function: per-session sequence, observer cap, emit."""

    def __init__(self, iii: Any, *, function_id: str, source: str, description: str) -> None:
        self._iii = iii
        self.function_id = function_id
        self.source = source
        self.description = description
        self.epoch = str(uuid.uuid4())
        self._observers: set[str] = set()
        self._seq_by_session: dict[str, int] = {}
        # Guards the counters and the observer set only; never held across an await.
        self._lock = threading.Lock()

    def register(self) -> AgentFeed:
        """Set the hooks by id, then register the private feed function."""
        self._iii.on_observer_added(self.function_id, self.admit)
        self._iii.on_observer_removed(self.function_id, self.release)
        self._iii.on_observer_match(self.function_id, observer=observer_keys, call=call_keys)
        self._iii.register_function(
            self.function_id,
            _identity,
            description=self.description,
            request_format=FEED_PAYLOAD_FORMAT,
            response_format=FEED_PAYLOAD_FORMAT,
            args_format=FEED_ARGS_FORMAT,
            private=True,
        )
        return self

    def admit(self, obs: Any) -> None:
        """`on_observer_added`: count the observer; past the cap, reject it.
        An id already counted (the engine re-announcing it) is accepted again."""
        with self._lock:
            if obs.id not in self._observers and len(self._observers) >= MAX_OBSERVERS:
                raise ObserveRejection(OBSERVER_LIMIT, f"{self.function_id}: observer limit ({MAX_OBSERVERS}) reached")
            self._observers.add(obs.id)

    def release(self, obs: Any) -> None:
        """`on_observer_removed`: stop counting the observer."""
        with self._lock:
            self._observers.discard(obs.id)

    def observer_count(self) -> int:
        with self._lock:
            return len(self._observers)

    async def emit(self, session_id: str, event: dict[str, Any]) -> bool:
        """Emit one frame; True when a call was made and succeeded."""
        with self._lock:
            seq = self._seq_by_session.get(session_id, 0)
            self._seq_by_session[session_id] = seq + 1
        if not self._iii.is_observed(self.function_id, {"input": {"session_id": session_id}}):
            return False
        payload = {
            "session_id": session_id,
            "event_id": f"{session_id}-{self.epoch}-{seq:08d}",
            "seq": seq,
            "epoch": self.epoch,
            "source": self.source,
            "event": event,
        }
        try:
            await self._iii.call_async(
                {"function_id": self.function_id, "payload": payload, "action": CallAction.Void()}
            )
            return True
        except Exception as exc:  # noqa: BLE001 - a gone consumer never fails the turn
            log.warning("%s: call failed: %s", self.function_id, exc)
            return False


@dataclass(frozen=True)
class Feeds:
    agent: AgentFeed
    raw: AgentFeed


def create_feeds(iii: Any) -> Feeds:
    """Register `hermes::on-agent-event` and `hermes::on-raw-event` on `iii`."""
    agent = AgentFeed(iii, function_id=ON_AGENT_EVENT_FN, source=SOURCE, description=AGENT_EVENT_DESCRIPTION)
    raw = AgentFeed(iii, function_id=ON_RAW_EVENT_FN, source=SOURCE, description=RAW_EVENT_DESCRIPTION)
    return Feeds(agent=agent.register(), raw=raw.register())
