/**
 * The database page (#/ext/database): the database worker's console surface —
 * configured databases, their schema, paged and sortable table contents with a
 * row inspector, an ad-hoc SQL panel, a schema diagram, connection health, and
 * the writes landing in the selected table as they commit.
 *
 * Nothing here computes what the worker can compute. The catalog, the filter
 * compiler, the plan parser and the diagram layout all live in `handlers/`,
 * which is why an agent can ask the same questions this page answers. The page
 * fetches and draws.
 *
 * The grid and its affordances stay read-only; the SQL panel runs writes,
 * routed through `database::execute` — the same function agents call, so
 * engine policy and the `database::row-changed` feed treat both alike.
 *
 * The host only mounts this page when the database worker is connected, so
 * there is no presence gate here; a failed `listDatabases` surfaces as the
 * "worker unavailable" panel below.
 */

import { Badge, Button, EmptyState, type Host, Select, Skeleton, StatusPanel } from '@iii-dev/console-ui'
import { type ComponentType, useCallback, useEffect, useMemo, useState } from 'react'
import { ALL, type Capabilities, MIN_VERSION_HINT, probe } from '../lib/capabilities'
import { DB } from '../lib/rpc'
import { ChangesPanel } from './ChangesPanel'
import { type DbInfo, listDbs, listTables, PAGE_SIZE, quoteTableRef } from './db-data'
import { ErdPanel } from './ErdPanel'
import { HealthPanel } from './HealthPanel'
import { AlertCircle, ChevronLeft, ChevronRight, Database, type IconProps, RefreshCw, Settings, Table2 } from './icons'
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

/** Collapsed state of the table rail, remembered per browser. */
const ASIDE_KEY = 'iii-console:database:aside'

/**
 * Fallback route for consoles that predate the shared configuration dialog:
 * the workers-tab editor deep-link. It navigates away from this page — the
 * lesser experience, kept only as the degradation path.
 */
