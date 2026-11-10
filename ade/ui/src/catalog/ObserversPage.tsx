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
  ConfirmDialog,
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
} from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Copy, CornerDownRight, Filter, Radio, RefreshCw, Trash2, Zap } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { describeCron, nextCronRun, untilLabel } from './cron'
import {
  disposeObserver,
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
import { familyKeyName, familyName, sentence } from './names'
import { observerDeliveryFacts } from './observer-info'
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
  DocHead,
  DocTile,
  EmptyDoc,
  ErrorNote,
  Facts,
  FamilyFilter,
  FamilyIcon,
  GroupHeader,
  IdText,
  LiveDot,
  type MenuAction,
  Note,
  type RowDot,
  SideCount,
  StateBadge,
  SwitchRow,
  TreeRow,
  useGroupToggle,
} from './widgets'

type Selection = { kind: 'function'; id: string } | { kind: 'observer'; observer: ObserverInfo }

interface SourceGroup {
  /** The observed function; a synthesized row when the list does not carry it. */
  source: FunctionSummary
  observers: ObserverInfo[]
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
    ['engine::on-observers-changed', 'engine::on-functions-changed', 'engine::on-workers-changed'],
    catalog.reload,
  )

  const functionsById = useMemo(
    () => new Map<string, FunctionSummary>((catalog.data?.functions ?? []).map((f) => [f.function_id, f])),
    [catalog.data],
  )
  const describeFunction = useCallback((id: string) => functionsById.get(id)?.description ?? undefined, [functionsById])

  const partitioned = useMemo(() => {
    if (!catalog.data) return { groups: [] as SourceGroup[], plumbing: [] as ObserverInfo[] }
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
      if (!isEventSource(fn) || fn.function_id.startsWith('ade::ui::')) continue
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
      observers: (bySource.get(source.function_id) ?? []).sort((a, b) => summarize(a).localeCompare(summarize(b))),
    }))
    plumbing.sort((a, b) => a.target.function_id.localeCompare(b.target.function_id))
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
        const source = Number(isEventSource(b.source)) - Number(isEventSource(a.source))
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
  const observerCounts = useMemo(() => new Map(groups.map((g) => [g.source.function_id, g.observers.length])), [groups])
  const groupState = useGroupToggle((id) => (observerCounts.get(id) ?? 0) > 0)

  const selectFunction = (id: string) =>
    setSelected((prev) => (prev?.kind === 'function' && prev.id === id ? null : { kind: 'function', id }))
  const selectObserver = (observer: ObserverInfo) =>
    setSelected((prev) =>
      prev?.kind === 'observer' && prev.observer.id === observer.id ? null : { kind: 'observer', observer },
    )

  // Keep an open document tied to the live registry. Observers change state
  // in place, or disappear when disposed.
  useEffect(() => {
    if (!catalog.data || !selected) return
    if (selected.kind === 'function') {
      const exists = functionsById.has(selected.id) || catalog.data.observers.some((o) => o.function_id === selected.id)
      if (!exists) setSelected(null)
      return
    }
    const current = catalog.data.observers.find((o) => o.id === selected.observer.id)
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

  const filtered = Boolean(search.trim() || family)
  const isSelectedObserver = (observer: ObserverInfo) =>
    selected?.kind === 'observer' && selected.observer.id === observer.id

  return (
    <CatalogShell
      side={side}
      label="Observers"
      hasSelection={selected !== null}
      header={
        <PageHeader
          icon={<Radio />}
          title="Observers"
          description={<span className="console-catalog-header-desc">Observers and the functions they observe</span>}
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
          placeholder="Search observers"
          aria-label="Search observers"
        />
      }
      filters={
        <>
          <FamilyFilter counts={familyCounts} selected={family} onSelect={setFamily} nameOf={familyKeyName} />
          <SwitchRow label="Internal" checked={showInternal} onChange={setShowInternal} />
        </>
      }
      sideFooter={
        <SideCount>
          {catalog.data === null
            ? 'Loading observers…'
            : filtered
              ? `Showing ${groups.length} of ${allGroups.length} functions · ${shownObservers} observers`
              : `${allGroups.length} functions · ${totalObservers} observers`}
        </SideCount>
      }
      list={
        catalog.error ? (
          <ErrorNote
            title="Couldn't load observers"
            call="engine::observers::list"
            message={catalog.error}
            onRetry={catalog.reload}
          />
        ) : catalog.data === null ? (
          <CatalogListSkeleton label="Loading observers" />
        ) : groups.length === 0 ? (
          <EmptyState
            title={filtered ? 'Nothing matches' : 'No observers'}
            description={
              filtered
                ? 'No function, target, path, topic or schedule contains that.'
                : 'Workers observe functions when they connect, and event sources list when their worker registers them. Start one and it shows here.'
            }
            action={
              filtered
                ? {
                    label: 'Clear filters',
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
              const open = groupState.isOpen(id)
              return (
                <section key={id} className="console-catalog-section" aria-label={id}>
                  <TreeRow
                    icon={<FamilyIcon family={spec.family} tone={spec.tone} />}
                    id={id}
                    tooltip={`${id}\n${familyName(spec)} · ${group.source.worker_name}`}
                    count={group.observers.length}
                    open={open}
                    onToggle={() => groupState.toggle(id)}
                    selected={selected?.kind === 'function' && selected.id === id}
                    onSelect={() => selectFunction(id)}
                  />
                  {open
                    ? group.observers.map((observer) => {
                        const text = observerRowText(observer, describeFunction)
                        return (
                          <CatalogRow
                            key={observer.id}
                            nested
                            primary={text.primary}
                            tooltip={text.tooltip}
                            dot={stateDot(observer.state)}
                            dotLabel={sentence(observer.state)}
                            meta={
                              observer.state === 'active' || observer.state === 'suspended'
                                ? undefined
                                : sentence(observer.state)
                            }
                            selected={isSelectedObserver(observer)}
                            onClick={() => selectObserver(observer)}
                          />
                        )
                      })
                    : null}
                </section>
              )
            })}
            {partitioned.plumbing.length > 0 && !filtered ? (
              <section className="console-catalog-section console-catalog-plumbing" aria-label="Plumbing">
                <GroupHeader
                  label="Plumbing"
                  meta="ADE and configuration internals"
                  count={partitioned.plumbing.length}
                  open={groupState.isOpen('__plumbing')}
                  onToggle={() => groupState.toggle('__plumbing')}
                />
                {groupState.isOpen('__plumbing')
                  ? partitioned.plumbing.map((observer) => (
                      <CatalogRow
                        key={observer.id}
                        icon={<FamilyIcon family={familyOf(observer.function_id).family} tone="ink" />}
                        primary={<IdText id={observer.target.function_id || summarize(observer)} />}
                        tooltip={`${observer.target.function_id} on ${observer.function_id}`}
                        selected={isSelectedObserver(observer)}
                        onClick={() => selectObserver(observer)}
                      />
                    ))
                  : null}
              </section>
            ) : null}
          </>
        )
      }
      main={
        selected === null ? (
          <EmptyDoc
            icon={<Radio />}
            title="Observers decide what runs"
            body="Pick a function to see the event it carries and the args its observers pass. Pick an observer under it to see the route, schedule, topic or filter that connects it to its target, and fire it."
            items={[
              {
                label: 'Function',
                icon: <Zap />,
                value: 'Its worker calls it with each event.',
              },
              {
                label: 'Observer',
                icon: <Filter />,
                value: 'Its args, filters and delivery pick which calls count.',
              },
              {
                label: 'Target',
                icon: <CornerDownRight />,
                value: 'The function each matching call runs.',
              },
            ]}
          />
        ) : selected.kind === 'function' ? (
          <SourceDocument
            key={selected.id}
            host={host}
            functionId={selected.id}
            observerCount={catalog.data?.observers.filter((o) => o.function_id === selected.id).length ?? 0}
            onBack={() => setSelected(null)}
          />
        ) : (
          <ObserverDocument
            key={selected.observer.id}
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
const ENGINE_SOURCE_PREFIXES = ['engine::', 'observability::', 'configuration::', 'stream::']

function isInternalSource(fn: FunctionSummary): boolean {
  return ENGINE_SOURCE_PREFIXES.some((prefix) => fn.function_id.startsWith(prefix))
}

/**
 * An observer's two lines under its function. A family that names the
 * observer (`GET /users/:id`, `Every 5 minutes`, a topic) leads with that
 * name and the target under it; one that would only repeat the function id
 * leads with the target and its description instead.
 */
function observerRowText(
  observer: ObserverInfo,
  describe: (id: string) => string | undefined,
): { primary: ReactNode; tooltip: string } {
  const target = observer.target.function_id
  const state = observer.state === 'active' ? '' : ` (${observer.state})`
  const name = summarize(observer)
  if (name !== observer.function_id) {
    return {
      primary: name,
      tooltip: `${name}${state}\n→ ${target || 'no target'}`,
    }
  }
  if (!target) return { primary: 'No target function', tooltip: `No target${state}` }
  const description = describe(target)
  return {
    primary: <IdText id={target} />,
    tooltip: `→ ${target}${state}${description ? `\n${description}` : ''}`,
  }
}

/** An observer's state as its row dot. */
function stateDot(state: string): RowDot {
  if (state === 'active') return 'ok'
  if (state === 'error') return 'alert'
  if (state === 'pending') return 'warn'
  return 'off'
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
  const load = useCallback(() => functionInfo(host, functionId), [host, functionId])
  const detail = useResource(load)
  const spec = familyOf(functionId)
  const data = detail.data
  const { state: copyState, copy } = useCopyFlash(functionId, 2000)

  return (
    <div className="console-catalog-doc">
      <DocHead
        tile={<DocTile family={spec.family} tone={spec.tone} />}
        eyebrow="Observed function"
        title={functionId}
        status={data && isEventSource(data) ? <StateBadge tone="ink">Event source</StateBadge> : null}
        description={data ? data.description || 'No description provided.' : undefined}
        stats={
          data
            ? [
                { label: 'Family', value: familyName(spec) },
                { label: 'Worker', value: data.worker_name, mono: true },
                { label: 'Observers', value: String(observerCount), mono: true },
                { label: 'Private', value: data.private ? 'Yes' : 'No' },
              ]
            : undefined
        }
        menu={[
          {
            label: copyState === 'copied' ? 'Copied' : 'Copy function id',
            icon: copyState === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />,
            onSelect: copy,
            keepOpen: true,
          },
        ]}
        menuLabel="More function actions"
        onBack={onBack}
        backLabel="Back to observers"
      />
      {detail.error ? (
        <div className="console-catalog-column">
          <ErrorNote
            title="Couldn't load the function"
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
        <ShadcnTabs defaultValue="args" className="console-catalog-tabs">
          <ShadcnTabsList variant="line" activateOnFocus className="console-catalog-tablist">
            <ShadcnTabsTrigger value="args">Args</ShadcnTabsTrigger>
            <ShadcnTabsTrigger value="event">Event</ShadcnTabsTrigger>
          </ShadcnTabsList>
          <ShadcnTabsContent value="args" className="console-catalog-panel">
            <SchemaTable
              schema={data.args_format}
              empty="This function takes no args. Observers pick its calls with filters on the event."
            />
          </ShadcnTabsContent>
          <ShadcnTabsContent value="event" className="console-catalog-panel">
            <SchemaTable schema={data.response_schema} empty="This function publishes no event schema." />
          </ShadcnTabsContent>
        </ShadcnTabs>
      )}
    </div>
  )
}

/** The fire tab's name, in the family's own verb. */
function fireLabel(family: Family): string {
  if (family === 'http') return 'Send request'
  if (family === 'queue') return 'Publish'
  if (family === 'cron') return 'Run now'
  return 'Call target'
}

function stateTone(state: string): 'ok' | 'warn' | 'alert' | 'ink' {
  if (state === 'active') return 'ok'
  if (state === 'error') return 'alert'
  if (state === 'pending' || state === 'suspended') return 'warn'
  return 'ink'
}

function totalOf(counts: Record<string, number>): number {
  return Object.values(counts).reduce((n, c) => n + c, 0)
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
  // a legacy `deliver: output` (an older engine), the call's input under
  // `deliver: input`.
  const load = useCallback(() => functionInfo(host, observer.function_id), [host, observer.function_id])
  const source = useResource(load)
  const [confirming, setConfirming] = useState(false)
  const { state: copyState, copy } = useCopyFlash(observer.id, 2000)

  const spec = familyOf(observer.function_id)
  const http = httpBinding(observer)
  const topic = queueTopic(observer)
  const target = observer.target.function_id
  const targetSchema =
    observer.deliver === 'output'
      ? source.data?.response_schema
      : observer.deliver === 'input'
        ? source.data?.request_schema
        : undefined

  const menu: (MenuAction | 'separator')[] = [
    {
      label: copyState === 'copied' ? 'Copied' : 'Copy observer id',
      icon: copyState === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />,
      onSelect: copy,
      keepOpen: true,
    },
  ]
  if (observer.state !== 'disposed') {
    menu.push('separator', {
      label: 'Dispose…',
      icon: <Trash2 aria-hidden />,
      destructive: true,
      onSelect: () => setConfirming(true),
    })
  }

  const filterCount = Object.keys(observer.filters).length
  // A family that names the observer (a route, a schedule, a topic) titles
  // the document with that name; otherwise the target is the useful name,
  // since the observed function is already in the fact line.
  const named = summarize(observer)
  const title = named !== observer.function_id || !target ? named : target

  return (
    <div className="console-catalog-doc">
      <DocHead
        tile={<DocTile family={spec.family} tone={spec.tone} />}
        eyebrow={`${familyName(spec)} observer`}
        title={title}
        status={<StateBadge tone={stateTone(observer.state)}>{sentence(observer.state)}</StateBadge>}
        description={
          !target
            ? 'This observer has no target function.'
            : title === target
              ? `Runs on each matching call of ${observer.function_id}.`
              : `Runs ${target} with each matching call of ${observer.function_id}.`
        }
        stats={[
          { label: 'Observes', value: observer.function_id, mono: true },
          { label: 'Fires', value: String(observer.fires), mono: true },
          { label: 'Fires on', value: observer.outcomes.map(sentence).join(', ') },
          {
            label: 'Worker',
            value: observer.worker.worker_name ?? observer.worker.worker_id ?? '—',
            mono: true,
          },
        ]}
        menu={menu}
        menuLabel="More observer actions"
        onBack={onBack}
        backLabel="Back to observers"
      />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="Dispose this observer?"
        description="Its target stops running on these calls. The worker that registered it has to observe again to bring it back."
        confirmLabel="Dispose"
        busyLabel="Disposing…"
        tone="danger"
        onConfirm={async () => {
          // The engine answers `false` when it keeps the observer; throwing
          // keeps the dialog open with the reason instead of closing as if
          // it had worked.
          const disposed = await disposeObserver(host, observer.id)
          if (!disposed) throw new Error('The engine kept this observer. Refresh and try again.')
          onDisposed()
        }}
      />
      {observer.reason ? (
        <div className="console-catalog-column">
          <Note>
            {observer.reason.code}: {observer.reason.message}
          </Note>
        </div>
      ) : null}

      <ShadcnTabs defaultValue="overview" className="console-catalog-tabs">
        <ShadcnTabsList variant="line" activateOnFocus className="console-catalog-tablist">
          <ShadcnTabsTrigger value="overview">Overview</ShadcnTabsTrigger>
          <ShadcnTabsTrigger value="fire">{fireLabel(spec.family)}</ShadcnTabsTrigger>
          <ShadcnTabsTrigger value="args">Args</ShadcnTabsTrigger>
          <ShadcnTabsTrigger value="filters">
            Filters
            {filterCount > 0 ? <span className="tab-count">{filterCount}</span> : null}
          </ShadcnTabsTrigger>
        </ShadcnTabsList>

        <ShadcnTabsContent value="overview" className="console-catalog-panel">
          <ObserverOverview observer={observer} description={description} />
        </ShadcnTabsContent>

        <ShadcnTabsContent value="fire" className="console-catalog-panel">
          {http ? (
            <HttpTester host={host} binding={http} />
          ) : topic ? (
            <QueuePublish host={host} topic={topic} />
          ) : target ? (
            <InvokePanel
              host={host}
              functionId={target}
              requestSchema={targetSchema}
              label={spec.family === 'cron' ? 'Run now' : 'Call target'}
              runningLabel={spec.family === 'cron' ? 'Running…' : 'Calling…'}
              hint={
                spec.family === 'cron'
                  ? `Calls ${target} with a tick-shaped payload. The schedule is untouched, and the next scheduled tick still runs it.`
                  : `Calls ${target} directly with this payload. ${observer.function_id} is not called, so this observer's filters and delivery do not apply.`
              }
            />
          ) : (
            <Note>This observer has no target function, so there is nothing to call.</Note>
          )}
        </ShadcnTabsContent>

        <ShadcnTabsContent value="args" className="console-catalog-panel">
          {Object.keys(observer.args).length === 0 ? (
            <Note>No args.</Note>
          ) : (
            <JsonHighlight code={pretty(observer.args)} className="console-catalog-json" wrap />
          )}
        </ShadcnTabsContent>

        <ShadcnTabsContent value="filters" className="console-catalog-panel">
          {filterCount === 0 ? (
            <Note>No filters: every call of the function reaches the target.</Note>
          ) : (
            <JsonHighlight code={pretty(observer.filters)} className="console-catalog-json" wrap />
          )}
        </ShadcnTabsContent>
      </ShadcnTabs>
    </div>
  )
}

/**
 * What used to be the side rail, read top to bottom: the target, the
 * family's own facts (schedule, route, topic), delivery, counters and the
 * observer's identity — each an outlined list on the Directory's recipe.
 */
function ObserverOverview({ observer, description }: { observer: ObserverInfo; description?: string }) {
  const dropped = Object.entries(observer.dropped)
  const failures = Object.entries(observer.activation_failures)
  const chips = observerChips(observer)
  return (
    <div className="console-catalog-stack">
      <section className="console-catalog-target">
        <h3>Target function</h3>
        {observer.target.function_id ? (
          <div className="target-card">
            <DocTile />
            <div className="copy">
              <span className="name">{observer.target.function_id}</span>
              <span className="desc">{description || 'No description provided.'}</span>
            </div>
            <span className="ns">{observer.target.namespace}</span>
          </div>
        ) : (
          <Note>No target function is registered.</Note>
        )}
      </section>

      <FamilyFacts observer={observer} />

      {chips.length > 0 ? (
        <Facts
          title="Matches"
          items={chips.map((chip) => ({
            label: sentence(chip.label),
            value: chip.value,
            mono: true,
          }))}
        />
      ) : null}

      {/* The rows come from observerDeliveryFacts (fires_on, or the legacy
          on/deliver on older engines); only the casing is this page's. */}
      <Facts
        title="Delivery"
        items={observerDeliveryFacts(observer).map((fact) => ({
          label: sentence(fact.label),
          value: sentence(fact.value),
        }))}
      />

      <Facts
        title="Counters"
        items={[
          { label: 'Fires', value: String(observer.fires), mono: true },
          {
            label: 'Dropped',
            mono: true,
            value: dropped.length
              ? dropped.map(([reason, n]) => `${n} ${reason}`).join(', ')
              : String(totalOf(observer.dropped)),
          },
          {
            label: 'Activation failures',
            mono: true,
            value: failures.length ? failures.map(([code, n]) => `${n} ${code}`).join(', ') : '0',
          },
        ]}
      />

      <Facts
        title="Identity"
        items={[
          { label: 'Observer id', value: observer.id, mono: true },
          { label: 'Namespace', value: observer.namespace, mono: true },
          {
            label: 'Created',
            value: observer.created_at_ms ? new Date(observer.created_at_ms).toLocaleString() : '—',
          },
        ]}
      />
    </div>
  )
}

/** The facts only a schedule, a route or a topic has. */
function FamilyFacts({ observer }: { observer: ObserverInfo }) {
  const expression = cronExpression(observer)
  if (expression) {
    const now = new Date()
    const next = nextCronRun(expression, now)
    return (
      <Facts
        wide
        title="Schedule"
        items={[
          {
            label: 'Runs',
            value: describeCron(expression) ?? 'Custom schedule',
          },
          { label: 'Expression', value: expression, mono: true },
          {
            label: 'Next run',
            value: next
              ? `${untilLabel(next, now)} · ${next.toLocaleTimeString()}`
              : 'Not derivable: the expression restricts dates or weekdays',
          },
        ]}
      />
    )
  }
  const http = httpBinding(observer)
  if (http) {
    return (
      <Facts
        wide
        title="Route"
        items={[
          { label: 'Method', value: http.method, mono: true },
          { label: 'Path', value: http.path, mono: true },
          {
            label: 'Path parameters',
            value: http.params.length ? http.params.join(', ') : 'None',
            mono: http.params.length > 0,
          },
        ]}
      />
    )
  }
  const topic = queueTopic(observer)
  if (topic) {
    return <Facts wide title="Queue" items={[{ label: 'Topic', value: topic, mono: true }]} />
  }
  return null
}
