/**
 * Paged read-only view of one table: `SELECT * LIMIT/OFFSET` (plus ORDER BY
 * when a column header is sorted) through `database::query`, a total row
 * count for the pager, and column introspection for PK markers + the row
 * inspector. The grid is the shared `ResultGrid`, so cells render the same
 * way as in the SQL panel and the chat family.
 */

import {
  EmptyState,
  type Host,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useMemo, useState } from 'react'
import {
  countTableRows,
  type DbDriver,
  fetchTablePage,
  PAGE_SIZE,
  type TableSort,
  tableColumns,
} from './db-data'
import { AlertCircle, type IconProps, Table2 } from './icons'
import { Pagination } from './pagination'
import { RowDetail } from './RowDetail'
import { ResultGrid } from './result-grid'
import { useDatabaseRead } from './useDatabaseRead'

interface TableDataPanelProps {
  host: Host
  db: string
  driver: DbDriver
  table: string
  enabled: boolean
  /** Seed the SQL panel with a SELECT for this table and switch to it. */
  onOpenInSql?: (table: string) => void
}

const TableIcon = (p: IconProps) => <Table2 size={28} {...p} />

export function TableDataPanel({
  host,
  db,
  driver,
  table,
  enabled,
  onOpenInSql,
}: TableDataPanelProps) {
  const [page, setPage] = useState(0)
  const [pageSize, setPageSize] = useState(PAGE_SIZE)
  const [sort, setSort] = useState<TableSort | null>(null)
  const [selectedRow, setSelectedRow] = useState<number | null>(null)

  const pageFetcher = useCallback(
    () => fetchTablePage(host, db, driver, table, page, pageSize, sort),
    [host, db, driver, table, page, pageSize, sort],
  )
  const countFetcher = useCallback(
    () => countTableRows(host, db, driver, table),
    [host, db, driver, table],
  )
  const columnsFetcher = useCallback(
    () => tableColumns(host, db, driver, table),
    [host, db, driver, table],
  )
  const pageRead = useDatabaseRead(enabled, pageFetcher)
  const countRead = useDatabaseRead(enabled, countFetcher)
  const columnsRead = useDatabaseRead(enabled, columnsFetcher)

  const columnInfo = useMemo(() => columnsRead.data ?? [], [columnsRead.data])
  const primaryKeys = useMemo(
    () => columnInfo.filter((c) => c.pk).map((c) => c.name),
    [columnInfo],
  )

  const total = countRead.data ?? null
  const hasMore = pageRead.data?.hasMore ?? false
  const totalPages = useMemo(() => {
    if (total !== null) return Math.max(1, Math.ceil(total / pageSize))
    // Unknown count (or count query failed): allow one more page only when the
    // sentinel row says a next page exists — bounds otherwise-unbounded "next".
    return hasMore ? page + 2 : page + 1
  }, [total, hasMore, page, pageSize])

  if (pageRead.error) {
    return (
      <StatusPanel
        variant="alert"
        icon={<AlertCircle size={18} />}
        headline="database read failed"
        detail={pageRead.error}
      />
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

  const result = pageRead.data?.result
  if (!result || (result.row_count === 0 && page === 0)) {
    return (
      <EmptyState
        icon={TableIcon}
        title="empty table"
        description={`${table} has no rows`}
      />
    )
  }

  const rows = result.rows
  const detailRow =
    selectedRow !== null && selectedRow < rows.length ? rows[selectedRow] : null

  return (
    <div className="db-data">
      <div className="db-data-main">
        <div className="db-data-bar">
          <span className="name">{table}</span>
          <span>
            {result.columns?.length ?? Object.keys(rows[0] ?? {}).length}{' '}
            columns
          </span>
          <span>{total !== null ? `${total} rows` : '… rows'}</span>
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
            <span
              className="db-pulse"
              style={{ color: 'var(--color-ink-ghost)' }}
            >
              refreshing…
            </span>
          ) : null}
          {onOpenInSql ? (
            <button
              type="button"
              className="db-linkish spacer"
              onClick={() => onOpenInSql(table)}
            >
              open in sql
            </button>
          ) : null}
        </div>
        <div className="db-data-scroll">
          <ResultGrid
            columns={result.columns}
            rows={rows}
            rowCount={total ?? result.row_count}
            primaryKeys={primaryKeys}
            sort={sort}
            onSort={(next) => {
              setSort(next)
              setPage(0)
              setSelectedRow(null)
            }}
            onRowClick={(i) => setSelectedRow((cur) => (cur === i ? null : i))}
            selectedRow={selectedRow}
            stickyHeader
            hideFooter
          />
        </div>
        <div className="db-data-foot">
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
      {detailRow ? (
        <RowDetail
          table={table}
          row={detailRow}
          columns={columnInfo}
          onClose={() => setSelectedRow(null)}
        />
      ) : null}
    </div>
  )
}
