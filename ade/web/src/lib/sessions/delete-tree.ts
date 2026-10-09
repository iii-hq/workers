import { errorCode } from '@iii-dev/console-ui/format'
import { getIiiClient } from '@/lib/iii-client'

export interface DeletionBlocker {
  kind: 'active_processing' | 'unconfirmed_cancellation' | 'unknown_completion'
  session_id: string
  function_id?: string
  call_id?: string
  started_at?: number
}

export interface SessionTreeDeletionSnapshot {
  operation_id: string
  /** Increments only when a failed operation is retried. */
  attempt: number
  session_id: string
  status: 'deleting' | 'completed' | 'failed'
  deleted_session_ids: string[]
  error?: string
  mode?: 'normal' | 'force'
  /** Explicit proof that no notification, cleanup or erase began. */
  data_retained?: boolean
  blockers?: DeletionBlocker[]
  force_eligible?: boolean
  failure_code?: 'blocked' | 'failed' | 'overlapping_deletion'
  existing_deletion?: { operation_id: string; session_id: string }
  /** Not confirmed complete; includes unconfirmed outcomes. */
  remaining_session_ids?: string[]
  /** Subset of remaining whose erase outcome has not been confirmed. */
  unconfirmed_session_ids?: string[]
}

export class SessionTreeDeletionError extends Error {
  constructor(public readonly snapshot: SessionTreeDeletionSnapshot) {
    super(snapshot.error || 'Unable to stop and delete this conversation tree.')
    this.name = 'SessionTreeDeletionError'
  }
}

export function canForceDelete(
  snapshot: SessionTreeDeletionSnapshot | null,
): boolean {
  return (
    !!snapshot &&
    snapshot.status === 'failed' &&
    snapshot.mode === 'normal' &&
    snapshot.failure_code === 'blocked' &&
    snapshot.force_eligible === true &&
    !!snapshot.blockers?.length &&
    !snapshot.unconfirmed_session_ids?.length
  )
}

/** A failed empty result cannot prove enumeration (pre-plan failures use []). */
export function hasDeletionOutcomeCounts(
  snapshot: SessionTreeDeletionSnapshot,
) {
  return (
    snapshot.remaining_session_ids !== undefined &&
    !(
      snapshot.status === 'failed' &&
      snapshot.remaining_session_ids.length === 0
    )
  )
}

/** Older generic failures cannot prove that remaining sessions were retained. */
export function deletionOutcomes(snapshot: SessionTreeDeletionSnapshot) {
  const remaining = snapshot.remaining_session_ids ?? []
  const unconfirmed = snapshot.unconfirmed_session_ids ?? remaining
  return {
    notDeleted: remaining.filter((id) => !unconfirmed.includes(id)),
    unconfirmed,
  }
}

export function deletionFailureMessage(snapshot: SessionTreeDeletionSnapshot) {
  if (snapshot.failure_code === 'overlapping_deletion')
    return 'Another deletion operation owns part of this conversation tree. Review the existing operation; closing this dialog does not cancel it.'
  if (
    snapshot.status === 'failed' &&
    snapshot.failure_code === 'blocked' &&
    snapshot.data_retained === true &&
    !snapshot.deleted_session_ids.length &&
    snapshot.unconfirmed_session_ids?.length === 0
  )
    return 'Deletion is blocked. No conversations were deleted; data retained.'
  return snapshot.failure_code === 'blocked'
    ? 'Deletion is blocked. Remaining conversations are not deleted or unconfirmed; retry is required.'
    : 'Deletion did not complete. Check the confirmed deleted, not deleted and unconfirmed outcomes before retrying.'
}

export function deletionCommandErrorMessage(error: unknown, force: boolean) {
  const code = errorCode(error)?.toLowerCase()
  return force &&
    (code === 'invalid_request' || code === 'harness/invalid_request')
    ? 'Force delete was rejected; nothing changed. Refresh the deletion status before retrying.'
    : null
}

export interface DeleteSessionTreeOptions {
  /** Stops this browser's wait, never the backend operation. */
  signal?: AbortSignal
  timeoutMs?: number
  force?: { operation_id: string; attempt: number }
  /** Read durable state on reopen without implicitly retrying. */
  recover?: boolean
  /** Server-provided identity, read only: no command if absent. */
  reviewOperationId?: string
}

export const DELETE_TREE_WAIT_MS = 120_000
let subscriptionSequence = 0

