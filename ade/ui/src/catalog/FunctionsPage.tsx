/**
 * The Functions page (page `functions`): a navigation sidebar of every
 * function on the bus — each row led by the `ƒ` tile, grouped by the worker
 * that registered it — and a workspace that is always present: an overview
 * of the bus when nothing is selected, the function document (toolbar,
 * masthead, contract/run/observers/calls tabs) when one is.
 *
 * The document opens on the contract: input schema, output schema,
 * metadata, and how to call it. The masthead carries every identity fact
 * once (worker, runtime, observers, last call), so no side rail repeats
 * them.
 *
 * Live, never polled. The engine calls `engine::on-functions-changed` when
 * the function set changes and `engine::on-workers-changed` when a worker
 * connects or dies, so either is visible here without a manual refresh
 * (within ~100ms), and rows that arrived on the last tick flash once so the
 * change is legible rather than silent.
 *
 * `engine::functions::list` is the catalogue (one cheap row per function);
 * `engine::functions::info` is fetched per selection, because that is where
 * the schemas live and the fleet has hundreds of functions.
 * `engine::workers::list` rides along for each worker's runtime.
 *
 * Internal functions are hidden by default: the console's own per-tab
 * handlers and every worker's UI plumbing register as internal, and they
 * would otherwise outnumber the functions an operator came to find.
 */

