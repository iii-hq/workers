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
  TurnCompletedEvent,
  TurnStartedEvent,
} from '@/types/iii-agent-event'

const TURN_COMPLETED_FN = 'iii::console::turn_completed'
const TURN_STARTED_FN = 'iii::console::turn_started'
const TURN_COMPLETED_TRIGGER = 'harness::turn-completed'
const TURN_STARTED_TRIGGER = 'harness::turn-started'

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
    if (payload && typeof payload === 'object') onEvent(payload as P)
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
