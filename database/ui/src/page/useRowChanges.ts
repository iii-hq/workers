/**
 * Live observer of `database::on-row-changed`.
 *
 * The worker announces a change after it commits, and what that covers depends
 * on the handle's configured capture mode:
 *
 *   statements  only writes made through this worker. A write from psql or
 *               another service is invisible.
 *   native      every client's committed writes to the watched table, via
 *               database triggers and NOTIFY / the binlog.
 *
 * A grid that claims to be live while missing a colleague's write is worse
 * than one that claims nothing, so this hook reports what it observes and
 * leaves the wording to the caller. Delivery is best effort either way — no
 * replay, no exactly-once — so it drives an indicator and a refresh prompt,
 * never a source of truth.
 *
 * **An observer must name a table.** `{ db }`-only args are rejected outright
 * on a native handle, which would leave a panel that looks live and receives
 * nothing. `table` is therefore required, and the hook stays idle until it
 * has one. The args carry it canonical (bare, unquoted, lowercased), as the
 * worker requires.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useId, useRef, useState } from 'react'

/** Mirrors the worker's event; `at` is epoch milliseconds, not an ISO string. */
export interface RowChangedEvent {
  db: string
  table: string | null
  op: 'insert' | 'update' | 'delete' | 'other'
  affected_rows: number
  /** Writer's RETURNING projection (statements capture) or the changed rows'
   *  primary keys (native capture, capped at 100). */
  returning?: Record<string, unknown>[]
  /** Native capture only: `returning` holds fewer keys than `affected_rows`. */
  truncated?: boolean
  at: number
}

export interface RowChange extends RowChangedEvent {
  /** Local receipt time. Ordering and the recency fade both use this, because
   *  `at` comes from the database clock and may skew against the browser's. */
  seen: number
  /** Monotonic arrival number — the collision-free list key (`seen` repeats
   *  for two events landing in the same millisecond). */
  seq: number
}

export interface RowChangeFeed {
  changes: RowChange[]
  /** Idle until a table is selected; observing once the observer is made. */
  status: 'idle' | 'observing' | 'unsupported'
  /** Events since the caller last acknowledged. */
  pending: number
  lastAt: number | null
  acknowledge: () => void
}

const MAX_KEPT = 60

/** Shared across feeds on purpose: any two changes anywhere differ. */
let feedSeq = 0

/** The worker's canonical table: the last dotted segment, unquoted, ASCII
 *  lowercased (`public."Orders"` → `orders`). Observer args must use it. */
export function canonicalTable(table: string): string {
  const last = table.split('.').pop() ?? table
  const trimmed = last.trim()
  const quoted = [
    ['"', '"'],
    ['`', '`'],
    ['[', ']'],
  ].find(([open, close]) => trimmed.length >= 2 && trimmed.startsWith(open) && trimmed.endsWith(close))
  const bare = quoted ? trimmed.slice(1, -1) : trimmed
  return bare.replace(/[A-Z]/g, (c) => c.toLowerCase())
}

export function useRowChanges(host: Host, db: string | undefined, table: string | null | undefined): RowChangeFeed {
  const [changes, setChanges] = useState<RowChange[]>([])
  const [status, setStatus] = useState<RowChangeFeed['status']>('idle')
  const [pending, setPending] = useState(0)
  const [lastAt, setLastAt] = useState<number | null>(null)
  // Unique per hook instance, so two mounted panels do not collide on the
  // browser-local function id.
  const instance = useId().replace(/[^a-zA-Z0-9]/g, '')
  const live = useRef(false)

  useEffect(() => {
    if (!db || !table) {
      setStatus('idle')
      return
    }
    live.current = true
    // `on` namespaces the handler per tab as `<id>::<browserId>`, so the
    // observer must name the namespaced id as its target.
    const localFn = `iii::database-ui::row-changed::${instance}`
    const boundFn = `${localFn}::${host.iii.browserId}`

    const off = host.iii.on<RowChangedEvent>(localFn, (event) => {
      if (!live.current) return
      const at = Number(event?.at)
      const change: RowChange = {
        db: event?.db ?? db,
        table: event?.table ?? table,
        op: event?.op ?? 'other',
        affected_rows: Number(event?.affected_rows ?? 0),
        returning: event?.returning,
        truncated: event?.truncated,
        at: Number.isFinite(at) ? at : Date.now(),
        seen: Date.now(),
        seq: feedSeq++,
      }
      setChanges((prev) => [change, ...prev].slice(0, MAX_KEPT))
      setPending((n) => n + 1)
      setLastAt(change.seen)
    })

    let dispose: (() => void) | undefined
    try {
      // The handler takes the event: extension observers receive the
      // function's output, which is the event.
      dispose = host.iii.observe(
        { function_id: 'database::on-row-changed', args: { db, table: canonicalTable(table) } },
        boundFn,
      )
      setStatus('observing')
    } catch {
      // A console without `observe`. Not a failure to report as an error —
      // the caller renders "not following" instead.
      setStatus('unsupported')
    }

    return () => {
      live.current = false
      setStatus('idle')
      try {
        dispose?.()
      } catch {
        /* the engine disposes the observer on disconnect anyway */
      }
      off()
    }
  }, [host, db, table, instance])

  // Changes belong to the table observed when they arrived; switching
  // tables must not carry the previous table's history across.
  useEffect(() => {
    setChanges([])
    setPending(0)
    setLastAt(null)
  }, [db, table])

  const acknowledge = useCallback(() => setPending(0), [])

  return { changes, status, pending, lastAt, acknowledge }
}
