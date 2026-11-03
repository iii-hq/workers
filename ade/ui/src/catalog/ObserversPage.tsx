/**
 * The Observers page (page `observers`): the same sidebar + workspace shell
 * as the functions page. The sidebar lists every observed function with its
 * observers indented beneath it, each row led by the family's glyph (globe,
 * clock, layers…); the workspace is always present — a hero when nothing is
 * selected, the function or observer document when something is.
 *
 * Two lists make one view. `engine::functions::list` names the event
 * sources (private functions their worker calls with each event, and
 * functions that take args); `engine::observers::list` is the set of live
 * OBSERVERS (what will run, and which target each call reaches). A source
 * nobody observes still lists — knowing a source exists is half of what the
 * page is for — and every other observed function lists under its own
 * heading. Event sources come first. The function itself is selected by
 * clicking its heading; only observers get rows.
 *
 * An observer is named by its family, from its args and filters
 * (`observer-kinds`): `GET /users/:id`, `every 5 min`, the queue topic, the
 * state scope. The detail pane then offers that family's REAL fire path
 * where one exists — an actual request for http, a real publish for a queue
 * subscriber — and falls back to calling the observer's target directly,
 * labelled as exactly that: a source is private, so nothing but its worker
 * can call it.
 */

import {
  Button,
  EmptyState,
  type Host,
  JsonHighlight,
  type PageCommandsApi,
  PageHeader,
  SearchField,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Radio } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { describeCron, nextCronRun, untilLabel } from './cron'
import {
  disposeObserver,
  type FunctionDetail,
  type FunctionSummary,
  functionInfo,
  isEventSource,
  listFunctions,
  listObservers,
  type ObserverInfo,
  useLiveSignals,
  useResource,
} from './engine'
import { HttpTester } from './HttpTester'
import { InvokePanel } from './InvokePanel'
import {
  cronExpression,
  type Family,
  familyOf,
  httpBinding,
  isPlumbing,
  observerChips,
  queueTopic,
  summarize,
} from './observer-kinds'
import { QueuePublish } from './QueuePublish'
import { SchemaTable } from './SchemaTable'
import { pretty } from './schema'
import {
  CatalogListSkeleton,
  CatalogRow,
  CatalogShell,
  CatalogWorkspace,
  Chip,
  ContextItem,
  ContextPanel,
  CopyButton,
  Crumb,
  ErrorNote,
  Facts,
  FamilyGlyph,
  FilterChips,
  FnGlyph,
  GroupHeader,
  Hero,
  IdentityHead,
  LiveDot,
  Note,
  SideCount,
  StatTile,
  useGroupToggle,
} from './widgets'

type Selection =
  | { kind: 'function'; id: string }
  | { kind: 'observer'; observer: ObserverInfo }

interface SourceGroup {
  /** The observed function; a synthesized row when the list does not carry it. */
  source: FunctionSummary
  observers: ObserverInfo[]
}

/** The chip tone of an observer's state. */
function stateTone(state: string): 'ok' | 'warn' | 'alert' | 'ink' {
  if (state === 'active') return 'ok'
  if (state === 'error') return 'alert'
  if (state === 'pending' || state === 'suspended') return 'warn'
  return 'ink'
}

function totalOf(counts: Record<string, number>): number {
  return Object.values(counts).reduce((n, c) => n + c, 0)
}

