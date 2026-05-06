import { useEffect, useReducer } from "react";
import { applyEvent } from "./reducer";
import {
  INITIAL_STREAM_STATE,
  type AgentEvent,
  type StreamState,
} from "./types";

type Action = { kind: "event"; event: AgentEvent } | { kind: "reset" };

function streamReducer(state: StreamState, action: Action): StreamState {
  if (action.kind === "reset") return INITIAL_STREAM_STATE;
  return applyEvent(state, action.event);
}

export function useAgentStream(sessionId: string | null): StreamState {
  const [state, dispatch] = useReducer(streamReducer, INITIAL_STREAM_STATE);

  useEffect(() => {
    if (!sessionId) {
      dispatch({ kind: "reset" });
      return;
    }
    dispatch({ kind: "reset" });
    const url = `/bridge/events?session_id=${encodeURIComponent(sessionId)}`;
    const es = new EventSource(url);
    es.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data) as AgentEvent;
        dispatch({ kind: "event", event: data });
      } catch (err) {
        console.warn("bad SSE frame", err);
      }
    };
    es.onerror = () => {
      // EventSource auto-reconnects; nothing to do unless we want to surface a
      // status pill. Leave as-is for v1.
    };
    return () => es.close();
  }, [sessionId]);

  return state;
}