const CONFIG_HASH = '#/workers/configuration/database'

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
  // Configuration opens HERE, in the console's own editor dialog — schema
  // fetch, custom-form resolution, dirty guard and save are all host-owned,
  // shared with the workers tab rather than duplicated. Read off
  // `host.components` at runtime, never imported: a console predating the
  // export degrades to navigation instead of failing the module load.
  const [configOpen, setConfigOpen] = useState(false)
  const HostConfigDialog = host.components.WorkerConfigurationDialog as
    | ComponentType<{ configurationId: string | null; onClose: () => void }>
    | undefined
  const openConfiguration = () => {
    if (HostConfigDialog) setConfigOpen(true)
    else window.location.hash = CONFIG_HASH
  }
  const [caps, setCaps] = useState<Capabilities>(ALL)
  // Where following foreign keys has taken you. The last entry supplies the
  // filter the current table opens with.
  const [trail, setTrail] = useState<Hop[]>([])
  // At dock widths the table rail costs every tab its first screenful, and
  // two of the five modes barely use it — so it collapses, and stays where
  // you left it.
  const [asideOpen, setAsideOpen] = useState(() => {
    try {
      return window.localStorage.getItem(ASIDE_KEY) !== 'closed'
    } catch {
      return true
    }
  })

  const toggleAside = () => {
    setAsideOpen((open) => {
      try {
        window.localStorage.setItem(ASIDE_KEY, open ? 'closed' : 'open')
      } catch {
        // remembering the rail is a convenience, not state
      }
      return !open
    })
  }

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

  // Hidden-not-broken leaves a trace: an older worker's page looks smaller,
  // and this one line says why (MIN_VERSION_HINT exists for exactly this).
  const hiddenModes = useMemo(
    () => (['diagram', 'health', 'changes'] as const).filter((m) => !modes.includes(m)),
    [modes],
  )

  // Never leave the page on a mode that just disappeared.
  useEffect(() => {
    if (!modes.includes(mode)) setMode('data')
  }, [modes, mode])

  const dbsFetcher = useCallback(() => listDbs(host), [host])
  const dbsRead = useDatabaseRead(true, dbsFetcher)
  const dbs = dbsRead.data ?? []
  const activeDb: DbInfo | undefined = dbs.find((db) => db.name === selectedDb) ?? dbs[0]

  // The list travels with the name of the database it came from. The read
  // hook keeps its previous data while a switch's fetch is in flight or
  // failed — untagged, mysql's tables kept rendering under a postgres
  // header, down to a stale "14 tables in postgres" count beside the
  // postgres error.
  const tablesFetcher = useCallback(() => {
    if (!activeDb) return Promise.resolve(null)
    const db = activeDb.name
    return listTables(host, db, activeDb.driver).then((list) => ({ db, list }))
  }, [host, activeDb])
  const tablesRead = useDatabaseRead(!!activeDb, tablesFetcher)
  const tables = tablesRead.data && tablesRead.data.db === activeDb?.name ? tablesRead.data.list : []
  const tableCount = tables.filter((t) => t.kind === 'table').length
  const viewCount = tables.length - tableCount

  // Selection means "open these rows" on data, a focus on the diagram, a
  // binding on changes — sql and health barely use it, and the rail says so
  // rather than letting the highlight imply navigation everywhere.
  const passiveAside = mode === 'sql' || mode === 'health'

  // Keep the selected table valid when the db or its table list changes.
  useEffect(() => {
    if (selectedTable && !tables.some((t) => t.name === selectedTable)) {
      setSelectedTable(null)
    }
  }, [tables, selectedTable])

  /**
   * A statement the SQL panel can offer that runs here. Prefers the selected
   * table, falls back to the first one — the point is that it is real, not
   * that it is interesting.
   */
  const starterSql = useMemo(() => {
    if (!activeDb) return undefined
    const target = tables.find((t) => t.name === selectedTable) ?? tables[0]
    if (!target) return undefined
    // Lowercase like every other string on the page — the panel speaks one
    // case.
    return `select * from ${quoteTableRef(activeDb.driver, target.name)} limit ${PAGE_SIZE}`
  }, [activeDb, tables, selectedTable])

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
          <Button variant="ghost" size="sm" onClick={openConfiguration}>
            <Settings size={14} aria-hidden />
            configure
          </Button>
        </div>
      </div>

      {HostConfigDialog ? (
        <HostConfigDialog
          configurationId={configOpen ? 'database' : null}
          onClose={() => {
            setConfigOpen(false)
            // A save may have happened in there: the worker hot-reloads its
            // pools, and this page re-reads what it derived from them.
            refresh()
          }}
        />
      ) : null}

      {dbs.length > 0 && hiddenModes.length > 0 ? (
        <p className="db-modes-hint">
          {hiddenModes.join(' · ')} hidden: {MIN_VERSION_HINT}
        </p>
      ) : null}

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
            description="databases are defined in the worker's configuration (databases: { name: { url } }) and appear here as soon as one connects."
            action={{
              label: 'open configuration',
              onClick: openConfiguration,
            }}
          />
        </div>
      ) : (
        <div className={`db-body${asideOpen ? '' : ' aside-collapsed'}`}>
          {!asideOpen ? (
            <aside className="db-aside collapsed">
              <button
                type="button"
                className="db-aside-reopen"
                onClick={toggleAside}
                aria-label="show the table list"
                title="show the table list"
              >
                <ChevronRight size={12} aria-hidden />
              </button>
            </aside>
          ) : (
            <aside className={`db-aside${passiveAside ? ' passive' : ''}`}>
              <div className="db-aside-head">
                <span>
                  tables
                  {tableCount > 0 ? ` · ${tableCount}` : ''}
                </span>
                <button
                  type="button"
                  className="db-aside-toggle"
                  onClick={toggleAside}
                  aria-label="hide the table list"
                  title="hide the table list"
                >
                  <ChevronLeft size={12} aria-hidden />
                </button>
              </div>
              {passiveAside ? (
                <p className="db-aside-note">
                  {mode === 'health' ? 'selection is not used on health' : 'selection only seeds the sql starter'}
                </p>
              ) : null}
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
          )}
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
              <>
                {/* Every panel that accumulates user work stays mounted and
                    hides (`.db-keep`) instead of unmounting: a peek at another
                    tab must not destroy a sql draft, the data tab's filters,
                    the changes feed, or a hand-arranged diagram. Health alone
                    remounts — it is a fresh-read tab, and kept mounted its
                    auto-refresh would keep polling while hidden. */}
                <div className="db-keep" hidden={mode !== 'sql'}>
                  {/* Keyed by db only: a refresh reloads metadata without
                      wiping the draft. After a write commits, the page
                      refreshes so the table list, completions and starter SQL
                      can't disconfirm what just happened. */}
                  <SqlPanel
                    key={activeDb.name}
                    host={host}
                    db={activeDb.name}
                    seedSql={seedSql}
                    tables={tables.map((t) => t.name)}
                    starterSql={starterSql}
                    onWrite={refresh}
                  />
                </div>
                <div className="db-keep" hidden={mode !== 'data'}>
                  {!selectedTable ? (
                    <div className="db-pad">
                      <EmptyState
                        icon={TableIcon}
                        title="select a table"
                        description={
                          tableCount > 0
                            ? `${tableCount} table${tableCount === 1 ? '' : 's'}${
                                viewCount > 0 ? ` and ${viewCount} view${viewCount === 1 ? '' : 's'}` : ''
                              } in ${activeDb.name} — pick one to browse its rows, sort columns, and inspect values.`
                            : `no tables in ${activeDb.name} yet`
                        }
                      />
                    </div>
                  ) : (
                    // Keyed by db/table/arrival — navigation restarts the
                    // panel clean. A header refresh re-reads through
                    // `refreshToken` instead of a key bump, so it can no
                    // longer wipe half-built filters.
                    <TableDataPanel
                      key={`${activeDb.name}:${selectedTable}:${arrivalKey}`}
                      host={host}
                      db={activeDb.name}
                      driver={activeDb.driver}
                      table={selectedTable}
                      initialFilters={arrival}
                      onFollow={follow}
                      enabled
                      refreshToken={bump}
                      onOpenInSql={(table) => {
                        setSeedSql(`select * from ${quoteTableRef(activeDb.driver, table)} limit ${PAGE_SIZE}`)
                        setMode('sql')
                      }}
                    />
                  )}
                </div>
                {modes.includes('diagram') ? (
                  <div className="db-keep" hidden={mode !== 'diagram'}>
                    <ErdPanel
                      // Keyed by the selected table so switching in the tree
                      // opens the diagram around it. A mode switch no longer
                      // remounts, so zoom and dragged nodes survive a peek at
                      // the rows.
                      key={`${activeDb.name}:${bump}:${selectedTable ?? ''}`}
                      host={host}
                      db={activeDb.name}
                      focusTable={selectedTable}
                    />
                  </div>
                ) : null}
                {modes.includes('changes') ? (
                  <div className="db-keep" hidden={mode !== 'changes'}>
                    {/* Keyed by table: the feed is per-binding. Kept mounted,
                        it keeps listening while you look at the rows it is
                        telling you about. */}
                    <ChangesPanel
                      key={`${activeDb.name}:${selectedTable ?? ''}`}
                      host={host}
                      db={activeDb.name}
                      table={selectedTable}
                      kind={tables.find((t) => t.name === selectedTable)?.kind}
                      onRefresh={refresh}
                    />
                  </div>
                ) : null}
                {mode === 'health' ? (
                  <HealthPanel key={activeDb.name} host={host} db={activeDb.name} refreshToken={bump} />
                ) : null}
              </>
            ) : null}
          </div>
        </div>
      )}
    </div>
  )
}
