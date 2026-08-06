/**
 * Paged read-only view of one table.
 *
 * Reads go through `database::browseTable`, which takes typed filters and
 * sorts and returns the page plus a total honouring the same filters — one
 * call where this used to make two and build its own SQL. The page no longer
 * writes `SELECT`, and neither does an agent asking the same question.
 *
 * The grid is the shared `ResultGrid`, so cells render the same way as in the
 * SQL panel and the chat family.
 */

import {
  EmptyState,
  type Host,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useMemo, useRef, useState } from 'react'
import { cellText, rowAsTsv } from '../lib/grid-cursor'
import { type FilterSpec, isComplete, PAGE_SIZE } from '../lib/rpc'
import { CellInspector } from './CellInspector'
import { ColumnStatsPanel } from './ColumnStatsPanel'
import { browseTableRef, type DbDriver, type TableSort, tableColumns } from './db-data'
import { FilterBar } from './FilterBar'
import { AlertCircle, type IconProps, Table2, X } from './icons'
import { Pagination } from './pagination'
import { RowDetail } from './RowDetail'
import { ResultGrid } from './result-grid'
import { useDatabaseRead } from './useDatabaseRead'
import { useGridKeyboard } from './useGridKeyboard'
import { useTableView } from './useTableView'

interface TableDataPanelProps {
  host: Host
  db: string
  driver: DbDriver
  table: string
  enabled: boolean
  /** Seed the SQL panel with a SELECT for this table and switch to it. */
  onOpenInSql?: (table: string) => void
  /** Follow a foreign key into another table, carrying the matching value. */
  onFollow?: (table: string, column: string, value: unknown) => void
  /**
   * A filter applied on arrival from a foreign key. It is shown as a chip like
   * any other, and is removable — arriving somewhere should not trap you
   * there.
   */
  initialFilters?: FilterSpec[]
  /**
   * Bumped by the page's refresh. Re-reads the page and columns in place —
   * a refresh must not remount the panel, because a remount wipes filters,
   * sort, and the cursor.
   */
  refreshToken?: number
}

const TableIcon = (p: IconProps) => <Table2 size={28} {...p} />

/** The modifier key, written the way this keyboard has it. */
const MOD = typeof navigator !== 'undefined' && /mac/i.test(navigator.platform || navigator.userAgent) ? '⌘' : 'ctrl+'

