import { getIiiClient } from '@/lib/iii-client'

export interface SessionTreeDeletionSnapshot {
  operation_id: string
  /** Increments only when a failed operation is retried. */
  attempt: number
  session_id: string
  status: 'deleting' | 'completed' | 'failed'
  deleted_session_ids: string[]
  error?: string
}

export interface DeleteSessionTreeOptions {
  /** Stops this browser's wait, never the backend operation. */
  signal?: AbortSignal
  timeoutMs?: number
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
    (snapshot.error === undefined || typeof snapshot.error === 'string')
  )
}

/**
 * One high-level command, one catch-up read, then terminal events only.
 * The backend owns tree traversal, cancellation and idempotency by session id.
 * Never fall back to session::delete, even on an older/unavailable harness.
 */
export function deleteSessionTree(
  sessionId: string,
  { signal, timeoutMs = DELETE_TREE_WAIT_MS }: DeleteSessionTreeOptions = {},
): Promise<SessionTreeDeletionSnapshot> {
  return new Promise((resolve, reject) => {
    let settled = false
    let operationId: string | undefined
    let attempt: number | undefined
    const eventKey = (value: SessionTreeDeletionSnapshot) =>
      JSON.stringify([value.operation_id, value.attempt])
    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined
    // A terminal event can beat the command's acceptance response. It is
    // usable only after that response identifies this attempt's operation.
    const earlyTerminals = new Map<string, SessionTreeDeletionSnapshot>()
    const cleanup = () => {
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      earlyTerminals.clear()
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
      if (value.status === 'completed') finish(value)
      else if (value.status === 'failed') {
        finish(
          undefined,
          new Error(
            value.error || 'Unable to stop and delete this conversation tree.',
          ),
        )
      }
    }

    signal?.addEventListener('abort', abort, { once: true })
    if (signal?.aborted) {
      abort()
      return
    }
    const start = async () => {
      const client = await getIiiClient()
      if (settled) return
      const handlerId = `iii::console::session_tree_deletion_${++subscriptionSequence}`
      offHandler = client.on(handlerId, accept)
      offTrigger = client.registerTrigger({
        type: 'harness::session-tree-deletion',
        function_id: `${handlerId}::${client.browserId}`,
        config: { session_id: sessionId },
      })
      const accepted = await client.trigger<SessionTreeDeletionSnapshot>(
        'harness::delete-session-tree',
        { session_id: sessionId },
      )
      if (settled) return
      if (!isSnapshot(accepted) || accepted.session_id !== sessionId) {
        throw new Error(
          'Invalid deletion response. Completion could not be confirmed.',
        )
      }
      operationId = accepted.operation_id
      attempt = accepted.attempt
      // Always start the single recovery read after acceptance. Do not wait
      // for it before accepting a terminal event (the read may be slow).
      const recovery = client.trigger<SessionTreeDeletionSnapshot | null>(
        'harness::delete-session-tree-status',
        { operation_id: operationId },
      )
      accept(earlyTerminals.get(eventKey(accepted)))
      earlyTerminals.clear()
      accept(accepted)
      try {
        accept(await recovery)
      } catch (error) {
        finish(undefined, error)
      }
    }
    void start().catch((error: unknown) => finish(undefined, error))
  })
}