export function ObserversPage({
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
  const [family, setFamily] = useState<Family | null>(null)
  const [selected, setSelected] = useState<Selection | null>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)

  // Two independent reads the page needs together, so it pays for one round
  // trip. Functions name the sources and put each target's description on
  // its row.
  const loadCatalog = useCallback(async () => {
    const [observers, functions] = await Promise.all([
      listObservers(host),
      listFunctions(host, { includeInternal: true }),
    ])
    return { observers, functions }
  }, [host])
  const catalog = useResource(loadCatalog)
  // Observers change state on their own; sources come and go with their
  // worker's function surface.
  useLiveSignals(
    host,
    [
      'engine::on-observers-changed',
      'engine::on-functions-changed',
      'engine::on-workers-changed',
    ],
    catalog.reload,
  )

  const functionsById = useMemo(
    () =>
      new Map<string, FunctionSummary>(
        (catalog.data?.functions ?? []).map((f) => [f.function_id, f]),
      ),
    [catalog.data],
  )
  const describeFunction = useCallback(
    (id: string) => functionsById.get(id)?.description ?? undefined,
    [functionsById],
  )

  const partitioned = useMemo(() => {
    if (!catalog.data)
      return { groups: [] as SourceGroup[], plumbing: [] as ObserverInfo[] }
    // Plumbing (per-tab delivery handlers, injected-UI assets, config
    // hot-reload handlers) is real but never what this page is opened FOR —
    // it folds into one collapsed section at the bottom instead of putting
    // `configuration::on-updated` above everything.
    const plumbing: ObserverInfo[] = []
    const bySource = new Map<string, ObserverInfo[]>()
    for (const observer of catalog.data.observers) {
      if (isPlumbing(observer)) {
        plumbing.push(observer)
        continue
      }
      const bucket = bySource.get(observer.function_id)
      if (bucket) bucket.push(observer)
      else bySource.set(observer.function_id, [observer])
    }

    const sources = new Map<string, FunctionSummary>()
    // Every event source lists, observed or not. Internal ones (the
    // engine's own, every worker's plumbing) only with `internal` on.
    for (const fn of catalog.data.functions) {
      if (!isEventSource(fn) || fn.function_id.startsWith('console::ui::'))
        continue
      if (!showInternal && isInternalSource(fn)) continue
      sources.set(fn.function_id, fn)
    }
    // An observed function the list does not carry (another namespace, or
    // hidden from this session) still needs a home: synthesize a heading
    // rather than dropping its rows.
    for (const id of bySource.keys()) {
      if (!sources.has(id)) {
        sources.set(
          id,
          functionsById.get(id) ?? {
            function_id: id,
            worker_name: 'unknown',
            description: null,
          },
        )
      }
    }

    const groups = [...sources.values()].map((source) => ({
      source,
      observers: (bySource.get(source.function_id) ?? []).sort((a, b) =>
        summarize(a).localeCompare(summarize(b)),
      ),
    }))
    plumbing.sort((a, b) =>
      a.target.function_id.localeCompare(b.target.function_id),
    )
    return { groups, plumbing }
  }, [catalog.data, functionsById, showInternal])
  const allGroups = partitioned.groups

  const familyCounts = useMemo(() => {
    const counts = new Map<Family, number>()
    for (const group of allGroups) {
      const key = familyOf(group.source.function_id).family
      counts.set(key, (counts.get(key) ?? 0) + 1)
    }
    return counts
  }, [allGroups])

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    return allGroups
      .filter((group) => {
        const id = group.source.function_id
        if (family && familyOf(id).family !== family) return false
        if (!needle) return true
        if (
          id.toLowerCase().includes(needle) ||
          group.source.worker_name.toLowerCase().includes(needle) ||
          (group.source.description ?? '').toLowerCase().includes(needle)
        ) {
          return true
        }
        return group.observers.some(
          (o) =>
            o.target.function_id.toLowerCase().includes(needle) ||
            (o.worker.worker_name ?? '').toLowerCase().includes(needle) ||
            summarize(o).toLowerCase().includes(needle),
        )
      })
      .sort((a, b) => {
        // Event sources first, then the busiest; never by activity, so rows
        // stay put as calls arrive.
        const source =
          Number(isEventSource(b.source)) - Number(isEventSource(a.source))
        if (source !== 0) return source
        const count = b.observers.length - a.observers.length
        if (count !== 0) return count
        return a.source.function_id.localeCompare(b.source.function_id)
      })
  }, [allGroups, family, search])

  const shownObservers = groups.reduce((n, g) => n + g.observers.length, 0)
  const totalObservers = allGroups.reduce((n, g) => n + g.observers.length, 0)

  // Most sources have no observers; expanding all of them buries the ones
  // that are observed, so a function opens by default only when something
  // observes it.
  const observerCounts = useMemo(
    () =>
      new Map(groups.map((g) => [g.source.function_id, g.observers.length])),
    [groups],
  )
  const groupState = useGroupToggle((id) => (observerCounts.get(id) ?? 0) > 0)

  const selectFunction = (id: string) =>
    setSelected((prev) =>
      prev?.kind === 'function' && prev.id === id
        ? null
        : { kind: 'function', id },
    )
  const selectObserver = (observer: ObserverInfo) =>
    setSelected((prev) =>
      prev?.kind === 'observer' && prev.observer.id === observer.id
        ? null
        : { kind: 'observer', observer },
    )

  // Keep an open document tied to the live registry. Observers change state
  // in place, or disappear when disposed.
  useEffect(() => {
    if (!catalog.data || !selected) return
    if (selected.kind === 'function') {
      const exists =
        functionsById.has(selected.id) ||
        catalog.data.observers.some((o) => o.function_id === selected.id)
      if (!exists) setSelected(null)
      return
    }
    const current = catalog.data.observers.find(
      (o) => o.id === selected.observer.id,
    )
    if (!current) setSelected(null)
    else if (current !== selected.observer) {
      setSelected({ kind: 'observer', observer: current })
    }
  }, [catalog.data, functionsById, selected])

  // The page's primary verbs, for the palette and the keyboard while this
  // pane has focus.
  useEffect(
    () =>
      commands?.register([
        {
          id: 'search',
          title: 'Search observers',
          detail: 'Focus the observer search field',
          keywords: ['find', 'filter'],
          run: () => searchInputRef.current?.focus(),
        },
        {
          id: 'refresh',
          title: 'Refresh observers',
          detail: 'Reload the observers and their sources',
          keywords: ['reload'],
          run: () => catalog.reload(),
        },
      ]),
    [commands, catalog.reload],
  )

  return (
    <CatalogShell
      side={side}
      hasSelection={selected !== null}
      header={
        <PageHeader
          icon={<Radio />}
          title="Observers"
          description={
            <span className="console-catalog-header-desc">
              Observers and the functions they observe
            </span>
          }
          onClose={onRequestClose}
          className="console-catalog-page-header"
          actions={
            <>
              <LiveDot />
              <Button
                variant="pill"
                size="sm"
                type="button"
                onClick={catalog.reload}
                disabled={catalog.loading}
              >
                {catalog.loading ? 'loading…' : 'refresh'}
              </Button>
            </>
          }
        />
      }
      sideTop={
        <>
          <div className="console-catalog-search-row">
            <SearchField
              ref={searchInputRef}
              name="catalog-search"
              className="console-catalog-search"
              value={search}
              onChange={setSearch}
              placeholder="search observers…"
              aria-label="search observers"
            />
            <Button
              variant="pill"
              size="sm"
              type="button"
              className="console-catalog-header-toggle"
              onClick={() => setShowInternal((v) => !v)}
              aria-pressed={showInternal}
              title="include the engine's own sources nobody observes"
            >
              internal
            </Button>
          </div>
          <FilterChips
            counts={familyCounts}
            selected={family}
            onSelect={setFamily}
          />
        </>
      }
      sideFooter={
        <SideCount>
          {catalog.data === null
            ? 'loading observers…'
            : search.trim() || family
              ? `showing ${groups.length} of ${allGroups.length} functions · ${shownObservers} observers`
              : `${allGroups.length} functions · ${totalObservers} observers`}
        </SideCount>
      }
      list={
        catalog.error ? (
          <ErrorNote
            title="couldn't load observers"
            call="engine::observers::list"
            message={catalog.error}
            onRetry={catalog.reload}
          />
        ) : catalog.data === null ? (
          <CatalogListSkeleton label="loading observers" />
        ) : groups.length === 0 ? (
          <EmptyState
            title={
              search.trim() || family ? 'nothing matches' : 'no observers'
            }
            description={
              search.trim() || family
                ? 'no function, target, path, topic, or schedule contains that.'
                : 'workers observe functions on connect, and event sources list when their worker registers them — start one and it lists here.'
            }
            action={
              search.trim() || family
                ? {
                    label: 'clear filters',
                    onClick: () => {
                      setSearch('')
                      setFamily(null)
                    },
                  }
                : undefined
            }
          />
        ) : (
          <>
            {groups.map((group) => {
              const id = group.source.function_id
              const spec = familyOf(id)
              return (
                <div key={id} className="console-catalog-section">
                  <GroupHeader
                    label={id}
                    meta={group.source.worker_name}
                    count={group.observers.length}
                    countLabel="observer"
                    tone={spec.tone}
                    toneLabel={spec.label}
                    open={groupState.isOpen(id)}
                    onToggle={() => groupState.toggle(id)}
                    collapsible={group.observers.length > 0}
                    onSelect={() => selectFunction(id)}
                    selected={
                      selected?.kind === 'function' && selected.id === id
                    }
                  />
                  {groupState.isOpen(id)
                    ? group.observers.map((observer) => (
                        <CatalogRow
                          key={observer.id}
                          icon={
                            <FamilyGlyph
                              family={spec.family}
                              tone={spec.tone}
                            />
                          }
                          primary={summarize(observer)}
                          secondary={targetLine(observer, describeFunction)}
                          selected={
                            selected?.kind === 'observer' &&
                            selected.observer.id === observer.id
                          }
                          onClick={() => selectObserver(observer)}
                        />
                      ))
                    : null}
                </div>
              )
            })}
            {partitioned.plumbing.length > 0 && !search.trim() && !family ? (
              <div className="console-catalog-section console-catalog-plumbing">
                <GroupHeader
                  label="plumbing"
                  meta="ADE + configuration internals"
                  count={partitioned.plumbing.length}
                  countLabel="observer"
                  open={groupState.isOpen('__plumbing')}
                  onToggle={() => groupState.toggle('__plumbing')}
                />
                {groupState.isOpen('__plumbing')
                  ? partitioned.plumbing.map((observer) => (
                      <CatalogRow
                        key={observer.id}
                        icon={
                          <FamilyGlyph
                            family={familyOf(observer.function_id).family}
                            tone="ink"
                          />
                        }
                        primary={summarize(observer)}
                        secondary={`${observer.function_id} → ${observer.target.function_id}`}
                        selected={
                          selected?.kind === 'observer' &&
                          selected.observer.id === observer.id
                        }
                        onClick={() => selectObserver(observer)}
                      />
                    ))
                  : null}
              </div>
            ) : null}
          </>
        )
      }
      main={
        selected === null ? (
          <Hero
            glyph={<FamilyGlyph family="other" size="hero" />}
            eyebrow="observers"
            title="understand what starts work"
            body="select a function to read the event it is called with and the args its observers pass, or select an observer nested beneath it to see the route, schedule, subscription or filter that connects it to a target."
            items={[
              {
                label: 'source',
                value: 'a function its worker calls with each event',
              },
              {
                label: 'observer',
                value: 'its args, filters, delivery and policy',
              },
              {
                label: 'target',
                value: 'the function each matching call runs',
              },
            ]}
          />
        ) : selected.kind === 'function' ? (
          <SourceDocument
            host={host}
            functionId={selected.id}
            observerCount={
              catalog.data?.observers.filter(
                (o) => o.function_id === selected.id,
              ).length ?? 0
            }
            onBack={() => setSelected(null)}
          />
        ) : (
          <ObserverDocument
            host={host}
            observer={selected.observer}
            description={describeFunction(selected.observer.target.function_id)}
            onBack={() => setSelected(null)}
            onDisposed={() => {
              setSelected(null)
              catalog.reload()
            }}
          />
        )
      }
    />
  )
}