export function TableDataPanel({
  host,
  db,
  driver,
  table,
  enabled,
  onOpenInSql,
  onFollow,
  initialFilters,
  refreshToken,
}: TableDataPanelProps) {
  const [page, setPage] = useState(0)
  const [pageSize, setPageSize] = useState(PAGE_SIZE)
  const [sort, setSort] = useState<TableSort | null>(null)
  const [selectedRow, setSelectedRow] = useState<number | null>(null)
  const [filters, setFilters] = useState<FilterSpec[]>(initialFilters ?? [])

  // Only complete, enabled chips are sent. Serialising them into the fetcher's
  // dependency key means a half-typed value does not trigger a read, and the
  // page settles once the chip becomes valid.
  const applied = useMemo(() => filters.filter((f) => !f.disabled && isComplete(f)), [filters])
  const appliedKey = JSON.stringify(applied)

  // Editing a filter invalidates the page number and the selected row: page 4
  // of the old result set is not page 4 of the new one.
  const changeFilters = useCallback((next: FilterSpec[]) => {
    setFilters(next)
    setPage(0)
    setSelectedRow(null)
  }, [])

  // `refreshToken` re-creates the fetchers so the reads re-run in place.
  const pageFetcher = useCallback(() => {
    void refreshToken
    return browseTableRef(host, db, table, {
      page,
      pageSize,
      sort: sort ? [{ column: sort.column, direction: sort.dir }] : [],
      filters: JSON.parse(appliedKey) as FilterSpec[],
    })
  }, [host, db, table, page, pageSize, sort, appliedKey, refreshToken])
  const columnsFetcher = useCallback(() => {
    void refreshToken
    return tableColumns(host, db, driver, table)
  }, [host, db, driver, table, refreshToken])
  const pageRead = useDatabaseRead(enabled, pageFetcher)
  const columnsRead = useDatabaseRead(enabled, columnsFetcher)

  const columnInfo = useMemo(() => columnsRead.data ?? [], [columnsRead.data])
  const primaryKeys = useMemo(() => columnInfo.filter((c) => c.pk).map((c) => c.name), [columnInfo])
  const foreignKeys = useMemo(() => {
    const out: Record<string, NonNullable<(typeof columnInfo)[number]['fk']>> = {}
    for (const c of columnInfo) {
      if (c.fk) out[c.name] = c.fk
    }
    return out
  }, [columnInfo])

  const view = useTableView(host, db, table)
  const gridRef = useRef<HTMLDivElement | null>(null)
  const gridRows = pageRead.data?.rows ?? []
  // The cursor indexes *visible* columns, so hiding or reordering one must
  // move the inspector with it rather than leaving it pointed at an index that
  // now means a different column.
  const gridCols = useMemo(() => {
    const declared = (pageRead.data?.columns ?? []).map((c) => c.name)
    const all = declared.length > 0 ? declared : Object.keys(gridRows[0] ?? {})
    return view.visible(all)
  }, [pageRead.data?.columns, gridRows, view])

  const total = pageRead.data?.total ?? null
  const hasMore = pageRead.data?.has_more ?? false

  const copyText = useCallback((text: string) => {
    void navigator.clipboard?.writeText(text)
  }, [])

  const keyboard = useGridKeyboard({
    rows: gridRows,
    columns: gridCols,
    primaryKeys,
    containerRef: gridRef,
    hasPrevPage: page > 0,
    hasNextPage: hasMore,
    onTurnPage: (delta) => setPage((p) => Math.max(0, p + delta)),
    onCopyCell: (r, c) => copyText(cellText(gridRows[r]?.[gridCols[c]])),
    onCopyRow: (r) => {
      const row = gridRows[r]
      if (row) copyText(rowAsTsv(row, gridCols))
    },
    onActivate: (r) => setSelectedRow((cur) => (cur === r ? null : r)),
  })

  const cursorColumn = keyboard.cursor ? gridCols[keyboard.cursor.col] : undefined
  const cursorMeta = columnInfo.find((c) => c.name === cursorColumn)
  const totalPages = useMemo(() => {
    if (total !== null) return Math.max(1, Math.ceil(total / pageSize))
    // Unknown count (or count query failed): allow one more page only when the
    // sentinel row says a next page exists — bounds otherwise-unbounded "next".
    return hasMore ? page + 2 : page + 1
  }, [total, hasMore, page, pageSize])

  if (pageRead.error) {
    // The filter bar stays: when a filter itself caused the failure, hiding
    // it traps the user with no way to edit or remove the offending chip.
    return (
      <div className="db-data">
        <div className="db-data-main">
          <FilterBar columns={columnInfo} filters={filters} onChange={changeFilters} />
          <div className="db-pad">
            <StatusPanel
              variant="alert"
              icon={<AlertCircle size={18} />}
              headline="database read failed"
              detail={pageRead.error}
            />
          </div>
        </div>
      </div>
    )
  }

  if (pageRead.loading && !pageRead.data) {
    return (
      <div className="db-skel">
        <Skeleton style={{ display: 'block', height: 28, width: '100%' }} />
        <Skeleton style={{ display: 'block', height: 24, width: '100%' }} />
        <Skeleton style={{ display: 'block', height: 24, width: '100%' }} />
        <Skeleton style={{ display: 'block', height: 24, width: '66%' }} />
      </div>
    )
  }

  const result = pageRead.data
  const filtering = applied.length > 0
  if (!result || (result.rows.length === 0 && page === 0)) {
    return (
      <div className="db-data">
        <div className="db-data-main">
          <FilterBar columns={columnInfo} filters={filters} onChange={changeFilters} />
          <EmptyState
            icon={TableIcon}
            title={filtering ? 'no matching rows' : 'empty table'}
            description={
              filtering
                ? `no rows in ${table} match the ${applied.length} filter${applied.length === 1 ? '' : 's'} above`
                : `${table} has no rows`
            }
          />
        </div>
      </div>
    )
  }

  const rows = result.rows
  const detailRow = selectedRow !== null && selectedRow < rows.length ? rows[selectedRow] : null

  return (
    <div className="db-data">
      <div className="db-data-main">
        <FilterBar columns={columnInfo} filters={filters} onChange={changeFilters} />
        <div className="db-data-bar db-toolbar">
          <span className="name">{table}</span>
          <span>
            {(() => {
              const n = result.columns?.length ?? Object.keys(rows[0] ?? {}).length
              return `${n} column${n === 1 ? '' : 's'}`
            })()}
          </span>
          {/* A count is only trustworthy when it is described: "12,443 rows"
              beside three active filters reads as the table's size. The
              worker counts with the same filters applied, so say so. */}
          <span className={total === null ? 'db-pulse' : undefined}>
            {total !== null ? `${total.toLocaleString()} row${total === 1 ? '' : 's'}` : '… rows'}
            {filtering ? ' matching' : ''}
          </span>
          {/* The audible twin of the counts: rows and page after each load,
              silence while loading. */}
          <span className="db-sr-only" role="status">
            {pageRead.loading
              ? ''
              : `${(total ?? rows.length).toLocaleString()} rows, page ${page + 1} of ${totalPages}`}
          </span>
          {sort ? (
            <button
              type="button"
              className="db-linkish"
              onClick={() => {
                setSort(null)
                setPage(0)
              }}
            >
              sorted by {sort.column} {sort.dir} · clear
            </button>
          ) : null}
          {pageRead.loading ? (
            <span className="db-pulse" style={{ color: 'var(--color-ink-faint)' }}>
              · refreshing…
            </span>
          ) : null}
          {view.view.hidden.length > 0 ? (
            <button type="button" className="db-linkish" onClick={view.showAll}>
              {view.view.hidden.length} column{view.view.hidden.length === 1 ? '' : 's'} hidden · show all
            </button>
          ) : null}
          {Object.keys(view.view.widths).length > 0 || view.view.order.length > 0 ? (
            <button type="button" className="db-linkish" onClick={view.reset}>
              reset layout
            </button>
          ) : null}
          {onOpenInSql ? (
            <button type="button" className="db-linkish spacer" onClick={() => onOpenInSql(table)}>
              open in sql
            </button>
          ) : null}
        </div>
        {/* One tab stop: the grid handles arrows itself. The container only
            catches keys bubbling from the focused cell, so it is a scroll
            region rather than a control of its own. */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: scroll container catching keys bubbled from the focused gridcell */}
        <div className="db-data-scroll" ref={gridRef} onKeyDown={keyboard.onKeyDown}>
          <ResultGrid
            keyboard={keyboard}
            view={view}
            columns={result.columns}
            rows={rows}
            rowCount={total ?? rows.length}
            primaryKeys={primaryKeys}
            sort={sort}
            onSort={(next) => {
              setSort(next)
              setPage(0)
              setSelectedRow(null)
            }}
            onRowClick={(i) => setSelectedRow((cur) => (cur === i ? null : i))}
            selectedRow={selectedRow}
            foreignKeys={foreignKeys}
            onFollowFk={onFollow ? (fk, value) => onFollow(fk.table, fk.column, value) : undefined}
            stickyHeader
            hideFooter
          />
        </div>
        <div className="db-data-foot">
          {/* Advertised nowhere else — without this line the roving-cursor
              grid reads as mouse-only. */}
          <span className="db-kbd-hint">
            arrows move · pgup/pgdn turn pages · {MOD}c copies a cell · {MOD}⇧c the row
          </span>
          <Pagination
            currentPage={page + 1}
            totalPages={totalPages}
            totalItems={total ?? rows.length + page * pageSize}
            pageSize={pageSize}
            onPageChange={(next) => {
              setPage(next - 1)
              setSelectedRow(null)
            }}
            onPageSizeChange={(next) => {
              setPageSize(next)
              setPage(0)
              setSelectedRow(null)
            }}
          />
        </div>
      </div>
      {/* Three views of the same selection share one slot — at 320px they
          would otherwise compete for width. */}
      {detailRow || keyboard.cursor ? (
        <div className="db-rowdetail">
          <Tabs defaultValue={detailRow ? 'row' : 'cell'}>
            {/* The inspector opens implicitly (row click, cell cursor) — so
                it must close explicitly too: without this button, a cell
                cursor pinned the panel open with no way out. */}
            <div className="db-inspector-head">
              <TabsList>
                <TabsTrigger value="row" disabled={!detailRow}>
                  row
                </TabsTrigger>
                <TabsTrigger value="cell" disabled={!keyboard.cursor}>
                  cell
                </TabsTrigger>
                <TabsTrigger value="stats" disabled={!cursorColumn}>
                  stats
                </TabsTrigger>
              </TabsList>
              <button
                type="button"
                className="db-icon-btn"
                onClick={() => {
                  setSelectedRow(null)
                  keyboard.setCursor(null)
                }}
                aria-label="close inspector"
                title="close inspector"
              >
                <X size={12} />
              </button>
            </div>
            <TabsContent value="row">
              {detailRow ? (
                <RowDetail
                  table={table}
                  row={detailRow}
                  columns={columnInfo}
                  onClose={() => setSelectedRow(null)}
                  embedded
                />
              ) : null}
            </TabsContent>
            <TabsContent value="cell">
              {keyboard.cursor && cursorColumn ? (
                <CellInspector
                  table={table}
                  column={cursorColumn}
                  type={cursorMeta?.type}
                  value={gridRows[keyboard.cursor.row]?.[cursorColumn]}
                  row={keyboard.cursor.row}
                  rowCount={gridRows.length}
                  col={keyboard.cursor.col}
                  colCount={gridCols.length}
                  foreignKey={cursorMeta?.fk}
                  onFollow={onFollow ? (fk, v) => onFollow(fk.table, fk.column, v) : undefined}
                />
              ) : null}
            </TabsContent>
            <TabsContent value="stats">
              {cursorColumn ? <ColumnStatsPanel host={host} db={db} table={table} column={cursorColumn} /> : null}
            </TabsContent>
          </Tabs>
        </div>
      ) : null}
    </div>
  )
}
