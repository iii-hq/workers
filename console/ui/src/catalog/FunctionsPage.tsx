/**
 * The Functions page (`#/ext/functions`): a navigation sidebar of every
 * function on the bus — each row led by the `ƒ` tile, grouped by the worker
 * that registered it — and a workspace that is always present: a hero when
 * nothing is selected, the function document (breadcrumb, identity head,
 * overview/invoke/input/output/triggers/activity tabs) when one is.
 *
 * Live, never polled. `engine::functions-available` fires whenever functions
 * are registered or unregistered, so a worker connecting or dying is visible
 * here within a beat, and rows that arrived on the last tick flash once so
 * the change is legible rather than silent.
 *
 * `engine::functions::list` is the catalogue (one cheap row per function);
 * `engine::functions::info` is fetched per selection, because that is where
 * the schemas live and the fleet has hundreds of functions.
 * `engine::workers::list` rides along for each worker's runtime — the
 * document's "language" fact.
 *
 * Internal functions are hidden by default: the console's own per-tab
 * handlers and every worker's UI plumbing register as internal, and they
 * would otherwise outnumber the functions an operator came to find.
 */

import {
  Badge,
  Button,
  EmptyState,
  type Host,
  JsonHighlight,
  PageHeader,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ActivityFeed, formatDuration } from './ActivityFeed'
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
import { InvokePanel } from './InvokePanel'
import { agoLabel, LastCallMeta, useLiveActivity } from './live'
import { SchemaTable } from './SchemaTable'
import { pretty } from './schema'
import { cronExpression, familyOf, summarize } from './trigger-kinds'
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
  FnGlyph,
  GroupHeader,
  Hero,
  IdentityHead,
  LiveDot,
  Note,
  SearchField,
  SideCount,
  useGroupToggle,
} from './widgets'

/** Worker groups always start expanded; there is no noisy bucket. */
const alwaysOpen = () => true

