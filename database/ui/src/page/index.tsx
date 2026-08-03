/**
 * The database page (#/ext/database): the database worker's read surface —
 * configured databases, their schema, paged and sortable table contents with a
 * row inspector, an ad-hoc read-only SQL panel, a schema diagram, connection
 * health, and the writes landing in the selected table as they commit.
 *
 * Nothing here computes what the worker can compute. The catalog, the filter
 * compiler, the plan parser and the diagram layout all live in `handlers/`,
 * which is why an agent can ask the same questions this page answers. The page
 * fetches and draws.
 *
 * Read-only on purpose: INSERT/UPDATE/DDL stay in agent flows behind the
 * approval gate.
 *
 * The host only mounts this page when the database worker is connected, so
 * there is no presence gate here; a failed `listDatabases` surfaces as the
 * "worker unavailable" panel below.
 */

import { Badge, Button, EmptyState, type Host, Select, Skeleton, StatusPanel } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { ALL, type Capabilities, probe } from '../lib/capabilities'
import { DB } from '../lib/rpc'
import { ChangesPanel } from './ChangesPanel'
import { type DbInfo, listDbs, listTables, PAGE_SIZE, quoteTableRef } from './db-data'
import { ErdPanel } from './ErdPanel'
import { HealthPanel } from './HealthPanel'
import { AlertCircle, Database, type IconProps, RefreshCw, Table2 } from './icons'
import { SchemaTree } from './SchemaTree'
import { SqlPanel } from './SqlPanel'
import { TableDataPanel } from './TableDataPanel'
import { useDatabaseRead } from './useDatabaseRead'

type PanelMode = 'data' | 'sql' | 'diagram' | 'health' | 'changes'

/**
 * Modes whose function the worker may not have. A panel whose function is
 * missing is hidden rather than rendered as a control that always errors — an
 * older worker should look smaller, not broken.
 */
const MODE_REQUIRES: Partial<Record<PanelMode, string>> = {
  diagram: DB.schemaDiagram,
  health: DB.health,
}

const ALWAYS: PanelMode[] = ['data', 'sql']

/** One step of following a foreign key. */
interface Hop {
  table: string
  column: string
  value: unknown
}

const DatabaseIcon = (p: IconProps) => <Database size={28} {...p} />
const TableIcon = (p: IconProps) => <Table2 size={28} {...p} />

