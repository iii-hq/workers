import {
  type AgentEvent,
  type AgentMessage,
  type PendingApproval,
  type StreamState,
} from "./types";

function messageKey(m: AgentMessage): string {
  return `${m.role}:${m.timestamp ?? 0}:${JSON.stringify(m.content).length}`;
}

export function applyEvent(state: StreamState, event: AgentEvent): StreamState {
  switch (event.type) {
    case "agent_start":
      return { ...state, status: "running" };
    case "agent_end":
      return { ...state, status: "ended" };
    case "turn_start":
    case "turn_end":
    case "message_start":
    case "message_update":
    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
      return state; // tool_use/tool_result blocks arrive via message_end frames
    case "message_end": {
      const key = messageKey(event.message);
      if (state.messages.some((m) => messageKey(m) === key)) {
        return state;
      }
      return { ...state, messages: [...state.messages, event.message] };
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
    default:
      return state;
  }
}

export function reduce(state: StreamState, events: AgentEvent[]): StreamState {
  return events.reduce(applyEvent, state);
}