import {
  Button,
  EmptyState,
  type Host,
  IconButton,
  JsonHighlight,
  type PageCommandsApi,
  PageHeader,
  SearchField,
  ShadcnTabs,
  ShadcnTabsContent,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  StatusDot,
} from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Braces, Check, CircleAlert, Copy, MessageSquare, Play, RefreshCw, SquareFunction } from 'lucide-react'
import {
  Fragment,
  type MutableRefObject,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { ActivityFeed, agoLabel, formatDuration } from './ActivityFeed'
import { asCliCommand, cliArgs } from './cli-command'
import { nextCronRun, untilLabel } from './cron'
import {
  type FunctionDetail,
  type FunctionSummary,
  functionInfo,
  isEventSource,
  listFunctions,
  listObservers,
  listWorkers,
  type ObserverInfo,
  type SpanEvent,
  useLiveSignals,
  useResource,
} from './engine'
import { InvokePanel } from './InvokePanel'
import { LastCallMeta, useLiveActivity } from './live'
import { familyName } from './names'
import { CRON_FN, cronExpression, familyOf, summarize } from './observer-kinds'
import { SchemaTable } from './SchemaTable'
import { pretty, templateFromSchema } from './schema'
import {
  CatalogListSkeleton,
  CatalogRow,
  CatalogShell,
  CodeBlock,
  CopyIconButton,
  DocHead,
  DocTile,
  ErrorNote,
  FamilyIcon,
  GroupHeader,
  IdText,
  LiveDot,
  type MenuAction,
  Note,
  SideCount,
  type Stat,
  StatStrip,
  SwitchRow,
  TerminalSnippet,
  useGroupToggle,
} from './widgets'

/** Worker groups always start expanded; there is no noisy bucket. */
const alwaysOpen = () => true

type DocTab = 'contract' | 'run' | 'observers' | 'calls'

export function FunctionsPage({
  host,
  side,
  onRequestClose,
  commands,
}: {
  host: Host
  side?: 'left' | 'right'
  onRequestClose?: () => void
  commands?: PageCommandsApi
}) {
  const [showInternal, setShowInternal] = useState(false)
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const groupState = useGroupToggle(alwaysOpen)
  const searchInputRef = useRef<HTMLInputElement>(null)
  // Set by the mounted InvokePanel (Run tab only) so Mod+Enter can reach
  // its run() without lifting the whole invoke form up to this page.
  const invokeRunRef = useRef<(() => void) | null>(null)

  // Functions and workers together: the sidebar groups by worker and the
  // document names each worker's runtime, so both loads share one beat.
  const load = useCallback(async () => {
    const [functions, workers] = await Promise.all([
      listFunctions(host, { includeInternal: showInternal }),
      listWorkers(host),
    ])
    return { functions, workers }
  }, [host, showInternal])
  const catalog = useResource(load)
  useLiveSignals(host, ['engine::on-functions-changed', 'engine::on-workers-changed'], catalog.reload)
  const activity = useLiveActivity(host)

  const functions = catalog.data?.functions ?? null

  // Ids that appeared on the last tick, so an arrival is visible instead of
  // silently changing the row count. The first load is not "new".
  const [arrived, setArrived] = useState<ReadonlySet<string>>(new Set())
  const seenRef = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!functions) return
    const ids = new Set(functions.map((f) => f.function_id))
    const previous = seenRef.current
    seenRef.current = ids
    if (!previous) return
    const fresh = new Set([...ids].filter((id) => !previous.has(id)))
    if (fresh.size === 0) return
    setArrived(fresh)
    const timer = window.setTimeout(() => setArrived(new Set()), 2000)
    return () => window.clearTimeout(timer)
  }, [functions])

  const runtimeOf = useMemo(() => {
    const byName = new Map((catalog.data?.workers ?? []).map((w) => [w.name, w.runtime]))
    return (worker: string) => byName.get(worker) ?? undefined
  }, [catalog.data])

  // Every worker with at least one listed function, for the overview table.
  const workerCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const fn of functions ?? []) {
      counts.set(fn.worker_name, (counts.get(fn.worker_name) ?? 0) + 1)
    }
    return [...counts.entries()].map(([name, count]) => ({ name, count })).sort((a, b) => a.name.localeCompare(b.name))
  }, [functions])

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    // Ids and workers only. Description text matches surprised more than
    // they helped: searching `config` surfaced harness::observers::list
    // because its description mentions config, which reads as broken.
    const matched = (functions ?? []).filter((fn) => {
      if (!needle) return true
      return fn.function_id.toLowerCase().includes(needle) || fn.worker_name.toLowerCase().includes(needle)
    })
    const byWorker = new Map<string, FunctionSummary[]>()
    for (const fn of matched) {
      const bucket = byWorker.get(fn.worker_name)
      if (bucket) bucket.push(fn)
      else byWorker.set(fn.worker_name, [fn])
    }
    return [...byWorker.entries()]
      .map(([label, items]) => ({
        label,
        items: items.sort((a, b) => a.function_id.localeCompare(b.function_id)),
      }))
      .sort((a, b) => a.label.localeCompare(b.label))
  }, [functions, search])

  const total = functions?.length ?? 0
  const shown = groups.reduce((n, g) => n + g.items.length, 0)

  // A live catalogue can remove the selected row while its document is open.
  // Returning to the list is less surprising than leaving a stale document
  // on screen for a function that no longer exists.
  useEffect(() => {
    if (selected && catalog.data && !catalog.data.functions.some((fn) => fn.function_id === selected)) {
      setSelected(null)
    }
  }, [catalog.data, selected])

  // The page's primary verbs, for the palette and the keyboard while this
  // pane has focus.
  useEffect(
    () =>
      commands?.register([
        {
          id: 'search',
          title: 'Search functions',
          detail: 'Focus the function search field',
          keywords: ['find', 'filter'],
          run: () => searchInputRef.current?.focus(),
        },
        {
          id: 'toggle-internal',
          title: 'Toggle internal functions',
          detail: 'Show or hide ADE and worker plumbing functions',
          keywords: ['internal', 'plumbing', 'hidden'],
          run: () => setShowInternal((v) => !v),
        },
        {
          id: 'refresh',
          title: 'Refresh functions',
          detail: 'Reload the function catalog',
          keywords: ['reload'],
          run: () => catalog.reload(),
        },
        {
          id: 'run-function',
          title: 'Run function',
          detail: 'Call the selected function with its current payload',
          keywords: ['invoke', 'call', 'run'],
          shortcut: 'Mod+Enter',
          firesWhileTyping: true,
          enabled: () => invokeRunRef.current !== null,
          run: () => invokeRunRef.current?.(),
        },
      ]),
    [commands, catalog.reload],
  )

  const workerTotal = groups.length
  const countLine =
    catalog.data === null
      ? 'Loading functions…'
      : search.trim()
        ? `Showing ${shown} of ${total} functions`
        : `${total} function${total === 1 ? '' : 's'} · ${workerTotal} worker${workerTotal === 1 ? '' : 's'}`

  return (
    <CatalogShell
      side={side}
      label="Functions"
      hasSelection={selected !== null}
      header={
        <PageHeader
          icon={<SquareFunction />}
          title="Functions"
          description={<span className="console-catalog-header-desc">Every function on the bus, by worker</span>}
          onClose={onRequestClose}
          className="console-catalog-page-header"
          actions={
            <>
              <LiveDot />
              <IconButton
                type="button"
                label={catalog.loading ? 'Refreshing…' : 'Refresh'}
                onClick={catalog.reload}
                disabled={catalog.loading}
              >
                <RefreshCw />
              </IconButton>
            </>
          }
        />
      }
      search={
        <SearchField
          ref={searchInputRef}
          name="catalog-search"
          className="console-catalog-search"
          value={search}
          onChange={setSearch}
          placeholder="Search functions or workers"
          aria-label="Search functions or workers"
        />
      }
      filters={<SwitchRow label="Include internal" checked={showInternal} onChange={setShowInternal} />}
      sideFooter={<SideCount>{countLine}</SideCount>}
      list={
        catalog.error ? (
          <ErrorNote
            title="Couldn't load functions"
            call="engine::functions::list"
            message={catalog.error}
            onRetry={catalog.reload}
          />
        ) : catalog.data === null ? (
          <CatalogListSkeleton label="Loading functions" />
        ) : shown === 0 ? (
          <EmptyState
            title={search.trim() ? 'Nothing matches' : 'No functions registered'}
            description={
              search.trim()
                ? 'No function id or worker name contains that text.'
                : 'Workers register their functions on connect. Start one and it appears here live.'
            }
            action={search.trim() ? { label: 'Clear search', onClick: () => setSearch('') } : undefined}
          />
        ) : (
          groups.map((group) => (
            <section key={group.label} className="console-catalog-section" aria-label={group.label}>
              <GroupHeader
                label={group.label}
                meta={runtimeOf(group.label)}
                count={group.items.length}
                open={groupState.isOpen(group.label)}
                onToggle={() => groupState.toggle(group.label)}
              />
              {!groupState.isOpen(group.label)
                ? null
                : group.items.map((fn) => (
                    <CatalogRow
                      key={fn.function_id}
                      primary={<IdText id={fn.function_id} />}
                      tooltip={fn.description ? `${fn.function_id}\n${fn.description}` : fn.function_id}
                      meta={<LastCallMeta span={activity.lastCall.get(fn.function_id)} />}
                      dot={activity.pulsing.has(fn.function_id) ? 'live' : 'idle'}
                      arrived={arrived.has(fn.function_id)}
                      selected={selected === fn.function_id}
                      onClick={() => setSelected((prev) => (prev === fn.function_id ? null : fn.function_id))}
                    />
                  ))}
            </section>
          ))
        )
      }
      main={
        selected ? (
          <FunctionDocument
            key={selected}
            host={host}
            functionId={selected}
            runtimeOf={runtimeOf}
            lastCall={activity.lastCall.get(selected)}
            onBack={() => setSelected(null)}
            onWorker={(worker) => {
              setSelected(null)
              setSearch(worker)
            }}
            invokeRunRef={invokeRunRef}
          />
        ) : (
          <FunctionsOverview
            feed={activity.feed}
            workers={workerCounts}
            runtimeOf={runtimeOf}
            listed={functions}
            onSelect={setSelected}
            onWorker={setSearch}
          />
        )
      }
    />
  )
}

