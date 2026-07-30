/**
 * ResultGrid — tabular rendering for `QueryResp`-shaped payloads
 * (`{ rows, row_count, columns }`). Cells render by value type: NULL is a
 * ghost literal, booleans and numbers are monospace (numbers right-aligned),
 * JSON values collapse to a compact preview, long strings truncate with the
 * full value on hover. Two interaction modes: without `onRowClick` every cell
 * copies its value on click; with it, the click (or Enter/Space on the focused
 * row) selects the row and the inspector carries the per-field copy.
 *
 * Ported from the console's chat `ResultGrid` for the injected page: the
 * source already rendered a real `<table>`, so the port keeps the markup and
 * swaps Tailwind utilities for the page's scoped classes + design tokens.
 */

import { useMemo, useRef, useState } from 'react'
import { useCopyFeedback } from './cells'
import { type ColumnMeta, type ForeignKeyRef, type TableSort, typeCategory } from './db-data'
import { ArrowDown, ArrowUp, Check, KeyRound, Link2, X } from './icons'
import type { GridKeyboard } from './useGridKeyboard'
import type { TableViewControls } from './useTableView'

const CLAMP_ROWS = 30
/** Below this a column cannot show even a truncated value. */
const MIN_COL_WIDTH = 48
const STRING_TRUNCATE = 160
const JSON_PREVIEW = 80

interface ResultGridProps {
  columns?: ColumnMeta[]
  rows: Record<string, unknown>[]
  rowCount?: number
  /** Column names that form the primary key — marked with a key glyph. */
  primaryKeys?: string[]
  /** Current sort; render an indicator on the sorted column. */
  sort?: TableSort | null
  /** When set, headers become buttons cycling asc → desc → off. */
  onSort?: (sort: TableSort | null) => void
  /** When set, rows highlight on hover and clicking selects one. */
  onRowClick?: (index: number) => void
  selectedRow?: number | null
  /** Sticky header + no clamp — the page's scroll container paginates. */
  stickyHeader?: boolean
  /** Hide the "N rows" footer (the page shows its own pagination bar). */
  hideFooter?: boolean
  /**
   * Foreign keys by column name. A cell in one of these columns becomes a
   * link to the row it points at — the affordance that turns a grid into a
   * way of moving through related data rather than one table at a time.
   */
  foreignKeys?: Record<string, ForeignKeyRef>
  onFollowFk?: (fk: ForeignKeyRef, value: unknown) => void
  /**
   * Roving-tabindex cursor. When supplied, cells stop being individual buttons
   * and the grid becomes one tab stop navigated with the arrow keys.
   */
  keyboard?: GridKeyboard
  /**
   * Column layout: widths, hiding and order. Supplied by the page, which
   * persists it through the worker rather than keeping it in this browser.
   */
  view?: TableViewControls
}

/** Column order: declared `columns` metadata first, else keys of row 0. */
function resolveColumns(columns: ColumnMeta[] | undefined, rows: Record<string, unknown>[]): ColumnMeta[] {
  if (columns && columns.length > 0) return columns
  if (rows.length === 0) return []
  return Object.keys(rows[0]).map((name) => ({ name }))
}

function jsonPreview(value: unknown): string {
  let text: string
  try {
    text = JSON.stringify(value)
  } catch {
    text = String(value)
  }
  if (text.length <= JSON_PREVIEW) return text
  return `${text.slice(0, JSON_PREVIEW)}…`
}

function CellValue({ value, category }: { value: unknown; category: ReturnType<typeof typeCategory> }) {
  if (value === null || value === undefined) {
    return <span className="db-cell-null">NULL</span>
  }
  if (typeof value === 'boolean') {
    return <span className={value ? 'db-cell-bool-true' : 'db-cell-bool-false'}>{String(value)}</span>
  }
  if (typeof value === 'number') {
    return <span className="db-cell-num">{String(value)}</span>
  }
  if (typeof value === 'object') {
    const preview = jsonPreview(value)
    return (
      <span className="db-cell-json" title={preview}>
        {preview}
      </span>
    )
  }
  const text = String(value)
  const cls = category === 'date' ? 'db-cell-date' : category === 'numeric' ? 'db-cell-num' : 'db-cell-str'
  if (text.length > STRING_TRUNCATE) {
    return (
      <span className={cls} title={text}>
        {text.slice(0, STRING_TRUNCATE)}…
      </span>
    )
  }
  return <span className={cls}>{text}</span>
}

function nextSort(current: TableSort | null | undefined, column: string) {
  if (!current || current.column !== column) {
    return { column, dir: 'asc' as const }
  }
  if (current.dir === 'asc') return { column, dir: 'desc' as const }
  return null
}

