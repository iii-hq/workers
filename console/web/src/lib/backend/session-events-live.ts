/**
 * Direct subscription to a session's `agent::events` stream, replacing the
 * harness fanout hop (`ui::subscribe` → per-browser `ui::session::event` push).
 *
 * The browser registers a local handler and binds a SCOPED engine stream
 * trigger (`config.group_id = session_id`) to it. The engine matches stream
 * triggers by `(stream_name, group_id, item_id)` and delivers only matching
 * frames straight to this browser's WS connection (engine
 * `stream.rs::invoke_triggers`), so a browser receives exactly its own
 * session's events — without the harness re-pushing them, and with no
 * `harness::fanout::*` / per-browser `ui::session::event` spans.
 *
 * The handler is named with the `iii::` prefix (`is_iii_builtin_function_id`),
 * so the spans produced by DELIVERING this trigger are tagged
 * `iii.function.kind=internal` — hidden from the Traces view by the default
 * `include_internal:false` query and skipped by the engine's trigger/stream
 * loop-break, matching the `traces-stream.ts` approach. Without this, every
 * delivery would flood the trace list with `session_event` spans.
 *
 * The iii-browser-sdk replays both registered functions and triggers on
 * reconnect (see `onSocketOpen`), so the trigger re-binds automatically; the
 * chat backend re-seeds turn state on start via `turn::get_state`, mirroring
 * the pre-existing fanout behavior (no per-event replay on reconnect).
 */

import type { IiiClient } from '@/lib/iii-client'
import type { AgentEvent } from '@/types/iii-agent-event'

/** iii:: prefix → engine-internal → delivery spans hidden + trigger loop-break skip. */
const SESSION_EVENT_FN = 'iii::console::session_event'
/** The firehose stream the harness writes every agent event onto. */
const EVENTS_STREAM = 'agent::events'

/**
 * Pull the AgentEvent out of a raw `agent::events` stream frame, scoped to one
 * session. The engine serializes `StreamWrapperMessage` as
 * `{ groupId, event: { data }, … }`; some shapes carry a flat `data`. This
 * mirrors the extraction the (now-removed) harness `agent::events` fanout
 * pump performed before browsers subscribed to the stream directly.
 *
 * Returns null when the frame is malformed, carries no extractable event, or
 * is addressed to a different session. The stream trigger is already
 * group-scoped, so the session check is defense-in-depth against mis-delivery
 * and preserves the strict `session_id` guard the fanout path enforced.
 */
export function extractSessionEvent(
  frame: unknown,
  sessionId: string,
): AgentEvent | null {
  if (!frame || typeof frame !== 'object') return null
  const obj = frame as Record<string, unknown>

  const groupId =
    (typeof obj.groupId === 'string' && obj.groupId) ||
    (typeof obj.group_id === 'string' && obj.group_id) ||
    null
  if (groupId !== sessionId) return null

  const wrapper =
    obj.event && typeof obj.event === 'object'
      ? (obj.event as Record<string, unknown>)
      : null
  const inner = wrapper && 'data' in wrapper ? wrapper.data : (obj.data ?? null)
  if (!inner || typeof inner !== 'object') return null
  return inner as AgentEvent
}

/**
 * Register the handler + a `agent::events` stream trigger scoped to
 * `sessionId`, delivering each extracted AgentEvent to `onEvent`. Returns a
 * cleanup that unregisters both (replacing the old `ui::unsubscribe`).
 */
export function startSessionEventsSubscription(
  client: Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>,
  sessionId: string,
  onEvent: (event: AgentEvent) => void,
): () => void {
  const off = client.on(SESSION_EVENT_FN, (frame: unknown) => {
    const event = extractSessionEvent(frame, sessionId)
    if (event) onEvent(event)
  })

  // `on()` registers under `<fn>::<browserId>`; the trigger must target that id.
  const functionId = `${SESSION_EVENT_FN}::${client.browserId}`
  const offTrigger = client.registerTrigger({
    type: 'stream',
    function_id: functionId,
    config: { stream_name: EVENTS_STREAM, group_id: sessionId },
  })

  return () => {
    off()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}
