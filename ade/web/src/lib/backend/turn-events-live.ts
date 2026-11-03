/**
 * Live observers of the harness's turn-boundary events, scoped to one
 * session. Replaces the `agent::events` `agent_end` signal: the Rust harness
 * calls `harness::on-turn-completed` when a turn reaches a terminal status
 * (and `harness::on-turn-started` at the first loop step).
 *
 * Each observer follows the standard two-step pattern (register a
 * browser-local handler, then observe the function with that handler as the
 * target), with the `iii::` handler prefix so delivery spans are tagged
 * internal and stay out of the Traces view — the same convention as
 * `lib/sessions/events.ts`. iii applies the `input.session_id` filter before
 * delivery, so a browser only sees its own turn's boundaries; delivery is
 * fire-and-forget and at-least-once.
 */

import type { IiiClient } from '@/lib/iii-client'
import type {
  MessageQueuedEvent,
  ObserversChangedEvent,
  TurnCompletedEvent,
  TurnStartedEvent,
} from '@/types/iii-agent-event'

const TURN_COMPLETED_HANDLER = 'iii::console::turn_completed'
const TURN_STARTED_HANDLER = 'iii::console::turn_started'
const MESSAGE_QUEUED_HANDLER = 'iii::console::message_queued'
const OBSERVERS_CHANGED_HANDLER = 'iii::console::observers_changed'
const ON_TURN_COMPLETED_FN = 'harness::on-turn-completed'
const ON_TURN_STARTED_FN = 'harness::on-turn-started'
const ON_MESSAGE_QUEUED_FN = 'harness::on-message-queued'
const ON_OBSERVERS_CHANGED_FN = 'harness::on-observers-changed'

type ClientSubset = Pick<IiiClient, 'browserId' | 'on' | 'observe'>

export interface TurnEventHandlers {
  onCompleted: (event: TurnCompletedEvent) => void
  /** Optional: most consumers only need the terminal event. */
  onStarted?: (event: TurnStartedEvent) => void
}

function bind<P>(
  client: ClientSubset,
  handlerFn: string,
  functionId: string,
  sessionId: string,
  onEvent: (payload: P) => void,
): () => void {
  const off = client.on(handlerFn, (payload: unknown) => {
    if (!payload || typeof payload !== 'object') return
    // The handler id is shared by every live stream (client.on fans out);
    // each observer's filter names one session, so another session's
    // events also land here — drop them.
    const sid = (payload as { session_id?: unknown }).session_id
    if (typeof sid === 'string' && sid !== sessionId) return
    onEvent(payload as P)
  })
  // `on()` registers under `<fn>::<browserId>`; the observer targets that id.
  const offObserver = client.observe(
    {
      function_id: functionId,
      filters: { 'input.session_id': { $eq: sessionId } },
    },
    `${handlerFn}::${client.browserId}`,
  )
  return () => {
    off()
    try {
      offObserver()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

/**
 * Observe `harness::on-turn-completed` (and optionally
 * `harness::on-turn-started`) for one session. Returns a cleanup that drops
 * the handler and disposes the observers.
 */
export function startTurnEventsSubscription(
  client: ClientSubset,
  sessionId: string,
  handlers: TurnEventHandlers,
): () => void {
  const offs: Array<() => void> = []
  offs.push(
    bind<TurnCompletedEvent>(
      client,
      TURN_COMPLETED_HANDLER,
      ON_TURN_COMPLETED_FN,
      sessionId,
      handlers.onCompleted,
    ),
  )
  if (handlers.onStarted) {
    offs.push(
      bind<TurnStartedEvent>(
        client,
        TURN_STARTED_HANDLER,
        ON_TURN_STARTED_FN,
        sessionId,
        handlers.onStarted,
      ),
    )
  }
  return () => {
    for (const off of offs) off()
  }
}

/**
 * Observe `harness::on-message-queued` for one session: called when a
 * message parks in the server-side queue mid-stream (another tab's send, a
 * subagent or an observer's notification). A refresh signal — consumers
 * refetch `harness::status` → `queued`, which stays idempotent under
 * at-least-once delivery. Returns a cleanup.
 */
export function startQueuedEventsSubscription(
  client: ClientSubset,
  sessionId: string,
  onQueued: (event: MessageQueuedEvent) => void,
): () => void {
  return bind<MessageQueuedEvent>(
    client,
    MESSAGE_QUEUED_HANDLER,
    ON_MESSAGE_QUEUED_FN,
    sessionId,
    onQueued,
  )
}

/**
 * Observe `harness::on-observers-changed` for one session: called when the
 * session's observer set or a fire count changes (an agent observing, a
 * dispose from any tab, a fire, expiry, GC). A doorbell — consumers refetch
 * `harness::observers::list`, which stays idempotent under at-least-once
 * delivery. Returns a cleanup.
 */
export function startObserversChangedSubscription(
  client: ClientSubset,
  sessionId: string,
  onChanged: (event: ObserversChangedEvent) => void,
): () => void {
  return bind<ObserversChangedEvent>(
    client,
    OBSERVERS_CHANGED_HANDLER,
    ON_OBSERVERS_CHANGED_FN,
    sessionId,
    onChanged,
  )
}
