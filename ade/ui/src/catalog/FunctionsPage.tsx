/**
 * The Functions page (page `functions`): a navigation sidebar of every
 * function on the bus — each row led by the `ƒ` tile, grouped by the worker
 * that registered it — and a workspace that is always present: an overview
 * of the bus when nothing is selected, the function document (toolbar,
 * masthead, contract/run/bindings/calls tabs) when one is.
 *
 * The document opens on the contract: input schema, output schema,
 * metadata, and how to call it. The masthead carries every identity fact
 * once (worker, runtime, bindings, last call), so no side rail repeats them.
 *
 * Live, never polled. `engine::functions-available` fires when the function
 * set changes and `engine::workers-available` when a worker connects or
 * dies, so either is visible here without a manual refresh (within ~100ms
 * on newer engines; older ones poll on a 5s tick), and rows that arrived on
 * the last tick flash once so the change is legible rather than silent.
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
  Badge,
  Breadcrumb,
  Button,
  EmptyState,
  Eyebrow,
  type Host,
  IconButton,
  JsonHighlight,
  type PageCommandsApi,
  PageHeader,
  SearchField,
  StatusDot,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Toolbar,
  uiClasses,
} from '@iii-dev/console-ui'
import {
  Activity,
  ArrowLeft,
  Braces,
  MessageSquare,
  Play,
  RefreshCw,
  SquareFunction,
  Zap,
} from 'lucide-react'
import {
  Fragment,
  type MutableRefObject,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useId,
  useRef,
  useState,
} from 'react'
import { ActivityFeed, agoLabel, formatDuration } from './ActivityFeed'
import { nextCronRun, untilLabel } from './cron'
import {
  type FunctionDetail,
  type FunctionSummary,
  functionInfo,
  listFunctions,
  listWorkers,
  type RegisteredTrigger,
  type SpanEvent,
  useLiveSignals,
  useResource,
} from './engine'
import { asCliCommand, InvokePanel } from './InvokePanel'
import { LastCallMeta, useLiveActivity } from './live'
import { SchemaTable } from './SchemaTable'
import { pretty, templateFromSchema } from './schema'
import { cronExpression, familyOf, summarize } from './trigger-kinds'
import {
  CatalogListSkeleton,
  CatalogRow,
  CatalogShell,
  CopyButton,
  CopyIconButton,
  ErrorNote,
  FamilyGlyph,
  FnGlyph,
  GroupHeader,
  LiveDot,
  Note,
  SideCount,
  useGroupToggle,
} from './widgets'

/** Worker groups always start expanded; there is no noisy bucket. */
const alwaysOpen = () => true