function isSnapshot(value: unknown): value is SessionTreeDeletionSnapshot {
  if (!value || typeof value !== 'object') return false
  const snapshot = value as SessionTreeDeletionSnapshot
  return (
    typeof snapshot.operation_id === 'string' &&
    snapshot.operation_id.length > 0 &&
    Number.isSafeInteger(snapshot.attempt) &&
    snapshot.attempt >= 1 &&
    typeof snapshot.session_id === 'string' &&
    ['deleting', 'completed', 'failed'].includes(snapshot.status) &&
    Array.isArray(snapshot.deleted_session_ids) &&
    snapshot.deleted_session_ids.every((id) => typeof id === 'string') &&
    (snapshot.error === undefined || typeof snapshot.error === 'string') &&
    (snapshot.mode === undefined ||
      ['normal', 'force'].includes(snapshot.mode)) &&
    (snapshot.data_retained === undefined ||
      typeof snapshot.data_retained === 'boolean') &&
    (snapshot.force_eligible === undefined ||
      typeof snapshot.force_eligible === 'boolean') &&
    (snapshot.failure_code === undefined ||
      ['blocked', 'failed', 'overlapping_deletion'].includes(
        snapshot.failure_code,
      )) &&
    (snapshot.existing_deletion === undefined ||
      (!!snapshot.existing_deletion &&
        snapshot.failure_code === 'overlapping_deletion' &&
        snapshot.force_eligible !== true &&
        typeof snapshot.existing_deletion.operation_id === 'string' &&
        snapshot.existing_deletion.operation_id.length > 0 &&
        typeof snapshot.existing_deletion.session_id === 'string' &&
        snapshot.existing_deletion.session_id.length > 0)) &&
    (snapshot.remaining_session_ids === undefined ||
      (Array.isArray(snapshot.remaining_session_ids) &&
        snapshot.remaining_session_ids.every(
          (id) => typeof id === 'string',
        ))) &&
    (snapshot.unconfirmed_session_ids === undefined ||
      (Array.isArray(snapshot.unconfirmed_session_ids) &&
        snapshot.unconfirmed_session_ids.every(
          (id) => typeof id === 'string',
        ) &&
        snapshot.unconfirmed_session_ids.every(
          (id) =>
            snapshot.remaining_session_ids?.includes(id) &&
            !snapshot.deleted_session_ids.includes(id),
        ))) &&
    (snapshot.blockers === undefined ||
      (Array.isArray(snapshot.blockers) &&
        snapshot.blockers.every(
          (blocker) =>
            !!blocker &&
            [
              'active_processing',
              'unconfirmed_cancellation',
              'unknown_completion',
            ].includes(blocker.kind) &&
            typeof blocker.session_id === 'string' &&
            (blocker.function_id === undefined ||
              typeof blocker.function_id === 'string') &&
            (blocker.call_id === undefined ||
              typeof blocker.call_id === 'string') &&
            (blocker.started_at === undefined ||
              Number.isSafeInteger(blocker.started_at)),
        )))
  )
}

/**
 * One high-level command, one catch-up read, then terminal events only.
 * The backend owns tree traversal, cancellation and idempotency by session id.
 * Never fall back to session::delete, even on an older/unavailable harness.
 */