/** The engine's own sources: listed when observed, or with `internal` on. */
const ENGINE_SOURCE_PREFIXES = [
  'engine::',
  'observability::',
  'configuration::',
  'stream::',
]

function isInternalSource(fn: FunctionSummary): boolean {
  return ENGINE_SOURCE_PREFIXES.some((prefix) =>
    fn.function_id.startsWith(prefix),
  )
}

function targetLine(
  observer: ObserverInfo,
  describe: (id: string) => string | undefined,
): string {
  const target = observer.target.function_id
  if (!target) return '(no target function)'
  const description = describe(target)
  const state = observer.state === 'active' ? '' : ` · ${observer.state}`
  return `→ ${target}${description ? ` — ${description}` : ''}${state}`
}

function SourceDocument({
  host,
  functionId,
  observerCount,
  onBack,
}: {
  host: Host
  functionId: string
  observerCount: number
  onBack: () => void
}) {
  const load = useCallback(
    () => functionInfo(host, functionId),
    [host, functionId],
  )
  const detail = useResource(load)
  const spec = familyOf(functionId)

  return (
    <CatalogWorkspace
      context={
        detail.data ? (
          <SourceContext detail={detail.data} observerCount={observerCount} />
        ) : undefined
      }
    >
      <div className="console-catalog-doc">
        <Crumb
          trail={['observers', detail.data?.worker_name, functionId]}
          onBack={onBack}
        />
        <IdentityHead
          glyph={
            <FamilyGlyph family={spec.family} tone={spec.tone} size="lg" />
          }
          title={functionId}
          status={
            detail.data && isEventSource(detail.data)
              ? 'event source'
              : 'observed function'
          }
          description={
            detail.data
              ? detail.data.description || 'no description provided.'
              : undefined
          }
          chips={
            detail.data ? (
              <>
                <Chip k={spec.label} v="source" tone={spec.tone} />
                <Chip k="worker" v={detail.data.worker_name} />
                <Chip k="observers" v={String(observerCount)} />
                {detail.data.private ? <Chip k="private" v="yes" /> : null}
              </>
            ) : null
          }
          actions={<CopyButton value={functionId} label="copy id" />}
        />
        {detail.error ? (
          <ErrorNote
            title="couldn't load the function"
            call="engine::functions::info"
            message={detail.error}
            onRetry={detail.reload}
          />
        ) : detail.data === null ? (
          <Note>Loading detail…</Note>
        ) : (
          <Tabs defaultValue="args" className="console-catalog-tabs">
            <TabsList>
              <TabsTrigger value="args">Args</TabsTrigger>
              <TabsTrigger value="event">Event</TabsTrigger>
            </TabsList>
            <TabsContent value="args">
              <SchemaTable
                schema={detail.data.args_format}
                empty="This function takes no args — observers select its calls with filters on the event."
              />
            </TabsContent>
            <TabsContent value="event">
              <SchemaTable
                schema={detail.data.response_schema}
                empty="This function publishes no event schema."
              />
            </TabsContent>
          </Tabs>
        )}
      </div>
    </CatalogWorkspace>
  )
}

