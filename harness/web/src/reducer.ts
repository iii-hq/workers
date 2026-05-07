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

function pushUnkeyed(state: StreamState, message: AgentMessage): StreamState {
  return { ...state, unkeyedMessages: [...state.unkeyedMessages, message] };
}

export function applyEvent(state: StreamState, event: AgentEvent): StreamState {
  switch (event.type) {
    case "agent_start":
      return { ...state, status: "running" };

    case "agent_end": {
      let s: StreamState = { ...state, status: "ended" };
      for (const pair of event.messages) {
        if (pair.entry_id !== undefined) {
          s = upsertMessage(s, pair.entry_id, pair.message);
        } else {
          s = pushUnkeyed(s, pair.message);
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
      const entry: PendingApproval = {
        tool_call_id: event.tool_call_id,
        tool_name: event.tool_name,
        args: event.args,
        expires_at: event.expires_at,
      };
      if (state.pendingApprovals.some((a) => a.tool_call_id === entry.tool_call_id)) {
        return state;
      }
      return { ...state, pendingApprovals: [...state.pendingApprovals, entry] };
    }

    case "approval_resolved":
      return {
        ...state,
        pendingApprovals: state.pendingApprovals.filter(
          (a) => a.tool_call_id !== event.tool_call_id,
        ),
      };

    case "turn_start":
    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
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
