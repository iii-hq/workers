/**
 * Live subscription to approval-gate's pending-inbox triggers, scoped to one
 * session, plus the catch-up read. Replaces the `agent::events`
 * `turn_state_changed` / `awaiting_approval[]` mirror: the gate now emits
 * discrete `approval::pending-created` / `approval::pending-resolved` events,
 * and `approval::list-pending` rebuilds the inbox after a reconnect.
 *
 * Same two-step binding + `iii::` handler-prefix convention as
 * `lib/sessions/events.ts` and `turn-events-live.ts`; the engine evaluates the
 * `{ session_id }` config filter.
 */

import type { IiiClient } from '@/lib/iii-client'
import type {
  PendingApprovalRecord,
  PendingResolvedEvent,
} from '@/types/iii-agent-event'

const CREATED_FN = 'iii::console::approval_pending_created'
const RESOLVED_FN = 'iii::console::approval_pending_resolved'
const CREATED_TRIGGER = 'approval::pending-created'
const RESOLVED_TRIGGER = 'approval::pending-resolved'

type ClientSubset = Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>

export interface ApprovalEventHandlers {
  onCreated: (record: PendingApprovalRecord) => void
  onResolved: (event: PendingResolvedEvent) => void
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
 * Bind `approval::pending-created` + `approval::pending-resolved` for one
 * session. Returns a cleanup that unregisters both handlers and triggers.
 */
export function startApprovalEventsSubscription(
  client: ClientSubset,
  sessionId: string,
  handlers: ApprovalEventHandlers,
): () => void {
  const offs = [
    bind<PendingApprovalRecord>(
      client,
      CREATED_FN,
      CREATED_TRIGGER,
      sessionId,
      handlers.onCreated,
    ),
    bind<PendingResolvedEvent>(
      client,
      RESOLVED_FN,
      RESOLVED_TRIGGER,
      sessionId,
      handlers.onResolved,
    ),
  ]
  return () => {
    for (const off of offs) off()
  }
}

/**
 * Catch-up read of the pending inbox for one session — the reconnect path.
 * Returns the held calls (ordered by `pending_at` ascending); empty when the
 * gate is absent or unreachable (best-effort).
 */
export async function listPendingApprovals(
  client: Pick<IiiClient, 'trigger'>,
  sessionId: string,
): Promise<PendingApprovalRecord[]> {
  const res = await client.trigger<{ pending?: PendingApprovalRecord[] }>(
    'approval::list-pending',
    { session_id: sessionId },
  )
  return Array.isArray(res?.pending) ? res.pending : []
}
