/**
 * The Functions page (`#/ext/functions`): every function registered on the
 * bus, grouped by namespace, with the detail, schema, invoke and live
 * activity panes on the right.
 *
 * Live, never polled. `engine::functions-available` fires whenever functions
 * are registered or unregistered, so a worker connecting or dying is visible
 * here within a beat, and rows that arrived on the last tick flash once so
 * the change is legible rather than silent.
 *
 * `engine::functions::list` is the catalogue (one cheap row per function);
 * `engine::functions::info` is fetched per selection, because that is where
 * the schemas live and the fleet has hundreds of functions.
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
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ActivityFeed } from './ActivityFeed'
import {
  type FunctionSummary,
  functionInfo,
  listFunctions,
  useLiveSignals,
  useResource,
} from './engine'
import { InvokePanel } from './InvokePanel'
import { LastCallMeta, NowStrip, useLiveActivity } from './live'
import { SchemaTable } from './SchemaTable'
import { compareGroups, namespaceOf, pretty } from './schema'
import {
  CatalogHead,
  CatalogRow,
  CatalogShell,
  Chip,
  CopyButton,
  DetailHead,
  ErrorNote,
  GroupHeader,
  LiveDot,
  Note,
  useGroupToggle,
} from './widgets'

/** Function namespaces always start expanded; there is no noisy bucket. */
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

  const load = useCallback(
    () => listFunctions(host, { includeInternal: showInternal }),
    [host, showInternal],
  )
  const functions = useResource(load)
  useLiveSignals(host, ['engine::functions-available'], functions.reload)
  const activity = useLiveActivity(host)

  // Ids that appeared on the last tick, so an arrival is visible instead of
  // silently changing the row count. The first load is not "new".
  const [arrived, setArrived] = useState<ReadonlySet<string>>(new Set())
  const seenRef = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!functions.data) return
    const ids = new Set(functions.data.map((f) => f.function_id))
    const previous = seenRef.current
    seenRef.current = ids
    if (!previous) return
    const fresh = new Set([...ids].filter((id) => !previous.has(id)))
    if (fresh.size === 0) return
    setArrived(fresh)
    const timer = window.setTimeout(() => setArrived(new Set()), 2000)
    return () => window.clearTimeout(timer)
  }, [functions.data])

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    // Ids and workers only. Description text matches surprised more than
    // they helped: searching `config` surfaced harness::triggers::list
    // because its description mentions config, which reads as broken.
    const matched = (functions.data ?? []).filter((fn) => {
      if (!needle) return true
      return (
        fn.function_id.toLowerCase().includes(needle) ||
        fn.worker_name.toLowerCase().includes(needle)
      )
    })
    const byGroup = new Map<string, FunctionSummary[]>()
    for (const fn of matched) {
      const group = namespaceOf(fn.function_id)
      const bucket = byGroup.get(group)
      if (bucket) bucket.push(fn)
      else byGroup.set(group, [fn])
    }
    return [...byGroup.entries()]
      .map(([label, items]) => ({
        label,
        items: items.sort((a, b) => a.function_id.localeCompare(b.function_id)),
      }))
      .sort((a, b) => compareGroups(a.label, b.label))
  }, [functions.data, search])

  const shown = groups.reduce((n, g) => n + g.items.length, 0)

  return (
    <CatalogShell
      side={side}
      head={
        <CatalogHead
          title="functions"
          count={
            search.trim() ? `${shown} of ${functions.data?.length ?? 0}` : shown
          }
          search={search}
          onSearch={setSearch}
          searchPlaceholder="search function ids or workers…"
          onRefresh={functions.reload}
          loading={functions.loading}
          onRequestClose={onRequestClose}
          below={
            <NowStrip
              activity={activity}
              onSelect={(functionId) => setSelected(functionId)}
            />
          }
        >
          <LiveDot />
          <Button
            variant="pill"
            size="sm"
            onClick={() => setShowInternal((v) => !v)}
            aria-pressed={showInternal}
          >
            {showInternal ? 'internal shown' : 'internal hidden'}
          </Button>
        </CatalogHead>
      }
      list={
        functions.error ? (
          <ErrorNote call="engine::functions::list" message={functions.error} />
        ) : functions.data === null ? (
          <Note>loading functions…</Note>
        ) : shown === 0 ? (
          <EmptyState
            title={
              search.trim() ? 'nothing matches' : 'no functions registered'
            }
            description={
              search.trim()
                ? 'no function id, worker, or description contains that text.'
                : 'workers register their functions on connect — start one and it appears here live.'
            }
          />
        ) : (
          groups.map((group) => (
            <div key={group.label} className="console-catalog-section">
              <GroupHeader
                label={group.label}
                meta={`${group.items.length}`}
                open={groupState.isOpen(group.label)}
                onToggle={() => groupState.toggle(group.label)}
              />
              {!groupState.isOpen(group.label)
                ? null
                : group.items.map((fn) => (
                    <CatalogRow
                      key={fn.function_id}
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
      detail={
        selected ? (
          <FunctionDetailPane
            host={host}
            functionId={selected}
            onClose={() => setSelected(null)}
          />
        ) : null
      }
    />
  )
}

function FunctionDetailPane({
  host,
  functionId,
  onClose,
}: {
  host: Host
  functionId: string
  onClose: () => void
}) {
  const load = useCallback(
    () => functionInfo(host, functionId),
    [host, functionId],
  )
  const detail = useResource(load)
  const [tab, setTab] = useState('invoke')
  const [prefill, setPrefill] = useState<{ value: unknown; nonce: number }>()

  // Replaying from the activity feed hands the recorded input to the invoke
  // editor and moves the operator there — the whole point of the button.
  const replay = useCallback((value: unknown) => {
    setPrefill({ value, nonce: Date.now() })
    setTab('invoke')
  }, [])

  return (
    <>
      <DetailHead
        title={functionId}
        subtitle={
          detail.data ? (
            <>
              <Chip k="worker" v={detail.data.worker_name} />
              {detail.data.registered_triggers.length > 0 ? (
                <Chip
                  k="triggers"
                  v={String(detail.data.registered_triggers.length)}
                />
              ) : null}
              {detail.data.description ? (
                <span className="console-catalog-desc">
                  {detail.data.description}
                </span>
              ) : null}
            </>
          ) : null
        }
        onClose={onClose}
      >
        <CopyButton value={functionId} label="copy id" />
      </DetailHead>
      {detail.error ? (
        <ErrorNote call="engine::functions::info" message={detail.error} />
      ) : detail.data === null ? (
        <Note>loading detail…</Note>
      ) : (
        <Tabs
          value={tab}
          onValueChange={setTab}
          className="console-catalog-tabs"
        >
          <TabsList>
            <TabsTrigger value="invoke">invoke</TabsTrigger>
            <TabsTrigger value="activity">activity</TabsTrigger>
            <TabsTrigger value="request">request</TabsTrigger>
            <TabsTrigger value="response">response</TabsTrigger>
            <TabsTrigger value="triggers">
              triggers
              {detail.data.registered_triggers.length > 0 ? (
                <Badge>{detail.data.registered_triggers.length}</Badge>
              ) : null}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="invoke">
            <InvokePanel
              host={host}
              functionId={functionId}
              requestSchema={detail.data.request_schema}
              prefill={prefill}
            />
          </TabsContent>
          <TabsContent value="activity">
            <ActivityFeed
              host={host}
              functionId={functionId}
              onReplay={replay}
            />
          </TabsContent>
          <TabsContent value="request">
            <SchemaTable
              schema={detail.data.request_schema}
              empty="this function registered no request schema."
            />
          </TabsContent>
          <TabsContent value="response">
            <SchemaTable
              schema={detail.data.response_schema}
              empty="this function registered no response schema."
            />
          </TabsContent>
          <TabsContent value="triggers">
            {detail.data.registered_triggers.length === 0 ? (
              <Note>
                nothing is bound to this function — it runs only when something
                calls it.
              </Note>
            ) : (
              detail.data.registered_triggers.map((trigger) => (
                <div key={trigger.id} className="console-catalog-binding">
                  <div className="console-catalog-binding-head">
                    <Chip k="type" v={trigger.trigger_type} />
                    <span className="console-catalog-id">{trigger.id}</span>
                  </div>
                  <JsonHighlight
                    code={pretty(trigger.config ?? {})}
                    className="console-catalog-json"
                    wrap
                  />
                </div>
              ))
            )}
          </TabsContent>
        </Tabs>
      )}
    </>
  )
}
