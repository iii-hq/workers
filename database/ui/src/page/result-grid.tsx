/**
 * ResultGrid — tabular rendering for `QueryResp`-shaped payloads
 * (`{ rows, row_count, columns }`). Cells render by value type: NULL is a
 * ghost literal, booleans and numbers are monospace (numbers right-aligned),
 * JSON values collapse to a compact preview, long strings truncate with the
 * full value on hover. Every cell copies its value on click.
 *
 * Ported from the console's chat `ResultGrid` for the injected page: the
 * source already rendered a real `<table>`, so the port keeps the markup and
 * swaps Tailwind utilities for the page's scoped classes + design tokens.
 */

import { useState } from 'react'
import { cellText, copyText } from './cells'
import { type ColumnMeta, type TableSort, typeCategory } from './db-data'
import { ArrowDown, ArrowUp, Check, KeyRound } from './icons'

const CLAMP_ROWS = 30
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
}: ResultGridProps) {
  const [expanded, setExpanded] = useState(false)
  const [copied, setCopied] = useState<string | null>(null)
  const cols = resolveColumns(columns, rows)
  const total = rowCount ?? rows.length
  const clamped = !stickyHeader && !expanded && rows.length > CLAMP_ROWS
  const visible = clamped ? rows.slice(0, CLAMP_ROWS) : rows
  const pkSet = new Set(primaryKeys ?? [])

  if (total === 0 || cols.length === 0) {
    return <div className="db-grid-empty">· 0 rows</div>
  }

  const copyCell = (key: string, value: unknown) => {
    copyText(cellText(value))
    setCopied(key)
    window.setTimeout(() => {
      setCopied((cur) => (cur === key ? null : cur))
    }, 1200)
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
                      <KeyRound
                        size={10}
                        aria-label="primary key"
                        style={{ color: 'var(--color-accent)' }}
                      />
                    ) : null}
                    <span className="colname">{col.name}</span>
                    {col.type ? (
                      <span className="coltype">{col.type}</span>
                    ) : null}
                    {sorted === 'asc' ? (
                      <ArrowUp
                        size={10}
                        style={{ color: 'var(--color-accent)' }}
                      />
                    ) : sorted === 'desc' ? (
                      <ArrowDown
                        size={10}
                        style={{ color: 'var(--color-accent)' }}
                      />
                    ) : null}
                  </>
                )
                return (
                  <th
                    key={col.name}
                    className={isNum ? 'num' : undefined}
                    aria-sort={
                      onSort
                        ? sorted === 'asc'
                          ? 'ascending'
                          : sorted === 'desc'
                            ? 'descending'
                            : 'none'
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
                  </th>
                )
              })}
            </tr>
          </thead>
          <tbody>
            {visible.map((row, i) => {
              const rowClass = [
                onRowClick ? 'clickable' : '',
                selectedRow === i ? 'selected' : '',
              ]
                .filter(Boolean)
                .join(' ')
              return (
                <tr
                  key={i}
                  onClick={onRowClick ? () => onRowClick(i) : undefined}
                  className={rowClass || undefined}
                >
                  {cols.map((col) => {
                    const category = typeCategory(col.type)
                    const isNum = category === 'numeric'
                    const key = `${i}:${col.name}`
                    const content =
                      copied === key ? (
                        <span className="db-copied">
                          <Check size={12} /> copied
                        </span>
                      ) : (
                        <CellValue value={row[col.name]} category={category} />
                      )
                    return (
                      <td key={col.name} className={isNum ? 'num' : undefined}>
                        {onRowClick ? (
                          content
                        ) : (
                          <button
                            type="button"
                            className="copycell"
                            onClick={() => copyCell(key, row[col.name])}
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
            <button
              type="button"
              className="db-linkish"
              onClick={() => setExpanded(true)}
            >
              show all · {rows.length} rows
            </button>
          ) : null}
          {expanded && rows.length > CLAMP_ROWS ? (
            <button
              type="button"
              className="db-linkish"
              onClick={() => setExpanded(false)}
            >
              collapse
            </button>
          ) : null}
        </div>
      )}
    </div>
  )
}
