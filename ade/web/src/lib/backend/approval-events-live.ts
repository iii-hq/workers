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
import type { Conversation } from '@/types/chat'
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

type PendingApprovalIdentity = Pick<
  PendingApprovalRecord,
  'session_id' | 'function_call_id'
>

/** Stable inbox slot shared by each approval stage for one function trigger. */
export function pendingApprovalKey(record: PendingApprovalIdentity): string {
  return `${record.session_id}:${record.function_call_id}`
}

/**
 * Revision within an inbox slot. A generic approval can transition into a
 * filesystem approval without changing its function-trigger id, so deduping by
 * slot alone hides the follow-up prompt until refresh.
 */
export function pendingApprovalRevision(record: PendingApprovalRecord): string {
  return JSON.stringify([
    record.pending_at,
    record.function_id,
    record.kind ?? 'function',
    record.access_request?.requested_root ?? null,
    record.access_request?.attempted_path ?? null,
    record.access_request?.error_code ?? null,
  ])
}

/** Record a pending revision; false means this exact event was already seen. */
export function acceptPendingApprovalRevision(
  seen: Map<string, string>,
  record: PendingApprovalRecord,
): boolean {
  const key = pendingApprovalKey(record)
  const revision = pendingApprovalRevision(record)
  if (seen.get(key) === revision) return false
  seen.set(key, revision)
  return true
}

function bind<P>(
  client: ClientSubset,
  handlerFn: string,
  triggerType: string,
  sessionId: string | undefined,
  onEvent: (payload: P) => void,
): () => void {
  const off = client.on(handlerFn, (payload: unknown) => {
    if (!payload || typeof payload !== 'object') return
    // The handler id is shared by every live stream (client.on fans out);
    // the engine-side trigger filter is per-binding, so another session's
    // events also land here — drop them.
    const sid = (payload as { session_id?: unknown }).session_id
    if (sessionId !== undefined && typeof sid === 'string' && sid !== sessionId)
      return
    onEvent(payload as P)
  })
  const offTrigger = client.registerTrigger({
    type: triggerType,
    function_id: `${handlerFn}::${client.browserId}`,
    config: sessionId ? { session_id: sessionId } : {},
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
  sessionId: string | undefined,
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
  sessionId?: string,
): Promise<PendingApprovalRecord[]> {
  const res = await client.trigger<{ pending?: PendingApprovalRecord[] }>(
    'approval::list-pending',
    sessionId ? { session_id: sessionId } : { limit: 500 },
  )
  return Array.isArray(res?.pending) ? res.pending : []
}

/**
 * True when an approval's owning session is the active conversation or one of
 * its descendants. The pending record's denormalized parent handles the
 * creation race for direct children; the live conversation tree resolves
 * deeper descendants without requiring a root id to be persisted separately.
 */
export function approvalBelongsToConversationTree(
  approval: Pick<PendingApprovalRecord, 'session_id' | 'session_metadata'>,
  rootSessionId: string,
  conversations: ReadonlyArray<Pick<Conversation, 'id' | 'parentId'>>,
): boolean {
  if (approval.session_id === rootSessionId) return true

  const parentById = new Map(
    conversations.map((conversation) => [
      conversation.id,
      conversation.parentId,
    ]),
  )
  const recordParent = approval.session_metadata?.parent_session_id
  let current =
    typeof recordParent === 'string'
      ? recordParent
      : parentById.get(approval.session_id)
  const visited = new Set<string>()
  while (current && !visited.has(current)) {
    if (current === rootSessionId) return true
    visited.add(current)
    current = parentById.get(current)
  }
  return false
}
