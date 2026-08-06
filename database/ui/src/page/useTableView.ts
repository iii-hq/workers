/**
 * Per-table layout: column widths, hidden columns, column order.
 *
 * Held on the `state` worker rather than in browser storage, so a layout
 * survives a restart and is not private to one machine. Writes are debounced
 * because a resize drag emits a width on every mouse move, and each of those
 * would otherwise be a round trip.
 *
 * Reads are optimistic: local state changes immediately and the save follows.
 * A failed save is not surfaced — losing a column width is not worth an error
 * panel over the data you were trying to read.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { getTableView, saveTableView, type TableView } from '../lib/rpc'

const SAVE_DEBOUNCE_MS = 600

const EMPTY: TableView = { widths: {}, hidden: [], order: [] }

export interface TableViewControls {
  view: TableView
  /** Columns in display order, with hidden ones removed. */
  visible: (columns: string[]) => string[]
  setWidth: (column: string, width: number) => void
  clearWidth: (column: string) => void
  hide: (column: string) => void
  showAll: () => void
  moveColumn: (column: string, before: string | null, columns: string[]) => void
  reset: () => void
}

export function useTableView(host: Host, db: string, table: string): TableViewControls {
  const [view, setView] = useState<TableView>(EMPTY)
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null)
  const latest = useRef<TableView>(EMPTY)
  // A queued write, captured with its target so a table switch cannot retarget it.
  const inflight = useRef<{ db: string; table: string; view: TableView } | null>(null)
  // Late-bound so the load effect above can flush without depending on it.
  const flushRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    let alive = true
    // Send anything still queued for the previous table before the ref is
    // reset, or the last change made there is silently dropped.
    flushRef.current?.()
    setView(EMPTY)
    latest.current = EMPTY
    getTableView(host, db, table)
      .then((v) => {
        if (!alive) return
        latest.current = v
        setView(v)
      })
      .catch(() => {
        // An older worker has no such function. The grid is simply not
        // customisable then, which is how it behaved before.
      })
    return () => {
      alive = false
    }
  }, [host, db, table])

  // Flushing on unmount too: closing the page mid-debounce should not lose
  // the resize you just made.
  useEffect(
    () => () => {
      flushRef.current?.()
    },
    [],
  )

  const commit = useCallback(
    (next: TableView) => {
      latest.current = next
      setView(next)
      if (pending.current) clearTimeout(pending.current)
      // The payload and its target are captured here, not read from the ref
      // when the timer fires. Reading later meant that selecting another table
      // inside the debounce window reset the ref to EMPTY first, and the
      // pending write then wiped the layout of the table you had just been
      // adjusting.
      const payload = next
      const forDb = db
      const forTable = table
      pending.current = setTimeout(() => {
        pending.current = null
        inflight.current = null
        void saveTableView(host, forDb, forTable, payload).catch(() => {})
      }, SAVE_DEBOUNCE_MS)
      inflight.current = { db: forDb, table: forTable, view: payload }
    },
    [host, db, table],
  )

  /** Send a queued write immediately. */
  const flush = useCallback(() => {
    if (pending.current) {
      clearTimeout(pending.current)
      pending.current = null
    }
    const queued = inflight.current
    inflight.current = null
    if (queued) {
      void saveTableView(host, queued.db, queued.table, queued.view).catch(() => {})
    }
  }, [host])
  flushRef.current = flush

  const setWidth = useCallback(
    (column: string, width: number) => {
      commit({ ...latest.current, widths: { ...latest.current.widths, [column]: width } })
    },
    [commit],
  )

  const clearWidth = useCallback(
    (column: string) => {
      const widths = { ...latest.current.widths }
      delete widths[column]
      commit({ ...latest.current, widths })
    },
    [commit],
  )

  const hide = useCallback(
    (column: string) => {
      if (latest.current.hidden.includes(column)) return
      commit({ ...latest.current, hidden: [...latest.current.hidden, column] })
    },
    [commit],
  )

  const showAll = useCallback(() => {
    commit({ ...latest.current, hidden: [] })
  }, [commit])

  /**
   * Move `column` to sit before `before` (or to the end when null).
   *
   * The stored order is materialised from the table's *current* columns, so a
   * column added since the last save lands in its natural position rather than
   * disappearing from an order list that predates it.
   */
  const moveColumn = useCallback(
    (column: string, before: string | null, columns: string[]) => {
      const current = orderColumns(columns, latest.current.order)
      const without = current.filter((c) => c !== column)
      const at = before === null ? without.length : without.indexOf(before)
      const index = at < 0 ? without.length : at
      const next = [...without.slice(0, index), column, ...without.slice(index)]
      commit({ ...latest.current, order: next })
    },
    [commit],
  )

  const reset = useCallback(() => commit(EMPTY), [commit])

  const visible = useCallback(
    (columns: string[]) => {
      const hidden = new Set(view.hidden)
      return orderColumns(columns, view.order).filter((c) => !hidden.has(c))
    },
    [view],
  )

  return { view, visible, setWidth, clearWidth, hide, showAll, moveColumn, reset }
}

/**
 * Apply a stored order to the table's actual columns.
 *
 * Stored names the table no longer has are dropped; columns the order does not
 * mention keep their natural position at the end. Both matter after a schema
 * change: a rename should cost you one column's position, not the whole
 * layout.
 */
export function orderColumns(columns: string[], order: string[]): string[] {
  if (order.length === 0) return columns
  const present = new Set(columns)
  const ordered = order.filter((c) => present.has(c))
  const seen = new Set(ordered)
  return [...ordered, ...columns.filter((c) => !seen.has(c))]
}