export function ResultGrid({
  columns,
  rows,
  rowCount,
  primaryKeys,
  sort,
  onSort,
  onRowClick,
  selectedRow,
  stickyHeader,
  hideFooter,
  foreignKeys,
  onFollowFk,
  keyboard,
  view,
}: ResultGridProps) {
  const [expanded, setExpanded] = useState(false)
  const [dragging, setDragging] = useState<string | null>(null)
  const [dragOver, setDragOver] = useState<string | null>(null)
  const resizeRef = useRef<{ column: string; startX: number; startWidth: number } | null>(null)
  const { copied, copy } = useCopyFeedback()
  const allCols = resolveColumns(columns, rows)
  const cols = useMemo(() => {
    if (!view) return allCols
    const names = view.visible(allCols.map((c) => c.name))
    const byName = new Map(allCols.map((c) => [c.name, c]))
    return names.map((n) => byName.get(n)).filter((c): c is ColumnMeta => c !== undefined)
  }, [allCols, view])
  const total = rowCount ?? rows.length
  const clamped = !stickyHeader && !expanded && rows.length > CLAMP_ROWS
  const visible = clamped ? rows.slice(0, CLAMP_ROWS) : rows
  const pkSet = new Set(primaryKeys ?? [])

  /**
   * Resize by pointer capture on the grip.
   *
   * Width is written on every move so the column tracks the cursor; the store
   * behind `setWidth` debounces the round trip, so this stays local until the
   * drag settles.
   */
  function startResize(e: React.PointerEvent, column: string) {
    if (!view) return
    e.preventDefault()
    e.stopPropagation()
    const th = (e.currentTarget as HTMLElement).closest('th')
    const startWidth = th?.getBoundingClientRect().width ?? 120
    resizeRef.current = { column, startX: e.clientX, startWidth }
    const target = e.currentTarget as HTMLElement
    target.setPointerCapture(e.pointerId)

    const onMove = (move: PointerEvent) => {
      const r = resizeRef.current
      if (!r) return
      view.setWidth(r.column, Math.max(MIN_COL_WIDTH, r.startWidth + (move.clientX - r.startX)))
    }
    const onUp = () => {
      resizeRef.current = null
      try {
        target.releasePointerCapture(e.pointerId)
      } catch {
        // Capture already lost — that is one of the paths that lands here.
      }
      target.removeEventListener('pointermove', onMove)
      target.removeEventListener('pointerup', onUp)
      target.removeEventListener('pointercancel', onUp)
      target.removeEventListener('lostpointercapture', onUp)
    }
    target.addEventListener('pointermove', onMove)
    target.addEventListener('pointerup', onUp)
    // A cancelled gesture never fires `pointerup`. Without these the move
    // listener stays attached and a later hover over the same grip keeps
    // resizing the column with no button held.
    target.addEventListener('pointercancel', onUp)
    target.addEventListener('lostpointercapture', onUp)
  }

  if (total === 0 || cols.length === 0) {
    return <div className="db-grid-empty">· 0 rows</div>
  }

  return (
    <div>
      <div className="db-grid-wrap">
        <table className="db-grid">
          <thead className={stickyHeader ? 'sticky' : undefined}>
            <tr>
              {cols.map((col) => {
                const category = typeCategory(col.type)
                const isNum = category === 'numeric'
                const sorted = sort?.column === col.name ? sort.dir : null
                const label = (
                  <>
                    {pkSet.has(col.name) ? (
                      <KeyRound size={10} aria-label="primary key" style={{ color: 'var(--color-accent)' }} />
                    ) : null}
                    <span className="colname">{col.name}</span>
                    {col.type ? <span className="coltype">{col.type}</span> : null}
                    {sorted === 'asc' ? (
                      <ArrowUp size={10} style={{ color: 'var(--color-accent)' }} />
                    ) : sorted === 'desc' ? (
                      <ArrowDown size={10} style={{ color: 'var(--color-accent)' }} />
                    ) : null}
                  </>
                )
                const width = view?.view.widths[col.name]
                return (
                  <th
                    key={col.name}
                    className={
                      [isNum ? 'num' : '', dragOver === col.name ? 'dropping' : ''].filter(Boolean).join(' ') ||
                      undefined
                    }
                    style={width ? { width, minWidth: width, maxWidth: width } : undefined}
                    aria-sort={
                      onSort ? (sorted === 'asc' ? 'ascending' : sorted === 'desc' ? 'descending' : 'none') : undefined
                    }
                    // Reorder by dragging the header. HTML drag-and-drop rather
                    // than pointer maths: it is the one interaction the platform
                    // already gets right, including the keyboard-less edge cases.
                    draggable={view ? true : undefined}
                    onDragStart={view ? () => setDragging(col.name) : undefined}
                    onDragOver={
                      view
                        ? (e) => {
                            if (!dragging || dragging === col.name) return
                            e.preventDefault()
                            setDragOver(col.name)
                          }
                        : undefined
                    }
                    onDragLeave={view ? () => setDragOver((d) => (d === col.name ? null : d)) : undefined}
                    onDrop={
                      view
                        ? (e) => {
                            e.preventDefault()
                            if (dragging && dragging !== col.name) {
                              view.moveColumn(
                                dragging,
                                col.name,
                                allCols.map((c) => c.name),
                              )
                            }
                            setDragging(null)
                            setDragOver(null)
                          }
                        : undefined
                    }
                    onDragEnd={
                      view
                        ? () => {
                            setDragging(null)
                            setDragOver(null)
                          }
                        : undefined
                    }
                  >
                    {onSort ? (
                      <button
                        type="button"
                        className="sorter"
                        onClick={() => onSort(nextSort(sort, col.name))}
                        title={`sort by ${col.name}`}
                      >
                        {label}
                      </button>
                    ) : (
                      <span className="static">{label}</span>
                    )}
                    {view ? (
                      <>
                        <button
                          type="button"
                          className="db-col-hide"
                          title={`hide ${col.name}`}
                          aria-label={`hide ${col.name}`}
                          onClick={() => view.hide(col.name)}
                        >
                          <X size={10} aria-hidden />
                        </button>
                        {/* biome-ignore lint/a11y/noStaticElementInteractions: pointer-only resize grip; the same width is settable through saveTableView */}
                        <span
                          className="db-col-grip"
                          title="drag to resize · double-click to reset"
                          onPointerDown={(e) => startResize(e, col.name)}
                          onDoubleClick={() => view.clearWidth(col.name)}
                        />
                      </>
                    ) : null}
                  </th>
                )
              })}
            </tr>
          </thead>
          <tbody>
            {visible.map((row, i) => {
              const rowClass = [onRowClick ? 'clickable' : '', selectedRow === i ? 'selected' : '']
                .filter(Boolean)
                .join(' ')
              return (
                <tr
                  key={i}
                  onClick={onRowClick ? () => onRowClick(i) : undefined}
                  // Selectable rows are their own control: reachable by tab,
                  // activated by Enter/Space like the click path.
                  onKeyDown={
                    onRowClick
                      ? (e) => {
                          if (e.key !== 'Enter' && e.key !== ' ') return
                          e.preventDefault()
                          onRowClick(i)
                        }
                      : undefined
                  }
                  tabIndex={onRowClick ? 0 : undefined}
                  aria-selected={onRowClick ? selectedRow === i : undefined}
                  className={rowClass || undefined}
                >
                  {cols.map((col, c) => {
                    const category = typeCategory(col.type)
                    const isNum = category === 'numeric'
                    const key = `${i}:${col.name}`
                    const value = row[col.name]
                    const content =
                      copied === key ? (
                        <span className="db-copied">
                          <Check size={12} /> copied
                        </span>
                      ) : (
                        <CellValue value={value} category={category} />
                      )
                    const fk = foreignKeys?.[col.name]
                    const focused = keyboard?.isFocused(i, c) ?? false
                    const cls = [isNum ? 'num' : '', focused ? 'cursor' : ''].filter(Boolean).join(' ')
                    return (
                      // biome-ignore lint/a11y/useKeyWithClickEvents: gridcell in the roving-tabindex pattern; keys are handled once on the grid container, never per cell
                      <td
                        key={col.name}
                        className={cls || undefined}
                        role={keyboard ? 'gridcell' : undefined}
                        // Roving tabindex: exactly one cell is tabbable, so Tab
                        // leaves the grid instead of walking every cell in it.
                        data-cell={keyboard ? `${i}-${c}` : undefined}
                        tabIndex={keyboard ? keyboard.tabIndexFor(i, c) : undefined}
                        onFocus={keyboard ? () => keyboard.setCursor({ row: i, col: c }) : undefined}
                        onClick={keyboard ? () => keyboard.setCursor({ row: i, col: c }) : undefined}
                      >
                        {fk && onFollowFk && value != null ? (
                          <button
                            type="button"
                            className="db-fk-link"
                            title={`show ${fk.table} where ${fk.column} = ${String(value)}`}
                            onClick={(e) => {
                              // The row is also clickable; following the key is
                              // the more specific intent.
                              e.stopPropagation()
                              onFollowFk(fk, value)
                            }}
                          >
                            <CellValue value={value} category={category} />
                            <Link2 size={10} className="db-fk-glyph" aria-hidden />
                          </button>
                        ) : onRowClick || keyboard ? (
                          content
                        ) : (
                          <button
                            type="button"
                            className="copycell"
                            onClick={() => copy(key, value)}
                            title="click to copy"
                          >
                            {content}
                          </button>
                        )}
                      </td>
                    )
                  })}
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
      {hideFooter ? null : (
        <div className="db-grid-foot">
          <span>
            {total} row{total === 1 ? '' : 's'}
          </span>
          {clamped ? (
            <button type="button" className="db-linkish" onClick={() => setExpanded(true)}>
              show all · {rows.length} rows
            </button>
          ) : null}
          {expanded && rows.length > CLAMP_ROWS ? (
            <button type="button" className="db-linkish" onClick={() => setExpanded(false)}>
              collapse
            </button>
          ) : null}
        </div>
      )}
    </div>
  )
}
