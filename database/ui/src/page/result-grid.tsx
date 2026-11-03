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

import {
  Button,
  ShadcnDropdownMenu as DropdownMenu,
  ShadcnDropdownMenuContent as DropdownMenuContent,
  ShadcnDropdownMenuGroup as DropdownMenuGroup,
  ShadcnDropdownMenuItem as DropdownMenuItem,
  ShadcnDropdownMenuLabel as DropdownMenuLabel,
  ShadcnDropdownMenuSeparator as DropdownMenuSeparator,
  ShadcnDropdownMenuTrigger as DropdownMenuTrigger,
  StatusBar,
  ShadcnTable as Table,
  ShadcnTableBody as TableBody,
  ShadcnTableCell as TableCell,
  ShadcnTableHead as TableHead,
  ShadcnTableHeader as TableHeader,
  ShadcnTableRow as TableRow,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { ArrowDown, ArrowDownUp, ArrowUp, Check, ChevronDown, Copy, EyeOff, KeyRound, Link2 } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { cellText } from '../lib/grid-cursor'
import {
  type ColumnMeta,
  type ForeignKeyRef,
  type TableSort,
  typeCategory,
} from './db-data'
import type { GridKeyboard } from './useGridKeyboard'
import type { TableViewControls } from './useTableView'

const CLAMP_ROWS = 30
/** Below this a column cannot show even a truncated value. */
const MIN_COL_WIDTH = 48
/** Where every column starts until it is resized. */
const DEFAULT_COL_WIDTH = 200
/** The row-number gutter. */
const GUTTER_WIDTH = 56
const STRING_TRUNCATE = 160
const JSON_PREVIEW = 80
/** Longer than a column usually shows: the full text rides on hover. */
const TITLE_FROM = 40

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
  /**
   * Leading row-number gutter, numbered from `rowOffset + 1` — the page's
   * position in the whole table, not in the page.
   */
  rowNumbers?: boolean
  rowOffset?: number
}