export function DatabasePage({ host }: { host: Host }) {
  const [selectedDb, setSelectedDb] = useState<string | undefined>(undefined)
  const [selectedTable, setSelectedTable] = useState<string | null>(null)
  const [mode, setMode] = useState<PanelMode>('data')
  const [seedSql, setSeedSql] = useState<string | undefined>(undefined)
  const [bump, setBump] = useState(0)
  const [caps, setCaps] = useState<Capabilities>(ALL)
  // Where following foreign keys has taken you. The last entry supplies the
  // filter the current table opens with.
  const [trail, setTrail] = useState<Hop[]>([])

  const follow = useCallback((table: string, column: string, value: unknown) => {
    setTrail((prev) => [...prev, { table, column, value }])
    setSelectedTable(table)
    setMode('data')
  }, [])

  /** Truncate the trail at `depth`, the way a breadcrumb behaves. */
  const goBackTo = useCallback((depth: number) => {
    setTrail((prev) => {
      const next = prev.slice(0, depth)
      const last = next[next.length - 1]
      setSelectedTable(last ? last.table : null)
      return next
    })
  }, [])

  const arrival = useMemo(() => {
    const last = trail[trail.length - 1]
    if (!last) return undefined
    return [{ column: last.column, op: 'equals' as const, value: last.value }]
  }, [trail])

  /**
   * Identity of the arrival filter, for the panel's remount key.
   *
   * `TableDataPanel` seeds its filters from `initialFilters` on mount only, so
   * arriving at the *same* table with a different value — a self-referencing
   * key, or a breadcrumb jump that truncates the trail — must remount it.
   * Keying on trail length alone missed both.
   */
  const arrivalKey = useMemo(() => {
    const last = trail[trail.length - 1]
    return last ? `${trail.length}:${last.column}=${String(last.value)}` : '0'
  }, [trail])

  // One probe per mount: which optional functions this worker actually has.
  useEffect(() => {
    let alive = true
    probe(host).then((c) => {
      if (alive) setCaps(c)
    })
    return () => {
      alive = false
    }
  }, [host])

  const modes = useMemo(() => {
    const optional = (['diagram', 'health', 'changes'] as const).filter((m) => {
      const needed = MODE_REQUIRES[m]
      return !needed || caps.has(needed as never)
    })
    return [...ALWAYS, ...optional] as PanelMode[]
  }, [caps])

  // Never leave the page on a mode that just disappeared.
  useEffect(() => {
    if (!modes.includes(mode)) setMode('data')
  }, [modes, mode])

  const dbsFetcher = useCallback(() => listDbs(host), [host])
  const dbsRead = useDatabaseRead(true, dbsFetcher)
  const dbs = dbsRead.data ?? []
  const activeDb: DbInfo | undefined = dbs.find((db) => db.name === selectedDb) ?? dbs[0]

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
      <div className="db-head">
        <div>
          <div className="db-title">database</div>
          <div className="db-sub">{subtitle}</div>
        </div>
        <div className="db-controls">
          {activeDb ? (
            <>
              <div className="db-modes">
                {modes.map((m) => (
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
              {tables.length > 0 ? ` · ${tables.filter((t) => t.kind === 'table').length}` : ''}
            </div>
            <div className="db-aside-body">
              {tablesRead.error ? (
                <p className="db-tree-msg alert" style={{ padding: '8px 12px' }}>
                  {tablesRead.error}
                </p>
              ) : tablesRead.loading && tables.length === 0 ? (
                <div className="db-skel">
                  <Skeleton style={{ display: 'block', height: 20, width: '100%' }} />
                  <Skeleton style={{ display: 'block', height: 20, width: '75%' }} />
                  <Skeleton style={{ display: 'block', height: 20, width: '83%' }} />
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
                  onSelectTable={(t) => {
                    // Choosing from the tree is a fresh start, not another hop.
                    setTrail([])
                    setSelectedTable(t)
                  }}
                />
              ) : null}
            </div>
          </aside>
          <div className="db-panel">
            {trail.length > 0 && mode === 'data' ? (
              <nav className="db-trail" aria-label="foreign key trail">
                <button type="button" className="db-trail-step" onClick={() => goBackTo(0)}>
                  {trail[0].table === selectedTable ? 'all tables' : 'start'}
                </button>
                {trail.map((hop, i) => (
                  <span key={`${hop.table}-${i}`} className="db-trail-seg">
                    <span className="db-trail-sep">›</span>
                    <button
                      type="button"
                      className={`db-trail-step${i === trail.length - 1 ? ' current' : ''}`}
                      onClick={() => goBackTo(i + 1)}
                    >
                      {hop.table}.{hop.column} = {String(hop.value)}
                    </button>
                  </span>
                ))}
              </nav>
            ) : null}
            {activeDb ? (
              mode === 'sql' ? (
                // Keyed by db only: a refresh reloads metadata, it must not
                // wipe an in-progress statement or its results.
                <SqlPanel
                  key={activeDb.name}
                  host={host}
                  db={activeDb.name}
                  seedSql={seedSql}
                  tables={tables.map((t) => t.name)}
                />
              ) : mode === 'diagram' ? (
                <ErdPanel
                  // Keyed by the selected table so switching in the tree opens
                  // the diagram around it rather than leaving the old focus.
                  key={`${activeDb.name}:${bump}:${selectedTable ?? ''}`}
                  host={host}
                  db={activeDb.name}
                  focusTable={selectedTable}
                />
              ) : mode === 'health' ? (
                <HealthPanel key={activeDb.name} host={host} db={activeDb.name} />
              ) : mode === 'changes' ? (
                // Keyed by table: the feed is per-binding, and a switch must
                // not carry the previous table's events across.
                <ChangesPanel
                  key={`${activeDb.name}:${selectedTable ?? ''}`}
                  host={host}
                  db={activeDb.name}
                  table={selectedTable}
                  kind={tables.find((t) => t.name === selectedTable)?.kind}
                  onRefresh={refresh}
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
                  // Keyed by the arrival filter too: following a key is a
                  // navigation, and the panel must restart with it applied.
                  key={`${activeDb.name}:${selectedTable}:${bump}:${arrivalKey}`}
                  host={host}
                  db={activeDb.name}
                  driver={activeDb.driver}
                  table={selectedTable}
                  initialFilters={arrival}
                  onFollow={follow}
                  enabled
                  onOpenInSql={(table) => {
                    setSeedSql(`SELECT * FROM ${quoteTableRef(activeDb.driver, table)} LIMIT ${PAGE_SIZE}`)
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
