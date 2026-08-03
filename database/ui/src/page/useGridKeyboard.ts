/**
 * Keyboard navigation for the grid, in the ARIA roving-tabindex pattern.
 *
 * Exactly one cell carries `tabIndex=0`; every other cell is `-1`. Tab moves
 * *past* the grid, arrows move *within* it. That is the whole reason this
 * exists: the previous behaviour made every cell a `<button>`, which on a
 * 50 × 20 page is a thousand tab stops between the filter bar and the pager.
 *
 * All movement logic lives in `lib/grid-cursor.ts` and is tested there. This
 * hook binds keys and handles the two things that need the DOM: scrolling the
 * focused cell into view, and clearing the cursor when the grid loses focus.
 */

import { type RefObject, useCallback, useEffect, useRef, useState } from 'react'
import { type Cursor, type CursorAction, moveCursor, reanchor, rowKey } from '../lib/grid-cursor'

export interface GridKeyboard {
  cursor: Cursor | null
  setCursor: (next: Cursor | null) => void
  onKeyDown: (e: React.KeyboardEvent) => void
  /** `0` for the focused cell, `-1` for the rest. */
  tabIndexFor: (row: number, col: number) => number
  isFocused: (row: number, col: number) => boolean
}

export function useGridKeyboard({
  rows,
  columns,
  primaryKeys,
  containerRef,
  hasPrevPage,
  hasNextPage,
  onTurnPage,
  onCopyCell,
  onCopyRow,
  onActivate,
}: {
  rows: Record<string, unknown>[]
  columns: string[]
  primaryKeys: string[]
  containerRef: RefObject<HTMLElement | null>
  hasPrevPage: boolean
  hasNextPage: boolean
  /** Turn the page and land on the given edge once the new rows arrive. */
  onTurnPage?: (delta: -1 | 1, landing: 'first' | 'last') => void
  onCopyCell?: (row: number, col: number) => void
  onCopyRow?: (row: number) => void
  onActivate?: (row: number) => void
}): GridKeyboard {
  const [cursor, setCursor] = useState<Cursor | null>(null)
  // Only scroll when the cursor moved by key. Without this, every mouse click
  // yanks the viewport to wherever the click landed.
  const byKeyboard = useRef(false)
  const anchor = useRef<{ key: string | null; column: string | null }>({ key: null, column: null })
  // Read by `apply`, which must not consult state through an updater.
  const cursorRef = useRef<Cursor | null>(null)
  // Which edge to land on once the page turn delivers new rows.
  const pendingLanding = useRef<'first' | 'last' | null>(null)

  useEffect(() => {
    cursorRef.current = cursor
  }, [cursor])

  // Remember what the cursor pointed at, so a sort or filter can re-find it.
  useEffect(() => {
    if (!cursor) {
      anchor.current = { key: null, column: null }
      return
    }
    const row = rows[cursor.row]
    anchor.current = {
      key: row ? rowKey(row, primaryKeys) : null,
      column: columns[cursor.col] ?? null,
    }
  }, [cursor, rows, primaryKeys, columns])

  // Rows changed underneath the cursor: follow the row it was on rather than
  // staying at the same index, which after a sort is a different row.
  const signature = `${rows.length}:${columns.join(',')}`
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed on the shape of the data, not its identity
  useEffect(() => {
    // A page turn is in flight: place the cursor on the edge the move asked
    // for, keeping its column. Re-anchoring instead would look for a row key
    // that is on the previous page and clamp to the wrong end.
    const landing = pendingLanding.current
    if (landing) {
      pendingLanding.current = null
      byKeyboard.current = true
      setCursor((cur) => ({
        row: landing === 'first' ? 0 : Math.max(0, rows.length - 1),
        col: Math.min(cur?.col ?? 0, Math.max(0, columns.length - 1)),
      }))
      return
    }
    setCursor((cur) => {
      if (!cur) return cur
      const keys = rows.map((r) => rowKey(r, primaryKeys))
      return reanchor(cur, anchor.current.key, keys, columns, anchor.current.column)
    })
  }, [signature])

  useEffect(() => {
    if (!cursor || !byKeyboard.current) return
    byKeyboard.current = false
    const el = containerRef.current?.querySelector<HTMLElement>(`[data-cell="${cursor.row}-${cursor.col}"]`)
    // `nearest` plus a `scroll-margin-top` equal to the sticky header height
    // keeps the focused row from parking underneath it.
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    el?.focus({ preventScroll: true })
  }, [cursor, containerRef])

  const apply = useCallback(
    (action: CursorAction) => {
      byKeyboard.current = true
      // Computed outside the updater. React may invoke an updater more than
      // once — StrictMode does it on every render in development — and calling
      // `onTurnPage` from inside would advance the page twice per keypress.
      const from = cursorRef.current ?? { row: 0, col: 0 }
      const move = moveCursor(from, action, {
        rowCount: rows.length,
        colCount: columns.length,
        pageRows: Math.max(1, rows.length - 1),
        hasPrevPage,
        hasNextPage,
      })
      if (move.pageDelta && onTurnPage) {
        // The new page has not arrived yet, so remember which edge to land on
        // and let the effect below place the cursor once the rows change.
        pendingLanding.current = move.landing ?? 'first'
        onTurnPage(move.pageDelta, move.landing ?? 'first')
        return
      }
      setCursor(move.cursor)
    },
    [rows.length, columns.length, hasPrevPage, hasNextPage, onTurnPage],
  )

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Never swallow typing in a filter input that happens to be inside.
      const target = e.target as HTMLElement | null
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return

      const mod = e.metaKey || e.ctrlKey
      const at = cursor

      switch (e.key) {
        case 'ArrowUp':
          apply({ type: 'up' })
          break
        case 'ArrowDown':
          apply({ type: 'down' })
          break
        case 'ArrowLeft':
          apply({ type: 'left' })
          break
        case 'ArrowRight':
          apply({ type: 'right' })
          break
        case 'Home':
          apply(mod ? { type: 'gridStart' } : { type: 'rowStart' })
          break
        case 'End':
          apply(mod ? { type: 'gridEnd' } : { type: 'rowEnd' })
          break
        case 'PageUp':
          apply({ type: 'pageUp' })
          break
        case 'PageDown':
          apply({ type: 'pageDown' })
          break
        case 'Enter':
          if (at && onActivate) onActivate(at.row)
          else return
          break
        case 'c':
        case 'C':
          if (!mod || !at) return
          // Shift copies the row as TSV so it pastes into a spreadsheet as
          // cells rather than one blob.
          if (e.shiftKey) onCopyRow?.(at.row)
          else onCopyCell?.(at.row, at.col)
          // Not prevented: let the platform copy run if nothing handled it.
          return
        default:
          return
      }
      e.preventDefault()
    },
    [apply, cursor, onActivate, onCopyCell, onCopyRow],
  )

  return {
    cursor,
    setCursor: useCallback((next: Cursor | null) => {
      byKeyboard.current = false
      setCursor(next)
    }, []),
    onKeyDown,
    tabIndexFor: (row, col) => {
      // With no cursor yet, the first cell is the way in.
      if (!cursor) return row === 0 && col === 0 ? 0 : -1
      return cursor.row === row && cursor.col === col ? 0 : -1
    },
    isFocused: (row, col) => cursor?.row === row && cursor?.col === col,
  }
}