/**
 * The workspace before a selection: what the bus is doing right now (the
 * live call feed every row's pulse already reads from) and which workers
 * serve the catalogue. Both are entry points into the sidebar.
 */
function FunctionsOverview({
  feed,
  workers,
  runtimeOf,
  listed,
  onSelect,
  onWorker,
}: {
  feed: readonly SpanEvent[]
  workers: readonly { name: string; count: number }[]
  runtimeOf: (worker: string) => string | undefined
  listed: readonly FunctionSummary[] | null
  onSelect: (functionId: string) => void
  onWorker: (worker: string) => void
}) {
  const known = useMemo(() => new Set((listed ?? []).map((fn) => fn.function_id)), [listed])
  const now = Date.now()
  return (
    <div className="console-catalog-doc console-fn-overview">
      <div className="console-catalog-column">
        <header className="console-fn-overview-head">
          <h2>Overview</h2>
          <p>
            Pick a function to read its contract, run it, or follow its calls. What the bus is doing right now is below.
          </p>
        </header>
        <StatStrip
          items={[
            { label: 'Functions', value: String(listed?.length ?? 0), mono: true },
            { label: 'Workers', value: String(workers.length), mono: true },
            { label: 'Last call', value: feed[0] ? agoLabel(feed[0].atMs, now) : 'None yet' },
            { label: 'Recent failures', value: String(feed.filter((span) => !span.ok).length), mono: true },
          ]}
        />
        <div className="console-fn-overview-grid">
          <section className="console-fn-section" aria-label="Recent calls">
            <div className="console-fn-section-head">
              <h3>Recent calls</h3>
              <span className="meta">Live, across every worker</span>
            </div>
            {feed.length === 0 ? (
              <div className="console-fn-quiet">
                No calls since this page opened. They show here as the engine records them.
              </div>
            ) : (
              <ul className="console-fn-list">
                {feed.map((span) => (
                  <li key={`${span.functionId}@${span.atMs}`}>
                    <button
                      type="button"
                      className="console-fn-feed-row"
                      disabled={!known.has(span.functionId)}
                      onClick={() => onSelect(span.functionId)}
                    >
                      <span className="status">
                        {span.ok ? <StatusDot tone="ok" /> : <CircleAlert aria-hidden />}
                        <span className="console-catalog-sr">{span.ok ? 'Succeeded' : 'Failed'}</span>
                      </span>
                      <span className="fn">{span.functionId}</span>
                      <span className="worker">{span.worker}</span>
                      <span className="duration">
                        {span.durationMs > 0 ? formatDuration(span.durationMs) : 'running'}
                      </span>
                      <span className="ago">{agoLabel(span.atMs, now)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <section className="console-fn-section" aria-label="Workers">
            <div className="console-fn-section-head">
              <h3>Workers</h3>
              <span className="meta">{workers.length} with functions</span>
            </div>
            <ul className="console-fn-list">
              {workers.map((worker) => (
                <li key={worker.name}>
                  <button
                    type="button"
                    className="console-fn-worker-row"
                    title={`Show only ${worker.name}'s functions`}
                    onClick={() => onWorker(worker.name)}
                  >
                    <span className="name">{worker.name}</span>
                    <span className="runtime">{runtimeOf(worker.name) ?? '—'}</span>
                    <span className="count">{worker.count}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        </div>
      </div>
    </div>
  )
}

/** `a::b::c` with a break opportunity after every `::`, never mid-word. */
function breakable(id: string): ReactNode {
  const parts = id.split('::')
  return parts.map((part, i) => (
    <Fragment key={`${i}-${part}`}>
      {part}
      {i < parts.length - 1 ? (
        <>
          ::
          <wbr />
        </>
      ) : null}
    </Fragment>
  ))
}

function FunctionDocument({
  host,
  functionId,
  runtimeOf,
  lastCall,
  onBack,
  onWorker,
  invokeRunRef,
}: {
  host: Host
  functionId: string
  /** Worker name → runtime, from the page-level workers list. */
  runtimeOf: (worker: string) => string | undefined
  lastCall?: SpanEvent
  onBack: () => void
  /** Leave the document for the list, filtered to one worker. */
  onWorker: (worker: string) => void
  invokeRunRef: MutableRefObject<(() => void) | null>
}) {
  const load = useCallback(() => functionInfo(host, functionId), [host, functionId])
  const detail = useResource(load)
  // Observers of this function, and observers whose target it is: one read,
  // split here, live on observer state changes.
  const loadObservers = useCallback(() => listObservers(host), [host])
  const observers = useResource(loadObservers)
  useLiveSignals(host, ['engine::on-observers-changed'], observers.reload)
  const related = useMemo(() => {
    const all = observers.data ?? []
    return {
      of: all.filter((o) => o.function_id === functionId),
      into: all.filter((o) => o.target.function_id === functionId),
    }
  }, [observers.data, functionId])
  // Selecting a function opens on its contract, not on the editor: the first
  // question an operator has is what the function takes and returns, and a
  // JSON box answers neither. Run is one click away in the masthead.
  const [tab, setTab] = useState<DocTab>('contract')
  const [prefill, setPrefill] = useState<{ value: unknown; nonce: number }>()

  useEffect(() => {
    setTab('contract')
    setPrefill(undefined)
  }, [functionId])

  // Replaying from the calls tab hands the recorded input to the Run
  // editor and moves the operator there — the whole point of the button.
  const replay = useCallback((value: unknown) => {
    setPrefill({ value, nonce: Date.now() })
    setTab('run')
  }, [])

  const data = detail.data
  const worker = data?.worker_name
  const language = worker ? runtimeOf(worker) : undefined
  const observerCount = related.of.length + related.into.length
  const compose = host.chat?.compose
  const { state: copyState, copy: copyId } = useCopyFlash(functionId, 2000)
  const copied = copyState === 'copied'

  const menu: (MenuAction | 'separator')[] = [
    {
      label: copied ? 'Copied' : 'Copy function id',
      icon: copied ? <Check aria-hidden /> : <Copy aria-hidden />,
      onSelect: copyId,
      keepOpen: true,
    },
  ]
  if (compose) {
    menu.push({
      label: 'Reference in chat',
      icon: <MessageSquare aria-hidden />,
      onSelect: () => compose({ text: `\`${functionId}\` ` }),
    })
  }

  return (
    <div className="console-catalog-doc console-fn-doc">
      <DocHead
        tile={<DocTile />}
        eyebrow="Function"
        title={breakable(functionId)}
        description={data ? data.description || 'No description provided.' : undefined}
        primary={
          data && !data.private && tab !== 'run' ? (
            <Button
              type="button"
              variant="primary"
              size="sm"
              className="console-catalog-btn"
              onClick={() => setTab('run')}
            >
              <Play aria-hidden />
              Run
            </Button>
          ) : null
        }
        menu={menu}
        menuLabel="More function actions"
        onBack={onBack}
        backLabel="Back to functions"
        stats={data ? functionStats({ detail: data, related, language, lastCall, onWorker }) : undefined}
      />

      {detail.error ? (
        <div className="console-catalog-column">
          <ErrorNote
            title="Couldn't load function details"
            call="engine::functions::info"
            message={detail.error}
            onRetry={detail.reload}
          />
        </div>
      ) : data === null ? (
        <div className="console-catalog-column">
          <Note>Loading details…</Note>
        </div>
      ) : (
        <ShadcnTabs value={tab} onValueChange={(value) => setTab(value as DocTab)} className="console-catalog-tabs">
          <ShadcnTabsList variant="line" activateOnFocus className="console-catalog-tablist">
            <ShadcnTabsTrigger value="contract">Contract</ShadcnTabsTrigger>
            <ShadcnTabsTrigger value="run">Run</ShadcnTabsTrigger>
            <ShadcnTabsTrigger value="observers">
              Observers
              {observerCount > 0 ? <span className="tab-count">{observerCount}</span> : null}
            </ShadcnTabsTrigger>
            <ShadcnTabsTrigger value="calls">Calls</ShadcnTabsTrigger>
          </ShadcnTabsList>
          <ShadcnTabsContent value="contract" className="console-catalog-panel">
            <FunctionContract detail={data} onEdit={() => setTab('run')} />
          </ShadcnTabsContent>
          <ShadcnTabsContent value="run" className="console-catalog-panel">
            {data.private ? (
              <Note>
                This function is private: only the worker that hosts it calls it, with each event. Observe it to receive
                its calls, or run one of its observers' targets from the Observers tab.
              </Note>
            ) : (
              <InvokePanel
                host={host}
                functionId={functionId}
                requestSchema={data.request_schema}
                prefill={prefill}
                label="Run"
                runningLabel="Running…"
                runRef={invokeRunRef}
              />
            )}
          </ShadcnTabsContent>
          <ShadcnTabsContent value="observers" className="console-catalog-panel">
            {observers.error ? (
              <ErrorNote
                title="Couldn't load observers"
                call="engine::observers::list"
                message={observers.error}
                onRetry={observers.reload}
              />
            ) : (
              <FunctionObservers detail={data} related={related} />
            )}
          </ShadcnTabsContent>
          <ShadcnTabsContent value="calls" className="console-catalog-panel">
            <ActivityFeed host={host} functionId={functionId} onReplay={replay} />
          </ShadcnTabsContent>
        </ShadcnTabs>
      )}
    </div>
  )
}

interface RelatedObservers {
  /** Observers of this function. */
  of: ObserverInfo[]
  /** Observers whose target is this function: what makes it run. */
  into: ObserverInfo[]
}

/**
 * Every identity fact once, in one line: who serves the function, in what
 * runtime, what it is observed by, what makes it run, and when it last ran.
 */
function functionStats({
  detail,
  related,
  language,
  lastCall,
  onWorker,
}: {
  detail: FunctionDetail
  related: RelatedObservers
  language?: string
  lastCall?: SpanEvent
  onWorker: (worker: string) => void
}): Stat[] {
  const now = new Date()
  const bySource = new Map<string, number>()
  for (const observer of related.into) {
    bySource.set(observer.function_id, (bySource.get(observer.function_id) ?? 0) + 1)
  }
  const nextRun = related.into
    .filter((observer) => observer.function_id === CRON_FN)
    .map(cronExpression)
    .map((expression) => (expression ? nextCronRun(expression, now) : null))
    .filter((d): d is Date => d !== null)
    .sort((a, b) => a.getTime() - b.getTime())[0]

  const items: Stat[] = [
    {
      label: 'Worker',
      mono: true,
      value: (
        <button
          type="button"
          className="console-catalog-linkish"
          title={`Show only ${detail.worker_name}'s functions`}
          onClick={() => onWorker(detail.worker_name)}
        >
          {detail.worker_name}
        </button>
      ),
    },
  ]
  if (language) items.push({ label: 'Runtime', value: language, mono: true })
  items.push({
    label: 'Runs on',
    value:
      bySource.size === 0 ? 'Calls only' : [...bySource.entries()].map(([source, n]) => `${n} ${source}`).join(', '),
  })
  if (isEventSource(detail) || related.of.length > 0) {
    items.push({
      label: 'Observers',
      value: `${related.of.length}${detail.private ? ' · private source' : ''}`,
    })
  }
  if (nextRun) {
    items.push({ label: 'Next run', value: untilLabel(nextRun, now), mono: true })
  }
  items.push({
    label: 'Last call',
    value: lastCall ? (
      <span className="last-call">
        <StatusDot tone={lastCall.ok ? 'ok' : 'alert'} />
        {lastCall.ok ? '' : 'Failed · '}
        {agoLabel(lastCall.atMs, Date.now())} ·{' '}
        {lastCall.durationMs > 0 ? formatDuration(lastCall.durationMs) : 'running'}
      </span>
    ) : (
      'None yet'
    ),
  })
  return items
}

/**
 * The function's contract, which is what the operator came to read: the input
 * fields a caller must supply, then the shape that comes back, then whatever
 * metadata the worker attached — and, beside it on a wide pane, the call
 * itself as a payload and as a terminal line.
 */
function FunctionContract({ detail, onEdit }: { detail: FunctionDetail; onEdit: () => void }) {
  const hasMetadata =
    detail.metadata !== undefined &&
    detail.metadata !== null &&
    (typeof detail.metadata !== 'object' || Array.isArray(detail.metadata) || Object.keys(detail.metadata).length > 0)

  const template = templateFromSchema(detail.request_schema)
  const payload = parseOr(template, {})
  const command = asCliCommand(detail.function_id, payload)

  return (
    <div className="console-fn-contract">
      <div className="contract-main">
        <SchemaTable label="Input" schema={detail.request_schema} empty="This function registered no input schema." />
        <SchemaTable
          label="Output"
          schema={detail.response_schema}
          empty="This function registered no output schema."
        />
        {hasMetadata ? (
          <div className="console-catalog-schema">
            <div className="console-catalog-schema-section">
              <h3 className="label">Metadata</h3>
            </div>
            <JsonHighlight code={pretty(detail.metadata)} className="console-catalog-json" wrap />
          </div>
        ) : null}
      </div>
      <aside className="console-fn-callit" aria-label="Call this function">
        <header className="callit-head">
          <h3 className="callit-title">Call it</h3>
          <Button type="button" variant="ghost" size="sm" className="console-catalog-btn" onClick={onEdit}>
            <Play aria-hidden />
            Open in Run
          </Button>
        </header>
        <CodeBlock label="json" icon={<Braces aria-hidden />} copy={template} copyLabel="Copy payload">
          <JsonHighlight code={template} className="snippet-json" />
        </CodeBlock>
        <TerminalSnippet functionId={detail.function_id} args={cliArgs(payload)} copy={command} />
      </aside>
    </div>
  )
}

function parseOr(text: string, fallback: unknown): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return fallback
  }
}

/**
 * The observers around this function, one card each: those whose target it
 * is (what makes it run), then those that observe it. Each card names the
 * observer in its family's words (`GET /users/:id`, `every 5 min`) and keeps
 * the raw args and filters for the cases those words compress away.
 */
function FunctionObservers({ detail, related }: { detail: FunctionDetail; related: RelatedObservers }) {
  if (related.of.length === 0 && related.into.length === 0) {
    return (
      <Note>
        {detail.private
          ? 'Nothing observes this function yet, and nothing calls it through an observer.'
          : 'Nothing observes this function, and no observer calls it. It runs only when something calls it.'}
      </Note>
    )
  }
  return (
    <div className="console-catalog-obscards-groups">
      {related.into.length > 0 ? (
        <section className="console-catalog-obsgroup">
          <h3>Runs on</h3>
          <ObserverCards observers={related.into} side="source" />
        </section>
      ) : null}
      {related.of.length > 0 ? (
        <section className="console-catalog-obsgroup">
          <h3>Observed by</h3>
          <ObserverCards observers={related.of} side="target" />
        </section>
      ) : null}
    </div>
  )
}

function ObserverCards({
  observers,
  side,
}: {
  observers: ObserverInfo[]
  /** Which end of each observer the card names beside its summary. */
  side: 'source' | 'target'
}) {
  const now = new Date()
  return (
    <ul className="console-catalog-obscards">
      {observers.map((observer) => {
        const spec = familyOf(observer.function_id)
        const expression = cronExpression(observer)
        const next = expression ? nextCronRun(expression, now) : null
        const args = pretty(observer.args)
        const filters = pretty(observer.filters)
        return (
          <li key={observer.id} className="console-catalog-obscard">
            <FamilyIcon family={spec.family} tone={spec.tone} />
            <div className="copy">
              <div className="line1">
                <span className="name">{summarize(observer)}</span>
                <span className="tag">{familyName(spec)}</span>
              </div>
              <span className="type">
                {side === 'source' ? observer.function_id : `→ ${observer.target.function_id}`}
                {observer.state === 'active' ? '' : ` · ${observer.state}`}
              </span>
              {next ? <span className="fine">Next run {untilLabel(next, now)}</span> : null}
              {args !== '{}' ? <JsonHighlight code={args} className="console-catalog-json" wrap /> : null}
              {filters !== '{}' ? <JsonHighlight code={filters} className="console-catalog-json" wrap /> : null}
            </div>
            <CopyIconButton value={observer.id} label="Copy observer id" />
          </li>
        )
      })}
    </ul>
  )
}
