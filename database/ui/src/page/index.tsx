/**
 * The database page (#/ext/database): the database worker's console surface —
 * configured databases, their schema, paged and sortable table contents with a
 * row inspector, an ad-hoc SQL panel, a schema diagram, connection health, and
 * the writes landing in the selected table as they commit.
 *
 * Chrome is the shared page shell (PageShell/PageHeader from
 * @iii-dev/console-ui): identity and database-level controls in the header, a
 * segmented panel switcher under it, then the schema tree as the fixed
 * navigation column beside the active panel. Layout adapts to the width the
 * page HAS (a ResizeObserver on its own root, not a viewport media query — the
 * console hosts pages in panes of any size): under NARROW_BELOW px it becomes
 * a drill-in flow — the tree fills the pane, opening a table (or picking a
 * panel) swaps to the workspace with a ← back button. Panels stay MOUNTED
 * through all of it, so drilling out never destroys a sql draft, the data
 * tab's filters, the changes feed, or a hand-arranged diagram.
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

import {
  Badge,
  Button,
  EmptyState,
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  Select,
  Skeleton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { type ComponentType, useCallback, useEffect, useMemo, useRef, useState } from 'react'
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

/** Container width (px) below which the page collapses to the drill-in
 * tree ⇄ panel flow. */
const NARROW_BELOW = 850

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

/** Observe the page root's own width. Returns a callback ref to put on the
 * root plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none) are ignored so a hidden page keeps its last
 * real layout. */
function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}

/** ← back affordance for the drill-in flow. */
function BackButton({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button type="button" className="db-ui-back" onClick={onClick} aria-label={label} title={label}>
      <ChevronLeft size={16} aria-hidden />
    </button>
  )
}