export function deleteSessionTree(
  sessionId: string,
  {
    signal,
    timeoutMs = DELETE_TREE_WAIT_MS,
    force,
    recover,
    reviewOperationId,
  }: DeleteSessionTreeOptions = {},
): Promise<SessionTreeDeletionSnapshot> {
  return new Promise((resolve, reject) => {
    let settled = false
    let operationId: string | undefined
    let attempt: number | undefined
    const eventKey = (value: SessionTreeDeletionSnapshot) =>
      JSON.stringify([value.operation_id, value.attempt])
    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined
    let offConnection: (() => void) | undefined
    let eventClient: Awaited<ReturnType<typeof getIiiClient>> | undefined
    const refreshed = new WeakSet<object>()
    // A terminal event can beat the command's acceptance response. It is
    // usable only after that response identifies this attempt's operation.
    const earlyTerminals = new Map<string, SessionTreeDeletionSnapshot>()
    const cleanup = () => {
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      earlyTerminals.clear()
      offConnection?.()
      try {
        offTrigger?.()
      } finally {
        offHandler?.()
      }
    }
    const finish = (
      snapshot?: SessionTreeDeletionSnapshot,
      error?: unknown,
    ) => {
      if (settled) return
      settled = true
      try {
        cleanup()
      } catch {
        // A disposed SDK must not mask the operation's outcome.
      }
      if (snapshot) resolve(snapshot)
      else reject(error)
    }
    const abort = () =>
      finish(
        undefined,
        new Error('Stopped waiting. Backend deletion may still be running.'),
      )
    const timer = setTimeout(
      () =>
        finish(
          undefined,
          new Error(
            'Timed out waiting for deletion. The backend may still be running; retry to check or resume the same deletion.',
          ),
        ),
      timeoutMs,
    )
    const accept = (value: unknown) => {
      if (settled || !isSnapshot(value) || value.session_id !== sessionId)
        return
      if (!operationId) {
        if (value.status !== 'deleting')
          earlyTerminals.set(eventKey(value), value)
        return
      }
      if (value.operation_id !== operationId || value.attempt !== attempt)
        return
      if (
        value.status === 'failed' &&
        value.force_eligible &&
        !refreshed.has(value) &&
        eventClient
      ) {
        void eventClient
          .trigger<SessionTreeDeletionSnapshot | null>(
            'harness::delete-session-tree-status',
            { operation_id: value.operation_id },
          )
          .then((fresh) => {
            if (
              !fresh ||
              !isSnapshot(fresh) ||
              fresh.session_id !== sessionId ||
              fresh.operation_id !== operationId ||
              fresh.attempt < (attempt ?? 1)
            )
              throw new Error('Unable to refresh deletion status.')
            attempt = fresh.attempt
            refreshed.add(fresh)
            accept(fresh)
          })
          .catch(() => {
            if (value.operation_id === operationId && value.attempt === attempt)
              finish(
                undefined,
                new SessionTreeDeletionError({
                  ...value,
                  force_eligible: false,
                }),
              )
          })
        return
      }
      if (value.status === 'completed') finish(value)
      else if (value.status === 'failed') {
        finish(undefined, new SessionTreeDeletionError(value))
      }
    }

    signal?.addEventListener('abort', abort, { once: true })
    if (signal?.aborted) {
      abort()
      return
    }
    const start = async () => {
      const client = await getIiiClient()
      eventClient = client
      if (settled) return
      offConnection = client.addConnectionStateListener((connection) => {
        if (connection !== 'connected' || !operationId || settled) return
        void client
          .trigger<SessionTreeDeletionSnapshot | null>(
            'harness::delete-session-tree-status',
            { operation_id: operationId },
          )
          .then((current) => {
            if (
              isSnapshot(current) &&
              current.session_id === sessionId &&
              current.operation_id === operationId &&
              current.attempt >= (attempt ?? 1)
            ) {
              attempt = current.attempt
              refreshed.add(current)
            }
            accept(current)
          })
          .catch((error: unknown) => finish(undefined, error))
      })
      const handlerId = `iii::console::session_tree_deletion_${++subscriptionSequence}`
      offHandler = client.on(handlerId, accept)
      offTrigger = client.registerTrigger({
        type: 'harness::session-tree-deletion',
        function_id: `${handlerId}::${client.browserId}`,
        config: { session_id: sessionId },
      })
      let accepted: SessionTreeDeletionSnapshot | null = null
      if (reviewOperationId) {
        accepted = await client.trigger<SessionTreeDeletionSnapshot | null>(
          'harness::delete-session-tree-status',
          { operation_id: reviewOperationId },
        )
        if (!accepted)
          throw new Error(
            'Existing deletion operation is unavailable. No deletion was started.',
          )
      } else if (recover) {
        const digest = await crypto.subtle.digest(
          'SHA-256',
          new TextEncoder().encode(sessionId),
        )
        const id =
          'delete_' +
          Array.from(new Uint8Array(digest), (byte) =>
            byte.toString(16).padStart(2, '0'),
          ).join('')
        accepted = await client.trigger<SessionTreeDeletionSnapshot | null>(
          'harness::delete-session-tree-status',
          { operation_id: id },
        )
      }
      if (!accepted)
        accepted = await client.trigger<SessionTreeDeletionSnapshot>(
          'harness::delete-session-tree',
          force
            ? { session_id: sessionId, mode: 'force', ...force }
            : { session_id: sessionId },
        )
      if (settled) return
      if (
        !isSnapshot(accepted) ||
        accepted.session_id !== sessionId ||
        (reviewOperationId && accepted.operation_id !== reviewOperationId)
      ) {
        throw new Error(
          'Invalid deletion response. Completion could not be confirmed.',
        )
      }
      operationId = accepted.operation_id
      attempt = accepted.attempt
      // Recovered terminal status is already fresh: no redundant eligibility
      // refresh, including buffered events, and no implicit retry.
      if ((recover || reviewOperationId) && accepted.status !== 'deleting') {
        refreshed.add(accepted)
        accept(accepted)
        return
      }
      // Always start the single recovery read after acceptance. Do not wait
      // for it before accepting a terminal event (the read may be slow).
      const recovery = client.trigger<SessionTreeDeletionSnapshot | null>(
        'harness::delete-session-tree-status',
        { operation_id: operationId },
      )
      refreshed.add(accepted)
      accept(accepted)
      accept(earlyTerminals.get(eventKey(accepted)))
      earlyTerminals.clear()
      try {
        const current = await recovery
        if (
          isSnapshot(current) &&
          current.session_id === sessionId &&
          current.operation_id === operationId &&
          current.attempt >= (attempt ?? 1)
        ) {
          attempt = current.attempt
          refreshed.add(current)
        }
        accept(current)
      } catch {
        // The event subscription remains authoritative until the wait deadline.
      }
    }
    void start().catch((error: unknown) => finish(undefined, error))
  })
}