function SourceContext({
  detail,
  observerCount,
}: {
  detail: FunctionDetail
  observerCount: number
}) {
  const spec = familyOf(detail.function_id)
  return (
    <>
      <ContextPanel title="function details">
        <Facts
          items={[
            { label: 'function id', value: <code>{detail.function_id}</code> },
            { label: 'family', value: spec.label },
            { label: 'worker', value: detail.worker_name },
            { label: 'observers', value: String(observerCount) },
            { label: 'private', value: detail.private ? 'yes' : 'no' },
          ]}
        />
      </ContextPanel>
      <ContextPanel
        title="contracts"
        description="Schemas published with this function."
      >
        <Facts
          items={[
            {
              label: 'args',
              value: detail.args_format !== undefined ? 'defined' : 'none',
            },
            {
              label: 'event',
              value: detail.response_schema !== undefined ? 'defined' : 'none',
            },
          ]}
        />
      </ContextPanel>
    </>
  )
}

function ObserverDocument({
  host,
  observer,
  description,
  onBack,
  onDisposed,
}: {
  host: Host
  observer: ObserverInfo
  description?: string
  onBack: () => void
  onDisposed: () => void
}) {
  // The event schema belongs to the observed function, so a direct call of
  // the target opens on the shape the target receives: the event under
  // `deliver: output`, the call's input under `deliver: input`.
  const load = useCallback(
    () => functionInfo(host, observer.function_id),
    [host, observer.function_id],
  )
  const source = useResource(load)
  const [disposing, setDisposing] = useState(false)
  const [disposeError, setDisposeError] = useState<string | null>(null)

  const spec = familyOf(observer.function_id)
  const http = httpBinding(observer)
  const topic = queueTopic(observer)
  const chips = observerChips(observer)
  const target = observer.target.function_id
  const targetSchema =
    observer.deliver === 'output'
      ? source.data?.response_schema
      : observer.deliver === 'input'
        ? source.data?.request_schema
        : undefined

  const dispose = async () => {
    setDisposing(true)
    setDisposeError(null)
    try {
      await disposeObserver(host, observer.id)
      onDisposed()
    } catch (err) {
      setDisposeError(errorMessage(err))
      setDisposing(false)
    }
  }

  return (
    <CatalogWorkspace
      context={<ObserverContext observer={observer} description={description} />}
    >
      <div className="console-catalog-doc">
        <Crumb
          trail={['observers', observer.function_id, summarize(observer)]}
          onBack={onBack}
        />
        <IdentityHead
          glyph={
            <FamilyGlyph family={spec.family} tone={spec.tone} size="lg" />
          }
          title={summarize(observer)}
          status={`${observer.state} observer`}
          description={
            target
              ? `Runs ${target} with each matching call of ${observer.function_id}.`
              : 'This observer has no target function.'
          }
          chips={
            <>
              <Chip
                k={spec.label}
                v={observer.function_id}
                tone={spec.tone}
              />
              <Chip
                k="state"
                v={observer.state}
                tone={stateTone(observer.state)}
              />
              <Chip k="fires" v={String(observer.fires)} />
              {chips.map((chip) => (
                <Chip key={chip.label} k={chip.label} v={chip.value} />
              ))}
            </>
          }
          actions={
            <>
              <CopyButton value={observer.id} label="copy id" />
              <Button
                variant="pill"
                size="sm"
                type="button"
                onClick={dispose}
                disabled={disposing || observer.state === 'disposed'}
              >
                {disposing ? 'disposing…' : 'dispose'}
              </Button>
            </>
          }
        />
        {disposeError ? (
          <ErrorNote
            title="couldn't dispose the observer"
            call="engine::observers::dispose"
            message={disposeError}
          />
        ) : null}
        {observer.reason ? (
          <Note>
            {observer.reason.code}: {observer.reason.message}
          </Note>
        ) : null}

        <div className="console-catalog-tabs">
          <Tabs defaultValue="fire">
            <TabsList>
              <TabsTrigger value="fire">
                {spec.family === 'http'
                  ? 'Send Request'
                  : spec.family === 'queue'
                    ? 'Publish'
                    : spec.family === 'cron'
                      ? 'Run Now'
                      : 'Call Target'}
              </TabsTrigger>
              <TabsTrigger value="args">Args</TabsTrigger>
              <TabsTrigger value="filters">Filters</TabsTrigger>
              <TabsTrigger value="delivery">Delivery</TabsTrigger>
            </TabsList>

            <TabsContent value="fire">
              {http ? (
                <HttpTester host={host} binding={http} />
              ) : topic ? (
                <QueuePublish host={host} topic={topic} />
              ) : target ? (
                <InvokePanel
                  host={host}
                  functionId={target}
                  requestSchema={targetSchema}
                  label={spec.family === 'cron' ? 'run now' : 'call target'}
                  runningLabel={spec.family === 'cron' ? 'running…' : 'calling…'}
                  hint={
                    spec.family === 'cron'
                      ? `Calls ${target} with a tick-shaped payload. The schedule is untouched, and the next scheduled tick still runs it.`
                      : `Calls ${target} directly with this payload. ${observer.function_id} is not called, so this observer's filters and delivery do not apply.`
                  }
                />
              ) : (
                <Note>
                  this observer carries no target function — nothing to call.
                </Note>
              )}
            </TabsContent>

            <TabsContent value="args">
              <JsonHighlight
                code={pretty(observer.args)}
                className="console-catalog-json"
                wrap
              />
            </TabsContent>

            <TabsContent value="filters">
              {Object.keys(observer.filters).length === 0 ? (
                <Note>No filters: every call of the function reaches the target.</Note>
              ) : (
                <JsonHighlight
                  code={pretty(observer.filters)}
                  className="console-catalog-json"
                  wrap
                />
              )}
            </TabsContent>

            <TabsContent value="delivery">
              <Facts
                items={[
                  { label: 'on', value: observer.on },
                  { label: 'deliver', value: observer.deliver },
                  {
                    label: 'capture',
                    value: observer.capture.length
                      ? observer.capture.join(', ')
                      : 'nothing',
                  },
                  {
                    label: 'max fires',
                    value:
                      observer.max_fires !== undefined
                        ? String(observer.max_fires)
                        : 'unlimited',
                  },
                  {
                    label: 'expires in',
                    value:
                      observer.expires_in_ms !== undefined
                        ? `${observer.expires_in_ms} ms`
                        : 'never',
                  },
                  {
                    label: 'max in flight',
                    value:
                      observer.max_in_flight !== undefined
                        ? String(observer.max_in_flight)
                        : 'engine default',
                  },
                ]}
              />
            </TabsContent>
          </Tabs>
        </div>
      </div>
    </CatalogWorkspace>
  )
}