export function DatabasePage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
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

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  // Narrow drill-in: false shows the tree, true the active panel. Wide mode
  // shows both and ignores it.
  const [drilled, setDrilled] = useState(false)

  // The SQL panel reports whether it holds a statement worth guarding — a
  // database switch remounts it, and an unsaved statement must not vanish
  // silently.
  const sqlDirtyRef = useRef(false)
  const onSqlDirtyChange = useCallback((dirty: boolean) => {
    sqlDirtyRef.current = dirty
  }, [])

  // At dock widths the table rail costs every tab its first screenful, and
  // two of the five modes barely use it — so it collapses, and stays where
  // you left it (per workspace tab; narrow mode ignores it — the drill-in
  // tree IS the pane there).
  const asideKey = `database-ui:${tabId || 'page'}:aside`
  const [asideOpen, setAsideOpen] = useState(() => {
    try {
      return window.localStorage.getItem(asideKey) !== 'closed'
    } catch {
      return true
    }
  })

  const toggleAside = () => {
    setAsideOpen((open) => {
      try {
        window.localStorage.setItem(asideKey, open ? 'closed' : 'open')
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

  const pickMode = (m: PanelMode) => {
    setMode(m)
    // Picking a panel is an intent to look at it — in the drill-in flow that
    // means leaving the tree.
    if (narrow) setDrilled(true)
  }

  // A configured database without a `url` (redacted in the worker's config
  // response) still counts as configured — fall back to its name, not to the
  // "nothing here" copy.
  const subtitle = activeDb
    ? (activeDb.url ?? activeDb.name)
    : dbsRead.error
      ? 'worker not connected'
      : 'no database configured'

  // Narrow: one pane at a time. Wide: both, with an optionally collapsed rail.
  const showSide = !narrow || !drilled
  const showMain = !narrow || drilled
  const sideCollapsed = !narrow && !asideOpen

  return (
    <PageShell className="db-ui-shell">
      <PageHeader
        icon={<Database size={16} />}
        title="database"
        description={subtitle}
        actions={
          <>
            {activeDb ? <Badge variant="accent">{activeDb.driver}</Badge> : null}
            {activeDb && dbs.length > 1 ? (
              <Select
                value={activeDb.name}
                options={dbs.map((db) => ({
                  value: db.name,
                  label: db.name,
                  title: db.url,
                }))}
                onChange={(next) => {
                  if (next === activeDb.name) return
                  // The SQL panel is per-database (remounted on switch), so an
                  // unsaved statement would vanish — ask first.
                  if (sqlDirtyRef.current && !window.confirm('Discard the unsaved SQL statement?')) return
                  setSelectedDb(next)
                  setSelectedTable(null)
                }}
                aria-label="database"
              />
            ) : null}
            <Button variant="ghost" size="sm" onClick={refresh}>
              <RefreshCw size={14} aria-hidden />
              refresh
            </Button>
            <Button variant="ghost" size="sm" onClick={openConfiguration}>
              <Settings size={14} aria-hidden />
              configure
            </Button>
          </>
        }
        onClose={onRequestClose}
      />

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

      {dbsRead.error ? (
        <div className="db-ui-state">
          <StatusPanel
            variant="alert"
            icon={<AlertCircle size={18} />}
            headline="database worker unavailable"
            detail={dbsRead.error}
          />
        </div>
      ) : dbsRead.loading && dbs.length === 0 ? (
        <div className="db-ui-state">
          <div className="db-msg db-pulse">· loading databases…</div>
        </div>
      ) : dbs.length === 0 ? (
        <div className="db-ui-state">
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
        <div className={`db-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`} ref={rootRef}>
          <div className="db-ui-modesbar">
            {/* biome-ignore lint/a11y/useSemanticElements: segmented control of buttons; fieldset chrome (min-content sizing) breaks the bar row */}
            <div className="db-ui-seg" role="group" aria-label="panel">
              {modes.map((m) => (
                <button
                  key={m}
                  type="button"
                  className={`db-ui-seg-btn${mode === m ? ' active' : ''}`}
                  aria-pressed={mode === m}
                  onClick={() => pickMode(m)}
                >
                  {m}
                </button>
              ))}
            </div>
            <span className="db-ui-modesbar-spacer" />
            {hiddenModes.length > 0 ? (
              <span className="db-ui-modes-hint">
                {hiddenModes.join(' · ')} hidden: {MIN_VERSION_HINT}
              </span>
            ) : null}
          </div>

          <div className="db-ui-body">
            {/* Both panes stay in the tree and hide with [hidden] — narrow
                drill-in must not unmount the workspace (sql drafts, filters,
                the diagram's hand-arranged nodes all live there). */}
            {sideCollapsed ? (
              <aside className="db-ui-side collapsed">
                <button
                  type="button"
                  className="db-ui-iconbtn"
                  onClick={toggleAside}
                  aria-label="show the table list"
                  title="show the table list"
                >
                  <ChevronRight size={12} aria-hidden />
                </button>
              </aside>
            ) : (
              <aside className={`db-ui-side${passiveAside ? ' passive' : ''}`} hidden={!showSide} aria-label="table list">
                <header className="db-ui-side-head">
                  <span className="label">tables</span>
                  {tableCount > 0 ? <span className="count">{tableCount}</span> : null}
                  <span className="spacer" />
                  {!narrow ? (
                    <button
                      type="button"
                      className="db-ui-iconbtn"
                      onClick={toggleAside}
                      aria-label="hide the table list"
                      title="hide the table list"
                    >
                      <ChevronLeft size={12} aria-hidden />
                    </button>
                  ) : null}
                </header>
                {passiveAside ? (
                  <p className="db-ui-side-note">
                    {mode === 'health' ? 'selection is not used on health' : 'selection only seeds the sql starter'}
                  </p>
                ) : null}
                <div className="db-ui-side-scroll">
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
                        if (narrow) {
                          // Opening a table shows its rows; sql/health don't
                          // render the selection, so drilling into them
                          // would look like nothing happened.
                          if (mode === 'sql' || mode === 'health') setMode('data')
                          setDrilled(true)
                        }
                      }}
                    />
                  ) : null}
                </div>
              </aside>
            )}

            <section className="db-ui-main" hidden={!showMain} aria-label="database workspace">
              {narrow ? (
                <div className="db-ui-backbar">
                  <BackButton onClick={() => setDrilled(false)} label="back to the table list" />
                  <span className="title">{mode === 'data' && selectedTable ? selectedTable : mode}</span>
                </div>
              ) : null}
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
                      onDirtyChange={onSqlDirtyChange}
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
                  {/* Health also unmounts when the drill-in flow hides the
                      workspace — its auto-refresh must not poll a pane the
                      user drilled out of. */}
                  {mode === 'health' && showMain ? (
                    <HealthPanel key={activeDb.name} host={host} db={activeDb.name} refreshToken={bump} />
                  ) : null}
                </>
              ) : null}
            </section>
          </div>
        </div>
      )}
    </PageShell>
  )
}
