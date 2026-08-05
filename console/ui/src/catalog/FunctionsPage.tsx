/**
 * The Functions page (`#/ext/functions`): every function registered on the
 * bus, grouped by namespace, with the detail and invoke panes on the right.
 *
 * Two calls back the page. `engine::functions::list` is the catalogue
 * (cheap, one row per function); `engine::functions::info` is fetched only
 * for the selected row, because that is where the schemas live and the
 * fleet has hundreds of functions.
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
import { useCallback, useMemo, useState } from 'react'
import {
  type FunctionSummary,
  functionInfo,
  listFunctions,
  useFleetChanges,
  useResource,
} from './engine'
import { InvokePanel } from './InvokePanel'
import { compareGroups, namespaceOf, pretty } from './schema'
import {
  CatalogHead,
  CatalogRow,
  CatalogShell,
  Chip,
  DetailHead,
  ErrorNote,
  GroupHeader,
  Note,
  useGroupToggle,
} from './widgets'

/** Function namespaces always start expanded; there is no noisy bucket. */
const alwaysOpen = () => true

export function FunctionsPage({ host }: { host: Host }) {
  const [showInternal, setShowInternal] = useState(false)
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const groupState = useGroupToggle(alwaysOpen)

  const load = useCallback(
    () => listFunctions(host, { includeInternal: showInternal }),
    [host, showInternal],
  )
  const functions = useResource(load)
  useFleetChanges(host, functions.reload)

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    const matched = (functions.data ?? []).filter((fn) => {
      if (!needle) return true
      return (
        fn.function_id.toLowerCase().includes(needle) ||
        fn.worker_name.toLowerCase().includes(needle) ||
        (fn.description ?? '').toLowerCase().includes(needle)
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
      head={
        <CatalogHead
          title="functions"
          count={
            search.trim() ? `${shown} of ${functions.data?.length ?? 0}` : shown
          }
          search={search}
          onSearch={setSearch}
          searchPlaceholder="search functions, workers, descriptions…"
          onRefresh={functions.reload}
          loading={functions.loading}
        >
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
                : 'workers register their functions on connect — start one and it lists here.'
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
                      selected={selected === fn.function_id}
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
      />
      {detail.error ? (
        <ErrorNote call="engine::functions::info" message={detail.error} />
      ) : detail.data === null ? (
        <Note>loading detail…</Note>
      ) : (
        <Tabs defaultValue="invoke" className="console-catalog-tabs">
          <TabsList>
            <TabsTrigger value="invoke">invoke</TabsTrigger>
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
            />
          </TabsContent>
          <TabsContent value="request">
            <SchemaPane
              schema={detail.data.request_schema}
              empty="this function registered no request schema."
            />
          </TabsContent>
          <TabsContent value="response">
            <SchemaPane
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

function SchemaPane({ schema, empty }: { schema: unknown; empty: string }) {
  if (schema === undefined || schema === null) return <Note>{empty}</Note>
  return (
    <JsonHighlight
      code={pretty(schema)}
      className="console-catalog-json"
      wrap
    />
  )
}