/** Column order: declared `columns` metadata first, else keys of row 0. */
function resolveColumns(
  columns: ColumnMeta[] | undefined,
  rows: Record<string, unknown>[],
): ColumnMeta[] {
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

function CellValue({
  value,
  category,
}: {
  value: unknown
  category: ReturnType<typeof typeCategory>
}) {
  if (value === null || value === undefined) {
    return <span className="db-cell-null">NULL</span>
  }
  if (typeof value === 'boolean') {
    return (
      <span className={value ? 'db-cell-bool-true' : 'db-cell-bool-false'}>
        {String(value)}
      </span>
    )
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
  // An empty string and NULL are different answers; a blank cell said neither.
  if (text === '') {
    return (
      <span className="db-cell-empty" title="empty string">
        empty
      </span>
    )
  }
  const cls =
    category === 'date'
      ? 'db-cell-date'
      : category === 'numeric'
        ? 'db-cell-num'
        : 'db-cell-str'
  if (text.length > STRING_TRUNCATE) {
    return (
      <span className={cls} title={text}>
        {text.slice(0, STRING_TRUNCATE)}…
      </span>
    )
  }
  return (
    <span className={cls} title={text.length > TITLE_FROM ? text : undefined}>
      {text}
    </span>
  )
}

/** A cell that copies itself on click; the flash is per cell, so each owns
 *  a hook. Null copies as the literal `NULL`, the way the inspector does. */
function CopyCell({
  value,
  children,
}: {
  value: unknown
  children: React.ReactNode
}) {
  const { state, copy } = useCopyFlash(
    value == null ? 'NULL' : cellText(value),
    1200,
  )
  return (
    <button
      type="button"
      className="copycell"
      onClick={() => copy()}
      title="click to copy"
    >
      {state === 'copied' ? (
        <span className="db-copied">
          <Check size={16} /> Copied
        </span>
      ) : (
        children
      )}
    </button>
  )
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
  rowNumbers,
  rowOffset = 0,
}: ResultGridProps) {
  const [expanded, setExpanded] = useState(false)
  const [dragging, setDragging] = useState<string | null>(null)
  const [dragOver, setDragOver] = useState<string | null>(null)
  const resizeRef = useRef<{
    column: string
    startX: number
    startWidth: number
  } | null>(null)
  const allCols = resolveColumns(columns, rows)
  const cols = useMemo(() => {
    if (!view) return allCols
    const names = view.visible(allCols.map((c) => c.name))
    const byName = new Map(allCols.map((c) => [c.name, c]))
    return names
      .map((n) => byName.get(n))
      .filter((c): c is ColumnMeta => c !== undefined)
  }, [allCols, view])
  // Numbers align right so magnitudes line up. A declared numeric type
  // decides; an untyped column (ad-hoc SQL) is numeric when its values are.
  const numericCols = useMemo(() => {
    const out = new Set<string>()
    for (const col of cols) {
      if (typeCategory(col.type) === 'numeric') {
        out.add(col.name)
        continue
      }
      if (col.type) continue
      const present = rows.map((r) => r[col.name]).filter((v) => v !== null && v !== undefined)
      if (present.length > 0 && present.every((v) => typeof v === 'number')) out.add(col.name)
    }
    return out
  }, [cols, rows])
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
      view.setWidth(
        r.column,
        Math.max(MIN_COL_WIDTH, r.startWidth + (move.clientX - r.startX)),
      )
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

  // Fixed layout honours widths only against an explicit table width; the
  // sum keeps every column at its own width (CSS `min-width: 100%` stretches
  // a narrow table to the pane).
  const tableWidth =
    (rowNumbers ? GUTTER_WIDTH : 0) +
    cols.reduce((sum, col) => sum + (view?.view.widths[col.name] ?? DEFAULT_COL_WIDTH), 0)

  return (
    <div>
      <div className="db-grid-wrap">
        {/* With the roving cursor the table is a grid for real — the role
            makes the per-cell `gridcell`s valid instead of orphaned. */}
        <Table className="db-grid" role={keyboard ? 'grid' : undefined} style={{ width: tableWidth }}>
          <TableHeader className={stickyHeader ? 'sticky' : undefined}>
            <TableRow>
              {rowNumbers ? (
                <TableHead className="db-gutter" aria-label="row number" style={{ width: GUTTER_WIDTH }}>
                  #
                </TableHead>
              ) : null}
              {cols.map((col) => {
                const isNum = numericCols.has(col.name)
                const sorted = sort?.column === col.name ? sort.dir : null
                const label = (
                  <>
                    {pkSet.has(col.name) ? (
                      <KeyRound size={16} aria-label="primary key" className="db-col-key" />
                    ) : null}
                    <span className="colname">{col.name}</span>
                    {col.type ? <span className="coltype">{col.type.toLowerCase()}</span> : null}
                    {sorted === 'asc' ? (
                      <ArrowUp size={16} className="db-col-sorted" aria-hidden />
                    ) : sorted === 'desc' ? (
                      <ArrowDown size={16} className="db-col-sorted" aria-hidden />
                    ) : null}
                  </>
                )
                const width = view?.view.widths[col.name]
                return (
                  <TableHead
                    key={col.name}
                    className={[isNum ? 'num' : '', dragOver === col.name ? 'dropping' : ''].filter(Boolean).join(' ') || undefined}
                    style={width ? { width, minWidth: width, maxWidth: width } : { width: DEFAULT_COL_WIDTH }}
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
                    <span className="db-th">
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
                      {onSort || view ? (
                        <ColumnMenu
                          column={col.name}
                          type={col.type}
                          sorted={sorted}
                          onSort={onSort ? (dir) => onSort(dir ? { column: col.name, dir } : null) : undefined}
                          onHide={view ? () => view.hide(col.name) : undefined}
                        />
                      ) : null}
                    </span>
                    {view ? (
                      // biome-ignore lint/a11y/noStaticElementInteractions: pointer-only resize grip; the same width is settable through saveTableView
                      <span
                        className="db-col-grip"
                        title="drag to resize · double-click to reset"
                        onPointerDown={(e) => startResize(e, col.name)}
                        onDoubleClick={() => view.clearWidth(col.name)}
                      />
                    ) : null}
                  </TableHead>
                )
              })}
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.map((row, i) => {
              const rowClass = [
                onRowClick ? 'clickable' : '',
                selectedRow === i ? 'selected' : '',
              ]
                .filter(Boolean)
                .join(' ')
              return (
                <TableRow
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
                  data-state={selectedRow === i ? 'selected' : undefined}
                >
                  {rowNumbers ? <TableCell className="db-gutter">{rowOffset + i + 1}</TableCell> : null}
                  {cols.map((col, c) => {
                    const category = typeCategory(col.type)
                    const isNum = numericCols.has(col.name)
                    const value = row[col.name]
                    const content = (
                      <CellValue value={value} category={category} />
                    )
                    const fk = foreignKeys?.[col.name]
                    const focused = keyboard?.isFocused(i, c) ?? false
                    const cls = [isNum ? 'num' : '', focused ? 'cursor' : '']
                      .filter(Boolean)
                      .join(' ')
                    return (
                      // biome-ignore lint/a11y/useKeyWithClickEvents: gridcell in the roving-tabindex pattern; keys are handled once on the grid container, never per cell
                      <TableCell
                        key={col.name}
                        className={cls || undefined}
                        role={keyboard ? 'gridcell' : undefined}
                        // Roving tabindex: exactly one cell is tabbable, so Tab
                        // leaves the grid instead of walking every cell in it.
                        data-cell={keyboard ? `${i}-${c}` : undefined}
                        tabIndex={
                          keyboard ? keyboard.tabIndexFor(i, c) : undefined
                        }
                        onFocus={
                          keyboard
                            ? () => keyboard.setCursor({ row: i, col: c })
                            : undefined
                        }
                        onClick={
                          keyboard
                            ? () => keyboard.setCursor({ row: i, col: c })
                            : undefined
                        }
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
                            <Link2
                              size={16}
                              className="db-fk-glyph"
                              aria-hidden
                            />
                          </button>
                        ) : onRowClick || keyboard ? (
                          content
                        ) : (
                          <CopyCell value={value}>{content}</CopyCell>
                        )}
                      </TableCell>
                    )
                  })}
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      </div>
      {hideFooter ? null : (
        <StatusBar className="db-grid-foot">
          <span>
            {total} row{total === 1 ? '' : 's'}
          </span>
          {clamped ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setExpanded(true)}
            >
              show all · {rows.length} rows
            </Button>
          ) : null}
          {expanded && rows.length > CLAMP_ROWS ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setExpanded(false)}
            >
              collapse
            </Button>
          ) : null}
        </StatusBar>
      )}
    </div>
  )
}

