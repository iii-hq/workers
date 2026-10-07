import type { ExtensionIii } from '@/types/injectable-ui'
import type { Accepted, OperationSnapshot } from './compose-api'

/** One `compose-operation` delivery (iii-compose `ProgressEvent`). */
export type ProgressEvent = {
  sequence: number
  operation_id: string
  container?: string | null
  phase: string
  detail: string
  current?: number | null
  total?: number | null
  elapsed_ms?: number
  terminal: boolean
}

export type OperationBus = Pick<
  ExtensionIii,
  'on' | 'registerTrigger' | 'browserId'
>

export type FollowOptions = {
  iii: OperationBus
  /** Submit the mutation under the id chosen here (`operation_id`). */
  start: (operationId: string) => Promise<Accepted>
  /** `compose::operation`: read once after the accept, and once at the end. */
  read: (operationId: string) => Promise<OperationSnapshot>
  /** The operation while it runs: the accept, then each progress event. */
  onProgress?: (snapshot: OperationSnapshot, operationId: string) => void
  /** Aborting unbinds and settles with `null`: the page went away. */
  signal?: AbortSignal
  /** Tests pin the id; the page lets this module choose one. */
  operationId?: string
}

/** A caller-selected compose operation id, chosen before submission. */
export function newOperationId(): string {
  return `compose:${crypto.randomUUID()}`
}

let handlerSeq = 0

/**
 * Follow a compose mutation by its pushed progress, never by polling: the
 * `compose-operation` binding goes in BEFORE the mutation is submitted, under
 * an id chosen here. `compose::operation` is read exactly twice at most — once
 * right after the accept (an operation that finished before the binding
 * landed is settled there), and once when the terminal event arrives, for the
 * final status the event does not carry. Resolves with the final snapshot, or
 * `null` when aborted.
 */
export function followOperation({
  iii,
  start,
  read,
  onProgress,
  signal,
  operationId = newOperationId(),
}: FollowOptions): Promise<OperationSnapshot | null> {
  const handlerId = `iii::console::workers::operation::${++handlerSeq}`
  let id = operationId
  let current: OperationSnapshot = {
    operation_id: id,
    status: 'running',
    phase: 'accepted',
    completed: 0,
    total: 0,
    last_event: null,
  }
  let lastSequence = 0
  let settled = false
  let ending = false
  let offTrigger: (() => void) | null = null
  let offHandler: (() => void) | null = null

  return new Promise<OperationSnapshot | null>((resolve, reject) => {
    const unbind = () => {
      try {
        offTrigger?.()
      } catch {
        // already gone
      }
      try {
        offHandler?.()
      } catch {
        // already gone
      }
      offTrigger = null
      offHandler = null
    }
    const settle = (outcome: OperationSnapshot | null, error?: unknown) => {
      if (settled) return
      settled = true
      unbind()
      signal?.removeEventListener('abort', abort)
      if (error !== undefined) reject(error)
      else resolve(outcome)
    }
    const abort = () => settle(null)
    const bind = (operation: string) => {
      try {
        offTrigger?.()
      } catch {
        // already gone
      }
      offTrigger = iii.registerTrigger({
        type: 'compose-operation',
        function_id: `${handlerId}::${iii.browserId}`,
        config: { operation_id: operation },
      })
    }
    /** The final status: the terminal event says only that it ended. */
    const finish = () => {
      if (ending || settled) return
      ending = true
      read(id).then(
        (snapshot) =>
          settle(
            snapshot.status === 'running'
              ? { ...current, status: 'failed' }
              : snapshot,
          ),
        (error: unknown) => settle(null, error),
      )
    }

    if (signal?.aborted) {
      settle(null)
      return
    }
    signal?.addEventListener('abort', abort)

    offHandler = iii.on<ProgressEvent>(handlerId, (event) => {
      if (settled || event?.operation_id !== id) return
      if (typeof event.sequence === 'number') {
        if (event.sequence <= lastSequence) return
        lastSequence = event.sequence
      }
      current = {
        ...current,
        phase: event.phase,
        // Per-container events carry their own counters (a tree depth);
        // only the terminal one counts the operation.
        ...(event.terminal
          ? {
              completed: event.current ?? current.completed,
              total: event.total ?? current.total,
            }
          : {}),
        last_event: {
          detail: event.detail,
          container: event.container ?? null,
          phase: event.phase,
        },
      }
      if (event.terminal) finish()
      else onProgress?.(current, id)
    })
    bind(id)

    start(id).then(
      (accepted) => {
        if (settled) return
        // A daemon that ignores the caller's id runs the operation under its
        // own: follow that one instead.
        if (accepted.operation_id && accepted.operation_id !== id) {
          id = accepted.operation_id
          current = { ...current, operation_id: id }
          bind(id)
        }
        onProgress?.(current, id)
        // The catch-up read: an operation that ended before the binding was
        // in place sends nothing more.
        read(id).then(
          (snapshot) => {
            if (settled || ending) return
            if (snapshot.status !== 'running') {
              ending = true
              settle(snapshot)
              return
            }
            // Newer than every event seen: it stands. Older: only its
            // operation-wide counters are news.
            const sequence = snapshot.last_sequence ?? 0
            if (sequence > lastSequence) {
              lastSequence = sequence
              current = snapshot
            } else {
              current = {
                ...current,
                completed: snapshot.completed,
                total: snapshot.total,
              }
            }
            onProgress?.(current, id)
          },
          // Events still arrive; only the catch-up is lost.
          () => {},
        )
      },
      (error: unknown) => settle(null, error),
    )
  })
}
