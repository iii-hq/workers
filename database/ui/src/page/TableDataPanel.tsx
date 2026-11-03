/**
 * Paged view of one table.
 *
 * Reads go through `database::browseTable`, which takes typed filters and
 * sorts and returns the page plus a total honouring the same filters — one
 * call where this used to make two and build its own SQL. The page no longer
 * writes `SELECT`, and neither does an agent asking the same question.
 *
 * The grid stays read-only; the one write is New row (tables only), a form
 * that inserts through `database::insertRow`.
 *
 * The grid is the shared `ResultGrid`, so cells render the same way as in the
 * SQL panel and the chat family.
 */

import {
  Button,
  EmptyState,
  type Host,
  IconButton,
  Separator,
  ShadcnBadge,
  Skeleton,
  StatusPanel,
  ShadcnTabs as Tabs,
  ShadcnTabsContent as TabsContent,
  ShadcnTabsList as TabsList,
  ShadcnTabsTrigger as TabsTrigger,
  uiClasses,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import {
  ArrowDown,
  ArrowUp,
  ChartColumn,
  Braces,
  CircleAlert,
  Columns3,
  Eye,
  LoaderCircle,
  Plus,
  RefreshCw,
  Rows3,
  SquareTerminal,
  Table2,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { cellText, rowAsTsv } from '../lib/grid-cursor'
import { type FilterSpec, isComplete, PAGE_SIZE } from '../lib/rpc'
import { CellInspector } from './CellInspector'
import { ColumnStatsPanel } from './ColumnStatsPanel'
import { browseTableRef, type DbDriver, distinctValuesRef, type TableSort, tableColumns } from './db-data'
import { FilterBar, type SuggestFn } from './FilterBar'
import { NewRowDialog } from './NewRowDialog'
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
  /** Views never offer New row. */
  kind?: 'table' | 'view'
  /** `database::insertRow` is registered. */
  canInsert?: boolean
  /** The worker takes raw SQL filter chips. */
  canRawFilter?: boolean
  /** `database::distinctValues` is registered. */
  canSuggest?: boolean
}

type InspectorTab = 'row' | 'cell' | 'stats'

const SUGGESTIONS = 12
const INSERTED_NOTE_MS = 4000

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
  kind,
  canInsert,
  canRawFilter,
  canSuggest,
}: TableDataPanelProps) {
  const [page, setPage] = useState(0)
  const [newRowOpen, setNewRowOpen] = useState(false)
  const [insertedNote, setInsertedNote] = useState<string | null>(null)
  useEffect(() => {
    if (!insertedNote) return
    const timer = setTimeout(() => setInsertedNote(null), INSERTED_NOTE_MS)
    return () => clearTimeout(timer)
  }, [insertedNote])
  const [pageSize, setPageSize] = useState(PAGE_SIZE)
  const [sort, setSort] = useState<TableSort | null>(null)
  const [selectedRow, setSelectedRow] = useState<number | null>(null)
  const [filters, setFilters] = useState<FilterSpec[]>(initialFilters ?? [])
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>('row')

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

  const suggest = useCallback<SuggestFn>(
    (column, search) =>
      distinctValuesRef(host, db, table, column, {
        search: search || undefined,
        limit: SUGGESTIONS,
      }).then((r) => r.values),
    [host, db, table],
  )

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

  const keyboard = useGridKeyboard({
    rows: gridRows,
    columns: gridCols,
    primaryKeys,
    containerRef: gridRef,
    hasPrevPage: page > 0,
    hasNextPage: hasMore,
    onTurnPage: (delta) => setPage((p) => Math.max(0, p + delta)),
    onCopyCell: (r, c) => void copyText(cellText(gridRows[r]?.[gridCols[c]])),
    onCopyRow: (r) => {
      const row = gridRows[r]
      if (row) void copyText(rowAsTsv(row, gridCols))
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

  const dialect = driver === 'unknown' ? 'unknown' : driver
  const filterBar = (
    <FilterBar
      columns={columnInfo}
      filters={filters}
      onChange={changeFilters}
      allowRaw={canRawFilter}
      dialect={dialect}
      suggest={canSuggest ? suggest : undefined}
    />
  )
  const insertable = kind === 'table' && canInsert === true
  const newRow = insertable ? (
    <NewRowDialog
      host={host}
      db={db}
      table={table}
      columns={columnInfo}
      dialect={dialect}
      open={newRowOpen}
      onOpenChange={setNewRowOpen}
      onInserted={() => {
        setInsertedNote(`Row inserted into ${table}`)
        pageRead.refresh()
      }}
    />
  ) : null

  const result = pageRead.data
  const filtering = applied.length > 0
  const rows = result?.rows ?? []
  const columnCount = result?.columns?.length ?? Object.keys(rows[0] ?? {}).length

  // One toolbar in every state: when a filter itself caused a failure (or
  // matched nothing), hiding the bar would trap the user with no way to edit
  // or remove the offending chip.
  const toolbar = (
    <div className="db-data-toolbar" role="toolbar" aria-label="table data">
      <div className="db-data-title">
        {kind === 'view' ? <Eye size={16} aria-hidden /> : <Table2 size={16} aria-hidden />}
        <span className="name" title={table}>
          {table}
        </span>
        {kind === 'view' ? <ShadcnBadge variant="outline">view</ShadcnBadge> : null}
      </div>
      <Separator orientation="vertical" className="db-data-sep" />
      {filterBar}
      <span className="db-toolbar-spacer" />
      {pageRead.loading && result ? (
        <span className="db-data-busy" role="status">
          <LoaderCircle size={16} className={uiClasses.spin} aria-hidden />
          Refreshing
        </span>
      ) : null}
      {insertedNote ? (
        <span className="db-data-note" role="status">
          {insertedNote}
        </span>
      ) : null}
      {sort ? (
        <Button
          variant="ghost"
          size="sm"
          className="db-sort-chip"
          title="clear the sort"
          onClick={() => {
            setSort(null)
            setPage(0)
          }}
        >
          {sort.dir === 'asc' ? <ArrowUp size={16} aria-hidden /> : <ArrowDown size={16} aria-hidden />}
          Sorted by {sort.column}
          <X size={16} aria-hidden />
        </Button>
      ) : null}
      {view.view.hidden.length > 0 ? (
        <Button variant="ghost" size="sm" onClick={view.showAll}>
          <Columns3 size={16} aria-hidden />
          {view.view.hidden.length} hidden · Show all
        </Button>
      ) : null}
      {Object.keys(view.view.widths).length > 0 || view.view.order.length > 0 ? (
        <Button variant="ghost" size="sm" onClick={view.reset}>
          Reset layout
        </Button>
      ) : null}
      {onOpenInSql ? (
        <Button variant="secondary" size="sm" onClick={() => onOpenInSql(table)}>
          <SquareTerminal size={16} aria-hidden />
          Open in SQL
        </Button>
      ) : null}
      {insertable ? (
        <Button variant="primary" size="sm" onClick={() => setNewRowOpen(true)}>
          <Plus size={16} aria-hidden />
          Insert row
        </Button>
      ) : null}
    </div>
  )

  if (pageRead.error) {
    return (
      <div className="db-data">
        <div className="db-data-main">
          {toolbar}
          <div className="db-pad">
            <StatusPanel
              variant="alert"
              icon={<CircleAlert size={18} />}
              headline={filtering ? 'Read failed — check the filters above' : 'Database read failed'}
              detail={pageRead.error}
              action={
                <Button variant="secondary" size="sm" onClick={pageRead.refresh}>
                  <RefreshCw size={16} aria-hidden />
                  Retry
                </Button>
              }
            />
          </div>
        </div>
        {newRow}
      </div>
    )
  }

  if (pageRead.loading && !pageRead.data) {
    return (
      <div className="db-data">
        <div className="db-data-main">
          {toolbar}
          <div className="db-skel">
            <Skeleton style={{ display: 'block', height: 36, width: '100%' }} />
            <Skeleton style={{ display: 'block', height: 32, width: '100%' }} />
            <Skeleton style={{ display: 'block', height: 32, width: '100%' }} />
            <Skeleton style={{ display: 'block', height: 32, width: '66%' }} />
          </div>
        </div>
      </div>
    )
  }

  if (!result || (result.rows.length === 0 && page === 0)) {
    return (
      <div className="db-data">
        <div className="db-data-main">
          {toolbar}
          <div className="db-data-empty">
            <EmptyState
              icon={Table2}
              title={filtering ? 'No matching rows' : 'This table is empty'}
              description={
                filtering
                  ? `No rows in ${table} match the ${applied.length} filter${applied.length === 1 ? '' : 's'} above.`
                  : insertable
                    ? `${table} has no rows yet. Insert one to get started.`
                    : `${table} has no rows yet.`
              }
              action={insertable ? { label: 'Insert row', onClick: () => setNewRowOpen(true) } : undefined}
            />
          </div>
        </div>
        {newRow}
      </div>
    )
  }

  const detailRow = selectedRow !== null && selectedRow < rows.length ? rows[selectedRow] : null
  // The picked tab, unless what it shows is gone: no row selected falls to
  // the cell, no cursor falls back to the row.
  const shownTab: InspectorTab =
    inspectorTab === 'row' && !detailRow ? 'cell' : inspectorTab !== 'row' && !keyboard.cursor ? 'row' : inspectorTab

  return (
    <div className="db-data">
      <div className="db-data-main">
        {toolbar}
        {/* The audible twin of the counts: rows and page after each load,
            silence while loading. */}
        <span className="db-sr-only" role="status">
          {pageRead.loading ? '' : `${(total ?? rows.length).toLocaleString()} rows, page ${page + 1} of ${totalPages}`}
        </span>
        {/* One tab stop: the grid handles arrows itself. The container only
            catches keys bubbling from the focused cell, so it is a scroll
            region rather than a control of its own. */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: scroll container catching keys bubbled from the focused gridcell */}
        <div
          className={`db-data-scroll${pageRead.loading ? ' loading' : ''}`}
          ref={gridRef}
          onKeyDown={keyboard.onKeyDown}
          aria-busy={pageRead.loading || undefined}
        >
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
            rowNumbers
            rowOffset={page * pageSize}
          />
        </div>
        <div className="db-data-foot">
          <Pagination
            currentPage={page + 1}
            totalPages={totalPages}
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
          {/* A count is only trustworthy when it is described: "12,443
              records" beside three active filters reads as the table's
              size. The worker counts with the same filters applied. */}
          <span className={`db-foot-stat${total === null ? ` ${uiClasses.pulse}` : ''}`}>
            {total !== null ? `${total.toLocaleString()} record${total === 1 ? '' : 's'}` : '… records'}
            {filtering ? ' matching' : ''}
          </span>
          <span className="db-foot-stat faint">
            {columnCount} column{columnCount === 1 ? '' : 's'}
            {typeof result.elapsed_ms === 'number' ? ` · ${result.elapsed_ms} ms` : ''}
          </span>
          <span className="db-toolbar-spacer" />
          {/* Advertised nowhere else — without this line the roving-cursor
              grid reads as mouse-only. */}
          <span className="db-kbd-hint">
            Arrows move · PgUp/PgDn turn pages · {MOD}C copies a cell · {MOD}⇧C the row
          </span>
        </div>
      </div>
      {/* Three views of the same selection share one slot — at 320px they
          would otherwise compete for width. */}
      {detailRow || keyboard.cursor ? (
        <aside className="db-rowdetail" aria-label="inspector">
          <Tabs
            value={shownTab}
            onValueChange={(next) => setInspectorTab(next as InspectorTab)}
            className="db-inspector"
          >
            {/* The inspector opens implicitly (row click, cell cursor) — so
                it must close explicitly too: without this button, a cell
                cursor pinned the panel open with no way out. */}
            <div className="db-inspector-head">
              <TabsList variant="line" className="db-inspector-tabs">
                <TabsTrigger value="row" disabled={!detailRow}>
                  <Rows3 size={16} aria-hidden />
                  Row
                </TabsTrigger>
                <TabsTrigger value="cell" disabled={!keyboard.cursor}>
                  <Braces size={16} aria-hidden />
                  Cell
                </TabsTrigger>
                <TabsTrigger value="stats" disabled={!cursorColumn}>
                  <ChartColumn size={16} aria-hidden />
                  Stats
                </TabsTrigger>
              </TabsList>
              <IconButton
                label="close inspector"
                onClick={() => {
                  setSelectedRow(null)
                  keyboard.setCursor(null)
                }}
              >
                <X size={16} />
              </IconButton>
            </div>
            <TabsContent value="row" className="db-inspector-body">
              {detailRow ? <RowDetail row={detailRow} columns={columnInfo} /> : null}
            </TabsContent>
            <TabsContent value="cell" className="db-inspector-body">
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
            <TabsContent value="stats" className="db-inspector-body">
              {cursorColumn ? <ColumnStatsPanel host={host} db={db} table={table} column={cursorColumn} /> : null}
            </TabsContent>
          </Tabs>
        </aside>
      ) : null}
      {newRow}
    </div>
  )
}
