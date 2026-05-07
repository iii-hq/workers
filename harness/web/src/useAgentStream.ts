import { useEffect, useReducer } from "react";
import { getIiiClient } from "./iii-client";
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

interface SessionEventEnvelope {
  session_id?: string;
  event?: AgentEvent;
  // For backwards-compat with payloads that put the AgentEvent fields at the
  // top level alongside session_id.
  type?: AgentEvent extends { type: infer T } ? T : string;
  [k: string]: unknown;
}

function extractEvent(
  payload: SessionEventEnvelope,
  sessionId: string,
): AgentEvent | null {
  if (payload.session_id !== sessionId) return null;
  if (payload.event && typeof payload.event === "object") {
    return payload.event as AgentEvent;
  }
  // Fall through: treat the rest of the envelope (minus session_id) as the
  // AgentEvent itself. The harness fanout in this step wraps frames as
  // `{session_id, event}` — but allow flat shapes for forward-compat.
  if (typeof payload.type === "string") {
    const { session_id: _drop, ...rest } = payload;
    void _drop;
    return rest as unknown as AgentEvent;
  }
  return null;
}

/**
 * Subscribe to live agent::events for `sessionId` over the iii WebSocket.
 *
 * Wire protocol:
 * - Browser registers `ui::session::event::<browser_id>` once at startup
 *   (handled by useEffect below; the harness fanout calls our handler with
 *   `{session_id, event}` envelopes and we dispatch into the reducer when
 *   `session_id` matches the active session).
 * - Browser calls `ui::subscribe { browser_id, session_id }` on session
 *   change so the fanout knows which sessions to forward.
 */
export function useAgentStream(sessionId: string | null): StreamState {
  const [state, dispatch] = useReducer(streamReducer, INITIAL_STREAM_STATE);

  useEffect(() => {
    dispatch({ kind: "reset" });
    if (!sessionId) return;

    let cancelled = false;
    let off: (() => void) | undefined;
    let subscribed = false;
    let browserId: string | null = null;

    void (async () => {
      try {
        const client = await getIiiClient();
        if (cancelled) return;
        browserId = client.browserId;

        off = client.on<SessionEventEnvelope>(
          "ui::session::event",
          (payload) => {
            const ev = extractEvent(payload, sessionId);
            if (!ev) return;
            dispatch({ kind: "event", event: ev });
          },
        );

        await client.call("ui::subscribe", {
          browser_id: browserId,
          session_id: sessionId,
        });
        subscribed = true;
      } catch (err) {
        // Connection bootstrap failure: surfaced via useConnection's status
        // pill. Don't throw here — hooks must not crash the tree.
        console.warn("[useAgentStream] subscribe failed", err);
      }
    })();

    return () => {
      cancelled = true;
      off?.();
      if (subscribed && browserId) {
        // Best-effort: tell the fanout to drop the subscription. Failures
        // here are recoverable on the next tick (the fanout treats stale
        // entries as harmless).
        void getIiiClient().then((client) =>
          client
            .call("ui::unsubscribe", {
              browser_id: browserId,
              session_id: sessionId,
            })
            .catch(() => {}),
        );
      }
    };
  }, [sessionId]);

  return state;
}