function ObserverContext({
  observer,
  description,
}: {
  observer: ObserverInfo
  description?: string
}) {
  const spec = familyOf(observer.function_id)
  const hasDeliveryFacts = Boolean(
    cronExpression(observer) || httpBinding(observer) || queueTopic(observer),
  )
  const dropped = Object.entries(observer.dropped)
  const failures = Object.entries(observer.activation_failures)
  return (
    <>
      <ContextPanel
        title="target function"
        description="Each matching call runs this function."
        wide
      >
        {observer.target.function_id ? (
          <ContextItem
            glyph={<FnGlyph />}
            title={observer.target.function_id}
            description={description || 'No function description provided.'}
            meta={observer.target.namespace}
          />
        ) : (
          <span className="console-catalog-context-empty">
            No target function is registered.
          </span>
        )}
      </ContextPanel>

      <ContextPanel title="observer details">
        <Facts
          items={[
            { label: 'observer id', value: <code>{observer.id}</code> },
            {
              label: 'observes',
              value: <code>{observer.function_id}</code>,
            },
            { label: 'family', value: spec.label },
            { label: 'namespace', value: observer.namespace },
            {
              label: 'observing worker',
              value:
                observer.worker.worker_name ?? observer.worker.worker_id ?? '—',
            },
            {
              label: 'created',
              value: observer.created_at_ms
                ? new Date(observer.created_at_ms).toLocaleString()
                : '—',
            },
          ]}
        />
      </ContextPanel>

      <ContextPanel title="counters">
        <Facts
          items={[
            { label: 'fires', value: String(observer.fires) },
            {
              label: 'dropped',
              value: dropped.length
                ? dropped.map(([reason, n]) => `${n} ${reason}`).join(', ')
                : String(totalOf(observer.dropped)),
            },
            {
              label: 'activation failures',
              value: failures.length
                ? failures.map(([code, n]) => `${n} ${code}`).join(', ')
                : '0',
            },
          ]}
        />
      </ContextPanel>

      {hasDeliveryFacts ? (
        <ContextPanel title="schedule, route or topic">
          <FamilyFacts observer={observer} />
        </ContextPanel>
      ) : null}
    </>
  )
}