type DocTab = 'contract' | 'run' | 'bindings' | 'calls'

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
  const internalSwitchId = useId()
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
  useLiveSignals(
    host,
    ['engine::functions-available', 'engine::workers-available'],
    catalog.reload,
  )
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
    const byName = new Map(
      (catalog.data?.workers ?? []).map((w) => [w.name, w.runtime]),
    )
    return (worker: string) => byName.get(worker) ?? undefined
  }, [catalog.data])

  // Every worker with at least one listed function, for the overview table.
  const workerCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const fn of functions ?? []) {
      counts.set(fn.worker_name, (counts.get(fn.worker_name) ?? 0) + 1)
    }
    return [...counts.entries()]
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => a.name.localeCompare(b.name))
  }, [functions])

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    // Ids and workers only. Description text matches surprised more than
    // they helped: searching `config` surfaced harness::triggers::list
    // because its description mentions config, which reads as broken.
    const matched = (functions ?? []).filter((fn) => {
      if (!needle) return true
      return (
        fn.function_id.toLowerCase().includes(needle) ||
        fn.worker_name.toLowerCase().includes(needle)
      )
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
    if (
      selected &&
      catalog.data &&
      !catalog.data.functions.some((fn) => fn.function_id === selected)
    ) {
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
          detail: 'Trigger the selected function with its current payload',
          keywords: ['invoke', 'trigger', 'call'],
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
      hasSelection={selected !== null}
      header={
        <PageHeader
          icon={<SquareFunction />}
          title="Functions"
          description={
            <span className="console-catalog-header-desc">
              Every function on the bus, by worker
            </span>
          }
          onClose={onRequestClose}
          className="console-catalog-page-header"
          actions={
            <>
              <LiveDot />
              <IconButton
                type="button"
                label={catalog.loading ? 'Refreshing…' : 'Refresh catalog'}
                onClick={catalog.reload}
                disabled={catalog.loading}
              >
                <RefreshCw />
              </IconButton>
            </>
          }
        />
      }
      sideTop={
        <>
          <SearchField
            ref={searchInputRef}
            name="catalog-search"
            className="console-catalog-search"
            value={search}
            onChange={setSearch}
            placeholder="Search functions or workers"
            aria-label="Search functions or workers"
          />
          <div className="console-catalog-switch-row">
            <label htmlFor={internalSwitchId}>Include internal</label>
            <Switch
              id={internalSwitchId}
              checked={showInternal}
              onChange={(event) => setShowInternal(event.target.checked)}
            />
          </div>
        </>
      }
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
          <CatalogListSkeleton label="loading functions" />
        ) : shown === 0 ? (
          <EmptyState
            title={
              search.trim() ? 'Nothing matches' : 'No functions registered'
            }
            description={
              search.trim()
                ? 'No function id or worker name contains that text.'
                : 'Workers register their functions on connect. Start one and it appears here live.'
            }
            action={
              search.trim()
                ? { label: 'Clear search', onClick: () => setSearch('') }
                : undefined
            }
          />
        ) : (
          groups.map((group) => (
            <div key={group.label} className="console-catalog-section">
              <GroupHeader
                label={group.label}
                meta={runtimeOf(group.label)}
                count={group.items.length}
                countLabel="function"
                open={groupState.isOpen(group.label)}
                onToggle={() => groupState.toggle(group.label)}
              />
              {!groupState.isOpen(group.label)
                ? null
                : group.items.map((fn) => (
                    <CatalogRow
                      key={fn.function_id}
                      icon={<FnGlyph />}
                      primary={fn.function_id}
                      secondary={fn.description ?? undefined}
                      meta={
                        <LastCallMeta
                          span={activity.lastCall.get(fn.function_id)}
                        />
                      }
                      selected={selected === fn.function_id}
                      flash={
                        arrived.has(fn.function_id) ||
                        activity.pulsing.has(fn.function_id)
                      }
                      onClick={() =>
                        setSelected((prev) =>
                          prev === fn.function_id ? null : fn.function_id,
                        )
                      }
                    />
                  ))}
            </div>
          ))
        )
      }
      main={
        selected ? (
          <FunctionDocument
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
  const known = useMemo(
    () => new Set((listed ?? []).map((fn) => fn.function_id)),
    [listed],
  )
  const now = Date.now()
  return (
    <div className="console-catalog-doc console-fn-overview">
      <header className="console-fn-overview-head">
        <h2>Overview</h2>
        <p>
          Pick a function to read its contract, run it, or follow its calls.
          What the bus is doing right now is below.
        </p>
      </header>
      <div className="console-fn-overview-grid">
        <section className="console-fn-section" aria-label="Recent calls">
          <div className="console-fn-section-head">
            <h3>Recent calls</h3>
            <span className="meta">Live, across every worker</span>
          </div>
          {feed.length === 0 ? (
            <Note>
              No calls since this page opened. They appear here as the engine
              records them.
            </Note>
          ) : (
            <ul className="console-fn-feed">
              {feed.map((span) => (
                <li key={`${span.functionId}@${span.atMs}`}>
                  <button
                    type="button"
                    className="console-fn-feed-row"
                    disabled={!known.has(span.functionId)}
                    onClick={() => onSelect(span.functionId)}
                  >
                    <StatusDot tone={span.ok ? 'ok' : 'alert'} />
                    <span className="fn">{span.functionId}</span>
                    <span className="worker">{span.worker}</span>
                    <span className="duration">
                      {span.durationMs > 0
                        ? formatDuration(span.durationMs)
                        : 'running'}
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
            <span className="meta">{workers.length} with listed functions</span>
          </div>
          <TableViewport>
            <TableFrame>
              <Table density="compact" aria-label="Workers">
                <TableHeader>
                  <TableRow>
                    <TableHead>Worker</TableHead>
                    <TableHead>Runtime</TableHead>
                    <TableHead className="num">Functions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {workers.map((worker) => (
                    <TableRow
                      key={worker.name}
                      interactive
                      tabIndex={0}
                      title={`Show only ${worker.name}'s functions`}
                      onClick={() => onWorker(worker.name)}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault()
                          onWorker(worker.name)
                        }
                      }}
                    >
                      <TableCell>{worker.name}</TableCell>
                      <TableCell className="mono faint">
                        {runtimeOf(worker.name) ?? '—'}
                      </TableCell>
                      <TableCell className="mono num">{worker.count}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </TableFrame>
          </TableViewport>
        </section>
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
  const load = useCallback(
    () => functionInfo(host, functionId),
    [host, functionId],
  )
  const detail = useResource(load)
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
  const bindings = data?.registered_triggers ?? []
  const compose = host.chat?.compose

  return (
    <div className="console-catalog-doc console-fn-doc">
      <Toolbar
        aria-label="Function"
        className="console-fn-toolbar"
        end={
          <>
            <CopyIconButton value={functionId} label="Copy function id" />
            {compose ? (
              <IconButton
                type="button"
                label="Reference in chat"
                onClick={() => compose({ text: `\`${functionId}\` ` })}
              >
                <MessageSquare />
              </IconButton>
            ) : null}
          </>
        }
      >
        <IconButton
          type="button"
          className="console-catalog-back"
          label="Back to functions"
          onClick={onBack}
        >
          <ArrowLeft />
        </IconButton>
        <Breadcrumb
          items={[
            { label: 'functions', onClick: onBack, key: 'root' },
            ...(worker
              ? [{ label: worker, onClick: () => onWorker(worker), key: 'w' }]
              : []),
            { label: functionId, key: 'fn' },
          ]}
        />
      </Toolbar>

      <header className="console-fn-head">
        <div className="title-row">
          <div className="title-copy">
            <h2 className="title">{breakable(functionId)}</h2>
            {data ? (
              <p className="description">
                {data.description || 'No description provided.'}
              </p>
            ) : null}
          </div>
          {data && tab !== 'run' ? (
            <Button
              type="button"
              variant="primary"
              size="md"
              className="run"
              onClick={() => setTab('run')}
            >
              <Play aria-hidden />
              Run
            </Button>
          ) : null}
        </div>
        {data ? (
          <FunctionFacts
            detail={data}
            language={language}
            lastCall={lastCall}
          />
        ) : null}
      </header>

      {detail.error ? (
        <ErrorNote
          title="Couldn't load function details"
          call="engine::functions::info"
          message={detail.error}
          onRetry={detail.reload}
        />
      ) : data === null ? (
        <Note>Loading detail…</Note>
      ) : (
        <Tabs
          value={tab}
          onValueChange={(value) => setTab(value as DocTab)}
          className="console-catalog-tabs console-fn-tabs"
        >
          <TabsList>
            <TabsTrigger value="contract" icon={<Braces />}>
              Contract
            </TabsTrigger>
            <TabsTrigger value="run" icon={<Play />}>
              Run
            </TabsTrigger>
            <TabsTrigger value="bindings" icon={<Zap />}>
              Bindings
              {bindings.length > 0 ? <Badge>{bindings.length}</Badge> : null}
            </TabsTrigger>
            <TabsTrigger value="calls" icon={<Activity />}>
              Calls
            </TabsTrigger>
          </TabsList>
          <TabsContent value="contract">
            <FunctionContract detail={data} onEdit={() => setTab('run')} />
          </TabsContent>
          <TabsContent value="run">
            <InvokePanel
              host={host}
              functionId={functionId}
              requestSchema={data.request_schema}
              prefill={prefill}
              label="Run"
              runningLabel="Running…"
              runRef={invokeRunRef}
              layout="split"
            />
          </TabsContent>
          <TabsContent value="bindings">
            <FunctionTriggers detail={data} />
          </TabsContent>
          <TabsContent value="calls">
            <ActivityFeed
              host={host}
              functionId={functionId}
              onReplay={replay}
            />
          </TabsContent>
        </Tabs>
      )}
    </div>
  )
}

/**
 * Every identity fact once, in one line: who serves the function, in what
 * runtime, what fires it, and when it last ran.
 */
function FunctionFacts({
  detail,
  language,
  lastCall,
}: {
  detail: FunctionDetail
  language?: string
  lastCall?: SpanEvent
}) {
  const now = new Date()
  const bindings = detail.registered_triggers
  const byType = new Map<string, number>()
  for (const ref of bindings) {
    byType.set(ref.trigger_type, (byType.get(ref.trigger_type) ?? 0) + 1)
  }
  const nextRun = bindings
    .map((ref) =>
      cronExpression({
        id: ref.id,
        trigger_type: ref.trigger_type,
        function_id: detail.function_id,
        worker_name: detail.worker_name,
        config: ref.config,
      }),
    )
    .map((expression) => (expression ? nextCronRun(expression, now) : null))
    .filter((d): d is Date => d !== null)
    .sort((a, b) => a.getTime() - b.getTime())[0]

  return (
    <dl className="console-fn-facts">
      <div>
        <dt>Worker</dt>
        <dd className="mono">{detail.worker_name}</dd>
      </div>
      {language ? (
        <div>
          <dt>Runtime</dt>
          <dd className="mono">{language}</dd>
        </div>
      ) : null}
      <div>
        <dt>Bindings</dt>
        <dd>
          {bindings.length === 0
            ? 'None, runs only when called'
            : [...byType.entries()]
                .map(([type, n]) => `${n} ${type}`)
                .join(', ')}
        </dd>
      </div>
      {nextRun ? (
        <div>
          <dt>Next run</dt>
          <dd className="mono">{untilLabel(nextRun, now)}</dd>
        </div>
      ) : null}
      <div>
        <dt>Last call</dt>
        <dd>
          {lastCall ? (
            <span className="last-call">
              <StatusDot tone={lastCall.ok ? 'ok' : 'alert'} />
              <span className="mono">
                {agoLabel(lastCall.atMs, Date.now())} ·{' '}
                {lastCall.durationMs > 0
                  ? formatDuration(lastCall.durationMs)
                  : 'running'}
              </span>
            </span>
          ) : (
            'None since this page opened'
          )}
        </dd>
      </div>
    </dl>
  )
}

/**
 * The function's contract, which is what the operator came to read: the input
 * fields a caller must supply, then the shape that comes back, then whatever
 * metadata the worker attached — and, beside it on a wide pane, the call
 * itself as a payload and as a terminal line.
 */
function FunctionContract({
  detail,
  onEdit,
}: {
  detail: FunctionDetail
  onEdit: () => void
}) {
  const hasMetadata =
    detail.metadata !== undefined &&
    detail.metadata !== null &&
    (typeof detail.metadata !== 'object' ||
      Array.isArray(detail.metadata) ||
      Object.keys(detail.metadata).length > 0)

  const template = templateFromSchema(detail.request_schema)
  const command = asCliCommand(detail.function_id, parseOr(template, {}))

  return (
    <div className="console-fn-contract">
      <div className="contract-main">
        <SchemaTable
          label="Input"
          schema={detail.request_schema}
          empty="This function registered no input schema."
        />
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
            <JsonHighlight
              code={pretty(detail.metadata)}
              className="console-catalog-json"
              wrap
            />
          </div>
        ) : null}
      </div>
      <aside className="console-fn-callit" aria-label="Call this function">
        <Eyebrow>Call it</Eyebrow>
        <div className="snippet">
          <span className="caption">
            Payload, generated from the input schema
          </span>
          <JsonHighlight
            code={template}
            className="console-catalog-json"
            wrap
          />
        </div>
        <div className="snippet">
          <span className="caption">Terminal</span>
          <code className="cli">{command}</code>
        </div>
        <div className="actions">
          <Button type="button" variant="ghost" size="sm" onClick={onEdit}>
            <Play aria-hidden />
            Edit in Run
          </Button>
          <CopyButton value={command} label="Copy command" />
        </div>
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
 * What fires this function, one card per binding: the family tile, the
 * binding in its family's words (`GET /users/:id`, `every 5 min`), and the
 * raw config for the cases those words compress away.
 */
function FunctionTriggers({ detail }: { detail: FunctionDetail }) {
  if (detail.registered_triggers.length === 0) {
    return (
      <Note>
        Nothing is bound to this function. It runs only when something calls it.
      </Note>
    )
  }
  const now = new Date()
  return (
    <div className="console-catalog-trigcards">
      {detail.registered_triggers.map((ref) => {
        // The refs on a function detail carry no worker/summary fields; the
        // function's own identity fills them so trigger-kinds can read the
        // binding the same way the triggers page does.
        const binding: RegisteredTrigger = {
          id: ref.id,
          trigger_type: ref.trigger_type,
          function_id: detail.function_id,
          worker_name: detail.worker_name,
          config: ref.config,
        }
        const spec = familyOf(ref.trigger_type)
        const expression = cronExpression(binding)
        const next = expression ? nextCronRun(expression, now) : null
        const config = pretty(ref.config ?? {})
        return (
          <div key={ref.id} className="console-catalog-trigcard">
            <FamilyGlyph family={spec.family} tone={spec.tone} />
            <div className="copy">
              <div className="line1">
                <span className="name">{summarize(binding)}</span>
                <span
                  className={`${uiClasses.eyebrow} console-catalog-tag`}
                  data-tone={spec.tone}
                >
                  {spec.label}
                </span>
              </div>
              <span className="type">
                {ref.trigger_type} · {ref.id}
              </span>
              {next ? (
                <span className="fine">Next run {untilLabel(next, now)}</span>
              ) : null}
              {config !== '{}' ? (
                <JsonHighlight
                  code={config}
                  className="console-catalog-json"
                  wrap
                />
              ) : null}
            </div>
            <CopyIconButton value={ref.id} label="Copy binding id" />
          </div>
        )
      })}
    </div>
  )
}
