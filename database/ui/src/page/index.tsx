/**
 * The database page (#/ext/database): the database worker's read surface —
 * configured databases, their schema (tables + columns via the driver's own
 * catalog, since the worker has no introspection functions), and paged,
 * sortable table contents with a row inspector, plus an ad-hoc read-only SQL
 * panel. Data loads on demand (db/table switch / refresh) — the worker emits
 * no change events yet, so there is nothing to subscribe to. Read-only on
 * purpose: INSERT/UPDATE/DDL stay in agent flows behind the approval gate.
 *
 * The host only mounts this page when the database worker is connected, so
 * there is no presence gate here; a failed `listDatabases` surfaces as the
 * "worker unavailable" panel below.
 */

import {
  Badge,
  Button,
  EmptyState,
  type Host,
  Select,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import {
  type DbInfo,
  listDbs,
  listTables,
  PAGE_SIZE,
  quoteTableRef,
} from './db-data'
import {
  AlertCircle,
  Database,
  type IconProps,
  RefreshCw,
  Table2,
} from './icons'
import { PAGE_CSS } from './page-styles'
import { SchemaTree } from './SchemaTree'
import { SqlPanel } from './SqlPanel'
import { TableDataPanel } from './TableDataPanel'
import { useDatabaseRead } from './useDatabaseRead'

type PanelMode = 'data' | 'sql'

const DatabaseIcon = (p: IconProps) => <Database size={28} {...p} />
const TableIcon = (p: IconProps) => <Table2 size={28} {...p} />

export function DatabasePage({ host }: { host: Host }) {
  const [selectedDb, setSelectedDb] = useState<string | undefined>(undefined)
  const [selectedTable, setSelectedTable] = useState<string | null>(null)
  const [mode, setMode] = useState<PanelMode>('data')
  const [seedSql, setSeedSql] = useState<string | undefined>(undefined)
  const [bump, setBump] = useState(0)

  const dbsFetcher = useCallback(() => listDbs(host), [host])
  const dbsRead = useDatabaseRead(true, dbsFetcher)
  const dbs = dbsRead.data ?? []
  const activeDb: DbInfo | undefined =
    dbs.find((db) => db.name === selectedDb) ?? dbs[0]

  const tablesFetcher = useCallback(() => {
    if (!activeDb) return Promise.resolve([])
    return listTables(host, activeDb.name, activeDb.driver)
  }, [host, activeDb])
  const tablesRead = useDatabaseRead(!!activeDb, tablesFetcher)
  const tables = tablesRead.data ?? []

  // Keep the selected table valid when the db or its table list changes.
  useEffect(() => {
    if (selectedTable && !tables.some((t) => t.name === selectedTable)) {
      setSelectedTable(null)
    }
  }, [tables, selectedTable])

  const refresh = () => {
    dbsRead.refresh()
    tablesRead.refresh()
    setBump((b) => b + 1)
  }

  // A configured database without a `url` (redacted in the worker's config
  // response) still counts as configured — fall back to its name, not to the
  // "nothing here" copy.
  const subtitle = activeDb
    ? (activeDb.url ?? activeDb.name)
    : dbsRead.error
      ? 'worker not connected'
      : 'no database configured'

  return (
    <div className="db-page">
      <style>{PAGE_CSS}</style>
      <div className="db-head">
        <div>
          <div className="db-title">database</div>
          <div className="db-sub">{subtitle}</div>
        </div>
        <div className="db-controls">
          {activeDb ? (
            <>
              <div className="db-modes">
                {(['data', 'sql'] as const).map((m) => (
                  <button
                    key={m}
                    type="button"
                    className={`db-mode${mode === m ? ' active' : ''}`}
                    aria-pressed={mode === m}
                    onClick={() => setMode(m)}
                  >
                    {m}
                  </button>
                ))}
              </div>
              <Badge variant="accent">{activeDb.driver}</Badge>
              {dbs.length > 1 ? (
                <Select
                  value={activeDb.name}
                  options={dbs.map((db) => ({
                    value: db.name,
                    label: db.name,
                    title: db.url,
                  }))}
                  onChange={(next) => {
                    setSelectedDb(next)
                    setSelectedTable(null)
                  }}
                  aria-label="database"
                />
              ) : null}
            </>
          ) : null}
          <Button variant="ghost" size="sm" onClick={refresh}>
            <RefreshCw size={14} aria-hidden />
            refresh
          </Button>
        </div>
      </div>

      {dbsRead.error ? (
        <div style={{ marginTop: 16 }}>
          <StatusPanel
            variant="alert"
            icon={<AlertCircle size={18} />}
            headline="database worker unavailable"
            detail={dbsRead.error}
          />
        </div>
      ) : dbsRead.loading && dbs.length === 0 ? (
        <div className="db-msg db-pulse" style={{ marginTop: 16 }}>
          · loading databases…
        </div>
      ) : dbs.length === 0 ? (
        <div style={{ marginTop: 16 }}>
          <EmptyState
            icon={DatabaseIcon}
            title="no databases configured"
            description="configure a database in the worker's config (databases: { name: { url } }) and it appears here."
          />
        </div>
      ) : (
        <div className="db-body">
          <aside className="db-aside">
            <div className="db-aside-head">
              tables
              {tables.length > 0
                ? ` · ${tables.filter((t) => t.kind === 'table').length}`
                : ''}
            </div>
            <div className="db-aside-body">
              {tablesRead.error ? (
                <p
                  className="db-tree-msg alert"
                  style={{ padding: '8px 12px' }}
                >
                  {tablesRead.error}
                </p>
              ) : tablesRead.loading && tables.length === 0 ? (
                <div className="db-skel">
                  <Skeleton
                    style={{ display: 'block', height: 20, width: '100%' }}
                  />
                  <Skeleton
                    style={{ display: 'block', height: 20, width: '75%' }}
                  />
                  <Skeleton
                    style={{ display: 'block', height: 20, width: '83%' }}
                  />
                </div>
              ) : tables.length === 0 ? (
                <p className="db-msg">no tables</p>
              ) : activeDb ? (
                // Remount on db/refresh so lazily-cached columns re-read.
                <SchemaTree
                  key={`${activeDb.name}:${bump}`}
                  host={host}
                  db={activeDb.name}
                  driver={activeDb.driver}
                  tables={tables}
                  selectedTable={selectedTable}
                  onSelectTable={setSelectedTable}
                />
              ) : null}
            </div>
          </aside>
          <div className="db-panel">
            {activeDb ? (
              mode === 'sql' ? (
                // Keyed by db only: a refresh reloads metadata, it must not
                // wipe an in-progress statement or its results.
                <SqlPanel
                  key={activeDb.name}
                  host={host}
                  db={activeDb.name}
                  driver={activeDb.driver}
                  seedSql={seedSql}
                  tables={tables.map((t) => t.name)}
                />
              ) : !selectedTable ? (
                <div className="db-pad">
                  <EmptyState
                    icon={TableIcon}
                    title="select a table"
                    description={
                      tables.length > 0
                        ? `${tables.length} table${tables.length === 1 ? '' : 's'} in ${activeDb.name} — pick one to browse its rows, sort columns, and inspect values.`
                        : `no tables in ${activeDb.name} yet`
                    }
                  />
                </div>
              ) : (
                // Remount on db/table/refresh so every fetch hook restarts clean.
                <TableDataPanel
                  key={`${activeDb.name}:${selectedTable}:${bump}`}
                  host={host}
                  db={activeDb.name}
                  driver={activeDb.driver}
                  table={selectedTable}
                  enabled
                  onOpenInSql={(table) => {
                    setSeedSql(
                      `SELECT * FROM ${quoteTableRef(activeDb.driver, table)} LIMIT ${PAGE_SIZE}`,
                    )
                    setMode('sql')
                  }}
                />
              )
            ) : null}
          </div>
        </div>
      )}
    </div>
  )
}