export function FunctionsPage({
  host,
  side,
  onRequestClose,
}: {
  host: Host
  side?: 'left' | 'right'
  onRequestClose?: () => void
}) {
  const [showInternal, setShowInternal] = useState(false)
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const groupState = useGroupToggle(alwaysOpen)

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

  return (
    <CatalogShell
      side={side}
      hasSelection={selected !== null}
      header={
        <PageHeader
          icon={<span aria-hidden>ƒ</span>}
          title="functions"
          description={
            <span className="console-catalog-header-desc">
              registered functions, grouped by worker
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
        <div className="console-catalog-search-row">
          <SearchField
            value={search}
            onChange={setSearch}
            placeholder="search functions…"
          />
          <Button
            variant="pill"
            size="sm"
            type="button"
            className="console-catalog-header-toggle"
            onClick={() => setShowInternal((v) => !v)}
            aria-pressed={showInternal}
            title="include internal functions"
          >
            internal
          </Button>
        </div>
      }
      sideFooter={
        <SideCount>
          {catalog.data === null
            ? 'loading functions…'
            : search.trim()
              ? `showing ${shown} of ${total} functions`
              : `${total} total function${total === 1 ? '' : 's'}`}
        </SideCount>
      }
      list={
        catalog.error ? (
          <ErrorNote
            title="couldn't load functions"
            call="engine::functions::list"
            message={catalog.error}
            onRetry={catalog.reload}
          />
        ) : catalog.data === null ? (
          <CatalogListSkeleton label="loading functions" />
        ) : shown === 0 ? (
          <EmptyState
            title={
              search.trim() ? 'nothing matches' : 'no functions registered'
            }
            description={
              search.trim()
                ? 'no function id or worker name contains that text.'
                : 'workers register their functions on connect — start one and it appears here live.'
            }
            action={
              search.trim()
                ? { label: 'clear search', onClick: () => setSearch('') }
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
          />
        ) : (
          <Hero
            glyph={<FnGlyph size="hero" />}
            eyebrow="function catalog"
            title="explore registered functions"
            body="select a function from the sidebar to understand its contract, test it, and follow its recent calls. the catalog updates whenever workers connect or disconnect."
            items={[
              {
                label: 'overview',
                value: 'ownership, runtime, triggers, and metadata',
              },
              {
                label: 'contracts',
                value: 'input and output schemas in a readable field table',
              },
              {
                label: 'test',
                value:
                  'trigger with json, copy a cli command, or replay a call',
              },
            ]}
          />
        )
      }
    />
  )
}

function FunctionDocument({
  host,
  functionId,
  runtimeOf,
  lastCall,
  onBack,
}: {
  host: Host
  functionId: string
  /** Worker name → runtime, from the page-level workers list. */
  runtimeOf: (worker: string) => string | undefined
  lastCall?: SpanEvent
  onBack: () => void
}) {
  const load = useCallback(
    () => functionInfo(host, functionId),
    [host, functionId],
  )
  const detail = useResource(load)
  const [tab, setTab] = useState('invoke')
  const [prefill, setPrefill] = useState<{ value: unknown; nonce: number }>()

  useEffect(() => {
    setTab('invoke')
    setPrefill(undefined)
  }, [functionId])

  // Replaying from the activity feed hands the recorded input to the invoke
  // editor and moves the operator there — the whole point of the button.
  const replay = useCallback((value: unknown) => {
    setPrefill({ value, nonce: Date.now() })
    setTab('invoke')
  }, [])

  const language = detail.data ? runtimeOf(detail.data.worker_name) : undefined

  return (
    <CatalogWorkspace
      context={
        detail.data ? (
          <FunctionContext
            detail={detail.data}
            language={language}
            lastCall={lastCall}
            onShowTriggers={() => setTab('triggers')}
            onShowActivity={() => setTab('activity')}
          />
        ) : undefined
      }
    >
      <div className="console-catalog-doc">
        <Crumb
          trail={['functions', detail.data?.worker_name, functionId]}
          onBack={onBack}
        />
        <IdentityHead
          glyph={<FnGlyph size="lg" />}
          title={functionId}
          status="executable function"
          description={
            detail.data
              ? detail.data.description || 'no description provided.'
              : undefined
          }
          chips={
            detail.data ? (
              <>
                <Chip k="worker" v={detail.data.worker_name} />
                {language ? <Chip k="language" v={language} /> : null}
                {detail.data.registered_triggers.length > 0 ? (
                  <Chip
                    k="triggers"
                    v={String(detail.data.registered_triggers.length)}
                  />
                ) : null}
              </>
            ) : null
          }
          actions={
            <>
              <CopyButton value={functionId} label="copy id" />
              <Button
                type="button"
                variant="pill"
                size="sm"
                onClick={() => setTab('invoke')}
              >
                run function
              </Button>
            </>
          }
        />
        {detail.error ? (
          <ErrorNote
            title="couldn't load function details"
            call="engine::functions::info"
            message={detail.error}
            onRetry={detail.reload}
          />
        ) : detail.data === null ? (
          <Note>loading detail…</Note>
        ) : (
          <Tabs
            value={tab}
            onValueChange={setTab}
            className="console-catalog-tabs"
          >
            <TabsList>
              <TabsTrigger value="overview">overview</TabsTrigger>
              <TabsTrigger value="invoke">trigger</TabsTrigger>
              <TabsTrigger value="input">input</TabsTrigger>
              <TabsTrigger value="output">output</TabsTrigger>
              <TabsTrigger value="triggers">
                triggers
                {detail.data.registered_triggers.length > 0 ? (
                  <Badge>{detail.data.registered_triggers.length}</Badge>
                ) : null}
              </TabsTrigger>
              <TabsTrigger value="activity">activity</TabsTrigger>
            </TabsList>
            <TabsContent value="overview">
              <FunctionOverview detail={detail.data} />
            </TabsContent>
            <TabsContent value="invoke">
              <InvokePanel
                host={host}
                functionId={functionId}
                requestSchema={detail.data.request_schema}
                prefill={prefill}
                label="run function"
                runningLabel="running…"
              />
            </TabsContent>
            <TabsContent value="input">
              <SchemaTable
                schema={detail.data.request_schema}
                empty="this function registered no input schema."
              />
            </TabsContent>
            <TabsContent value="output">
              <SchemaTable
                schema={detail.data.response_schema}
                empty="this function registered no output schema."
              />
            </TabsContent>
            <TabsContent value="triggers">
              <FunctionTriggers detail={detail.data} />
            </TabsContent>
            <TabsContent value="activity">
              <ActivityFeed
                host={host}
                functionId={functionId}
                onReplay={replay}
              />
            </TabsContent>
          </Tabs>
        )}
      </div>
    </CatalogWorkspace>
  )
}

function FunctionContext({
  detail,
  language,
  lastCall,
  onShowTriggers,
  onShowActivity,
}: {
  detail: FunctionDetail
  language?: string
  lastCall?: SpanEvent
  onShowTriggers: () => void
  onShowActivity: () => void
}) {
  const now = new Date()
  return (
    <>
      <ContextPanel
        title="related triggers"
        description={
          detail.registered_triggers.length === 0
            ? 'No bindings currently trigger this function.'
            : `Triggered by ${detail.registered_triggers.length} binding${detail.registered_triggers.length === 1 ? '' : 's'}.`
        }
        action={
          detail.registered_triggers.length > 0
            ? { label: 'view all', onClick: onShowTriggers }
            : undefined
        }
        wide
      >
        {detail.registered_triggers.length === 0 ? (
          <span className="console-catalog-context-empty">
            This function runs only when called directly.
          </span>
        ) : (
          detail.registered_triggers.slice(0, 3).map((ref) => {
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
            return (
              <ContextItem
                key={ref.id}
                glyph={<FamilyGlyph family={spec.family} tone={spec.tone} />}
                title={summarize(binding)}
                description={ref.trigger_type}
                meta={next ? `next run ${untilLabel(next, now)}` : spec.label}
                onClick={onShowTriggers}
              />
            )
          })
        )}
      </ContextPanel>

      <ContextPanel title="function details">
        <Facts
          items={[
            { label: 'function id', value: <code>{detail.function_id}</code> },
            { label: 'worker', value: detail.worker_name },
            ...(language ? [{ label: 'language', value: language }] : []),
            {
              label: 'input schema',
              value: detail.request_schema !== undefined ? 'defined' : 'none',
            },
            {
              label: 'output schema',
              value: detail.response_schema !== undefined ? 'defined' : 'none',
            },
          ]}
        />
      </ContextPanel>

      <ContextPanel
        title="recent activity"
        action={
          lastCall ? { label: 'open feed', onClick: onShowActivity } : undefined
        }
      >
        {lastCall ? (
          <button
            type="button"
            className="console-catalog-context-activity"
            onClick={onShowActivity}
          >
            <span className="dot" data-ok={lastCall.ok} />
            <span className="activity-copy">
              <span>{agoLabel(lastCall.atMs, Date.now())}</span>
              <span>{lastCall.ok ? 'successful call' : 'failed call'}</span>
            </span>
            <span className="duration">
              {lastCall.durationMs > 0
                ? formatDuration(lastCall.durationMs)
                : 'running'}
            </span>
          </button>
        ) : (
          <span className="console-catalog-context-empty">
            No call observed since this page opened.
          </span>
        )}
      </ContextPanel>
    </>
  )
}

/**
 * Function identity and contract facts already live in the contextual rail.
 * The overview is reserved for extra metadata so it never repeats that same
 * fact sheet in the main work surface.
 */
function FunctionOverview({ detail }: { detail: FunctionDetail }) {
  const hasMetadata =
    detail.metadata !== undefined &&
    detail.metadata !== null &&
    (typeof detail.metadata !== 'object' ||
      Array.isArray(detail.metadata) ||
      Object.keys(detail.metadata).length > 0)

  return hasMetadata ? (
    <div className="console-catalog-overview">
      <span className="console-catalog-field-label">metadata</span>
      <JsonHighlight
        code={pretty(detail.metadata)}
        className="console-catalog-json"
        wrap
      />
    </div>
  ) : (
    <Note>this function registered no additional metadata.</Note>
  )
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
        nothing is bound to this function — it runs only when something calls
        it.
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
                <span className="console-catalog-tag" data-tone={spec.tone}>
                  {spec.label}
                </span>
              </div>
              <span className="type">
                {ref.trigger_type} · {ref.id}
              </span>
              {next ? (
                <span className="fine">next run {untilLabel(next, now)}</span>
              ) : null}
              {config !== '{}' ? (
                <JsonHighlight
                  code={config}
                  className="console-catalog-json"
                  wrap
                />
              ) : null}
            </div>
          </div>
        )
      })}
    </div>
  )
}