/**
 * The header's ⌄ menu: sort either way, clear the sort, copy the name, hide
 * the column. Clicking the label still cycles the sort; the menu is the
 * discoverable, explicit route to the same verbs.
 */
function ColumnMenu({
  column,
  type,
  sorted,
  onSort,
  onHide,
}: {
  column: string
  type?: string
  sorted: 'asc' | 'desc' | null
  onSort?: (dir: 'asc' | 'desc' | null) => void
  onHide?: () => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="db-col-menu" aria-label={`${column} column options`}>
        <ChevronDown size={16} aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {/* Base UI only allows a menu label inside a group. */}
        <DropdownMenuGroup>
          <DropdownMenuLabel>
            {column}
            {type ? ` · ${type.toLowerCase()}` : ''}
          </DropdownMenuLabel>
        </DropdownMenuGroup>
        {onSort ? (
          <>
            <DropdownMenuItem onClick={() => onSort('asc')} disabled={sorted === 'asc'}>
              <ArrowUp size={16} aria-hidden />
              Sort ascending
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => onSort('desc')} disabled={sorted === 'desc'}>
              <ArrowDown size={16} aria-hidden />
              Sort descending
            </DropdownMenuItem>
            {sorted ? (
              <DropdownMenuItem onClick={() => onSort(null)}>
                <ArrowDownUp size={16} aria-hidden />
                Clear sort
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
          </>
        ) : null}
        <DropdownMenuItem onClick={() => void copyText(column)}>
          <Copy size={16} aria-hidden />
          Copy name
        </DropdownMenuItem>
        {onHide ? (
          <DropdownMenuItem onClick={onHide}>
            <EyeOff size={16} aria-hidden />
            Hide column
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