/** The stat tiles that only make sense for a given family. */
function FamilyFacts({ observer }: { observer: ObserverInfo }) {
  const expression = cronExpression(observer)
  if (expression) {
    const now = new Date()
    const next = nextCronRun(expression, now)
    return (
      <>
        <div className="console-catalog-schedule">
          <span className="readable">
            {describeCron(expression) ?? 'custom schedule'}
          </span>
          <code>{expression}</code>
        </div>
        <div className="console-catalog-tiles">
          <StatTile
            label="next run"
            value={next ? untilLabel(next, now) : 'not derivable'}
            hint={
              next
                ? next.toLocaleTimeString()
                : 'the expression restricts dates or weekdays'
            }
          />
          <StatTile
            label="status"
            value={observer.state === 'active' ? 'scheduled' : observer.state}
            tone={observer.state === 'active' ? 'ok' : 'warn'}
          />
        </div>
      </>
    )
  }

  const http = httpBinding(observer)
  if (http) {
    return (
      <div className="console-catalog-tiles">
        <StatTile label="method" value={http.method} />
        <StatTile
          label="path parameters"
          value={http.params.length ? http.params.join(', ') : 'none'}
        />
      </div>
    )
  }

  const topic = queueTopic(observer)
  if (topic) {
    return (
      <div className="console-catalog-tiles">
        <StatTile label="topic" value={topic} />
        <StatTile
          label="status"
          value={observer.state === 'active' ? 'consuming' : observer.state}
          tone={observer.state === 'active' ? 'ok' : 'warn'}
        />
      </div>
    )
  }
  return null
}
