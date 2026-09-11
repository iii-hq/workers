/**
 * Live subscription to the harness's async turn-boundary triggers, scoped to
 * one session. Replaces the `agent::events` `agent_end` signal: the Rust
 * harness emits `harness::turn-completed` when a turn reaches a terminal
 * status (and `harness::turn-started` at the first loop step).
 *
 * Binding follows the standard two-step pattern (register a browser-local
 * handler, then bind a trigger of that type to it), with the `iii::` handler
 * prefix so delivery spans are tagged internal and stay out of the Traces
 * view — the same convention as `lib/sessions/events.ts`. The engine evaluates
 * the `{ session_id }` config filter, so a browser only sees its own turn's
 * boundaries; delivery is fire-and-forget and at-least-once.
 */

import type { IiiClient } from '@/lib/iii-client'
import type {
  MessageQueuedEvent,
  TriggersChangedEvent,
  TurnCompletedEvent,
  TurnStartedEvent,
} from '@/types/iii-agent-event'

const TURN_COMPLETED_FN = 'iii::console::turn_completed'
const TURN_STARTED_FN = 'iii::console::turn_started'
const MESSAGE_QUEUED_FN = 'iii::console::message_queued'
const TRIGGERS_CHANGED_FN = 'iii::console::triggers_changed'
const TURN_COMPLETED_TRIGGER = 'harness::turn-completed'
const TURN_STARTED_TRIGGER = 'harness::turn-started'
const MESSAGE_QUEUED_TRIGGER = 'harness::message-queued'
const TRIGGERS_CHANGED_TRIGGER = 'harness::triggers-changed'

type ClientSubset = Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>

export interface TurnEventHandlers {
  onCompleted: (event: TurnCompletedEvent) => void
  /** Optional: most consumers only need the terminal event. */
  onStarted?: (event: TurnStartedEvent) => void
}

function bind<P>(
  client: ClientSubset,
  handlerFn: string,
  triggerType: string,
  sessionId: string,
  onEvent: (payload: P) => void,
): () => void {
  const off = client.on(handlerFn, (payload: unknown) => {
    if (!payload || typeof payload !== 'object') return
    // The handler id is shared by every live stream (client.on fans out);
    // the engine-side trigger filter is per-binding, so another session's
    // events also land here — drop them.
    const sid = (payload as { session_id?: unknown }).session_id
    if (typeof sid === 'string' && sid !== sessionId) return
    onEvent(payload as P)
  })
  // `on()` registers under `<fn>::<browserId>`; the trigger targets that id.
  const offTrigger = client.registerTrigger({
    type: triggerType,
    function_id: `${handlerFn}::${client.browserId}`,
    config: { session_id: sessionId },
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

/**
 * Bind `harness::turn-completed` (and optionally `harness::turn-started`) for
 * one session. Returns a cleanup that unregisters both handler and trigger.
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
      TURN_COMPLETED_FN,
      TURN_COMPLETED_TRIGGER,
      sessionId,
      handlers.onCompleted,
    ),
  )
  if (handlers.onStarted) {
    offs.push(
      bind<TurnStartedEvent>(
        client,
        TURN_STARTED_FN,
        TURN_STARTED_TRIGGER,
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
 * Bind `harness::message-queued` for one session: fires when a message parks
 * in the server-side queue mid-stream (another tab's send, a subagent or
 * subscription notification). A refresh signal — consumers refetch
 * `harness::status` → `queued`, which stays idempotent under the trigger's
 * at-least-once delivery. Returns a cleanup.
 */
export function startQueuedEventsSubscription(
  client: ClientSubset,
  sessionId: string,
  onQueued: (event: MessageQueuedEvent) => void,
): () => void {
  return bind<MessageQueuedEvent>(
    client,
    MESSAGE_QUEUED_FN,
    MESSAGE_QUEUED_TRIGGER,
    sessionId,
    onQueued,
  )
}

/**
 * Bind `harness::triggers-changed` for one session: fires when the session's
 * trigger-binding set or a fire count changes (registration, unregistration
 * from any tab, a fire, expiry, GC). A doorbell — consumers refetch
 * `harness::triggers::list`, which stays idempotent under the trigger's
 * at-least-once delivery. Returns a cleanup.
 */
export function startTriggersChangedSubscription(
  client: ClientSubset,
  sessionId: string,
  onChanged: (event: TriggersChangedEvent) => void,
): () => void {
  return bind<TriggersChangedEvent>(
    client,
    TRIGGERS_CHANGED_FN,
    TRIGGERS_CHANGED_TRIGGER,
    sessionId,
    onChanged,
  )
}
