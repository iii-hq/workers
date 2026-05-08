import {
  type AgentEvent,
  type AgentMessage,
  type EntryId,
  type PendingApproval,
  type StreamState,
} from "./types";

function maxEntryId(a: EntryId | null, b: EntryId): EntryId {
  if (a === null) return b;
  return b > a ? b : a;
}

function upsertMessage(state: StreamState, entryId: EntryId, message: AgentMessage): StreamState {
  const existed = state.messageMap.has(entryId);
  // Late event for an entry we've moved past and have never seen — drop.
  if (!existed && state.lastEntryId !== null && entryId < state.lastEntryId) {
    return state;
  }
  const next = new Map(state.messageMap);
  if (existed) {
    const prev = next.get(entryId)!;
    // Pick the message with the longer stringified content. Streaming deltas
    // arrive incrementally; we want the most-complete view of the message.
    const merged =
      JSON.stringify(prev.content).length >= JSON.stringify(message.content).length
        ? prev
        : message;
    next.set(entryId, merged);
  } else {
    next.set(entryId, message);
  }
  return {
    ...state,
    messageMap: next,
    messageOrder: existed ? state.messageOrder : [...state.messageOrder, entryId],
    lastEntryId: maxEntryId(state.lastEntryId, entryId),
  };
}

function unkeyedKey(m: AgentMessage): string {
  // Content-hash key. Same role + timestamp + content-length collapse to one
  // entry, so reconnect replays and message_end echoes don't duplicate.
  // Matches the pre-Phase-B dedupe approach.
  return `${m.role}:${m.timestamp ?? 0}:${JSON.stringify(m.content).length}`;
}

function pushUnkeyed(state: StreamState, message: AgentMessage): StreamState {
  // Defensive: never let an undefined slot reach consumers (SessionView etc.
  // map over messages and assume each is a real object).
  if (!message || typeof message !== "object" || !("role" in message)) {
    return state;
  }
  const key = unkeyedKey(message);
  if (state.unkeyedMessages.some((m) => unkeyedKey(m) === key)) {
    return state;
  }
  return { ...state, unkeyedMessages: [...state.unkeyedMessages, message] };
}

export function applyEvent(state: StreamState, event: AgentEvent): StreamState {
  switch (event.type) {
    case "agent_start":
      return { ...state, status: "running" };

    case "agent_end": {
      // agent_end carries the canonical full transcript at end-of-turn.
      // Route each item: entry-id-bearing items upsert into messageMap (idempotent),
      // bare items push into unkeyedMessages (deduped by content hash).
      // The dedupe in `pushUnkeyed` is what prevents the user-visible "repeated
      // answers" bug — message_end fires per-message during the turn, then
      // agent_end fires with the full transcript at end; without dedupe we'd
      // see every message twice.
      let s: StreamState = { ...state, status: "ended" };
      for (const item of event.messages) {
        // Tolerant of two shapes: bare AgentMessage (current backend) or
        // {entry_id?, message} (forward-compat). Detect by presence of `role`.
        const looksBare = item && typeof item === "object" && "role" in item;
        if (looksBare) {
          s = pushUnkeyed(s, item as AgentMessage);
        } else {
          const pair = item as { entry_id?: EntryId; message: AgentMessage };
          if (!pair.message) continue;
          if (pair.entry_id !== undefined) {
            s = upsertMessage(s, pair.entry_id, pair.message);
          } else {
            s = pushUnkeyed(s, pair.message);
          }
        }
      }
      return s;
    }

    case "message_start":
    case "message_update":
    case "message_end":
    case "turn_end": {
      if (event.entry_id !== undefined) {
        return upsertMessage(state, event.entry_id, event.message);
      }
      // Backwards-compat: only `message_end`-style "this message is final"
      // events go to unkeyedMessages. Streaming partials without entry_id
      // are dropped — there's nothing to key them by.
      if (event.type === "message_end") {
        return pushUnkeyed(state, event.message);
      }
      return state;
    }

    case "approval_requested": {
      const id =
        event.function_call_id ||
        ("tool_call_id" in event && event.tool_call_id ? event.tool_call_id : "");
      const name =
        event.function_id ||
        ("tool_name" in event && event.tool_name ? event.tool_name : "");
      if (!id) return state;
      const entry: PendingApproval = {
        function_call_id: id,
        function_id: name,
        args: event.args,
        expires_at: event.expires_at,
      };
      if (state.pendingApprovals.some((a) => a.function_call_id === entry.function_call_id)) {
        return state;
      }
      return { ...state, pendingApprovals: [...state.pendingApprovals, entry] };
    }

    case "approval_resolved": {
      const rid =
        event.function_call_id ||
        ("tool_call_id" in event && event.tool_call_id ? event.tool_call_id : "");
      if (!rid) return state;
      return {
        ...state,
        pendingApprovals: state.pendingApprovals.filter((a) => a.function_call_id !== rid),
      };
    }

    case "turn_start":
    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
    case "function_execution_start":
    case "function_execution_update":
    case "function_execution_end":
      return state;

    default:
      return state;
  }
}

export function reduce(state: StreamState, events: AgentEvent[]): StreamState {
  return events.reduce(applyEvent, state);
}

/** Compose visible messages in order: keyed messages first (in messageOrder),
 *  then unkeyed messages appended in arrival order. */
export function visibleMessages(state: StreamState): AgentMessage[] {
  const keyed = state.messageOrder
    .map((id) => state.messageMap.get(id))
    .filter((m): m is AgentMessage => m !== undefined);
  return [...keyed, ...state.unkeyedMessages];
}
