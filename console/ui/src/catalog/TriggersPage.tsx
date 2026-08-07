/**
 * The Triggers page (`#/ext/triggers`): every trigger type published on the
 * bus, each with its live bindings underneath.
 *
 * Two lists make one view. `engine::triggers::list` is the catalogue of
 * TYPES (what can fire); `engine::registered-triggers::list` is the set of
 * live REGISTERED TRIGGERS (what will fire, and into which function). A type
 * with none still lists — knowing a type exists is half of what the page is
 * for — and a binding whose type is not in the catalogue lists under its own
 * heading rather than disappearing.
 *
 * A registered trigger is named by its family, not by its raw config (`trigger-kinds`):
 * `GET /users/:id`, `every 5 min`, the queue topic. The detail pane then
 * offers that family's REAL fire path where one exists — an actual request
 * for http, a real publish for a queue subscriber — and falls back to calling
 * the bound function directly, labelled as exactly that, where the engine has
 * no way to synthesize a firing.
 */

import {
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
import { describeCron, nextCronRun, untilLabel } from './cron'
import {
  type FunctionSummary,
  listFunctions,
  listRegisteredTriggers,
  listTriggerTypes,
  type RegisteredTrigger,
  type TriggerTypeSummary,
  triggerTypeInfo,
  useLiveSignals,
  useResource,
} from './engine'
import { HttpTester } from './HttpTester'
import { InvokePanel } from './InvokePanel'
import { LastCallMeta, NowStrip, useLiveActivity } from './live'
import { QueuePublish } from './QueuePublish'
import { pretty } from './schema'
import {
  configChips,
  cronExpression,
  type Family,
  familyOf,
  httpBinding,
  isPlumbing,
  queueTopic,
  summarize,
} from './trigger-kinds'
import {
  CatalogHead,
  CatalogRow,
  CatalogShell,
  Chip,
  CopyButton,
  DetailHead,
  ErrorNote,
  FilterChips,
  GroupHeader,
  Note,
  StatTile,
  useGroupToggle,
} from './widgets'

type Selection =
  | { kind: 'type'; id: string }
  | { kind: 'binding'; binding: RegisteredTrigger }

interface TypeGroup {
  type: TriggerTypeSummary
  bindings: RegisteredTrigger[]
}

export function TriggersPage({
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
  const [family, setFamily] = useState<Family | null>(null)
  const [selected, setSelected] = useState<Selection | null>(null)

  // Three independent reads the page needs together, so it pays for one round
  // trip. Functions come along to put each binding's target description on
  // the row, the way the old console did.
  const loadCatalog = useCallback(async () => {
    const [types, bindings, functions] = await Promise.all([
      listTriggerTypes(host, { includeInternal: showInternal }),
      listRegisteredTriggers(host, { includeInternal: showInternal }),
      listFunctions(host, { includeInternal: true }),
    ])
    return { types, bindings, functions }
  }, [host, showInternal])
  const catalog = useResource(loadCatalog)
  // A binding registers and unregisters with its worker's function surface,
  // so the same two engine signals cover this page.
  useLiveSignals(
    host,
    ['engine::functions-available', 'engine::workers-available'],
    catalog.reload,
  )

  const describeFunction = useMemo(() => {
    const byId = new Map<string, FunctionSummary>(
      (catalog.data?.functions ?? []).map((f) => [f.function_id, f]),
    )
    return (id: string) => byId.get(id)?.description ?? undefined
  }, [catalog.data])

  const activity = useLiveActivity(host)

  const partitioned = useMemo(() => {
    if (!catalog.data)
      return { groups: [] as TypeGroup[], plumbing: [] as RegisteredTrigger[] }
    // Plumbing (per-tab delivery handlers, injected-UI assets, config
    // hot-reload hooks) is real but never what this page is opened FOR —
    // it folds into one collapsed section at the bottom instead of putting
    // `configuration` above everything alphabetically.
    const plumbing: RegisteredTrigger[] = []
    const byType = new Map<string, RegisteredTrigger[]>()
    for (const binding of catalog.data.bindings) {
      if (isPlumbing(binding)) {
        plumbing.push(binding)
        continue
      }
      const bucket = byType.get(binding.trigger_type)
      if (bucket) bucket.push(binding)
      else byType.set(binding.trigger_type, [binding])
    }

    const known = new Map(catalog.data.types.map((t) => [t.id, t]))
    // A registration whose type the catalogue does not carry still needs a
    // home: synthesize a heading for it rather than dropping the row.
    for (const type of byType.keys()) {
      if (!known.has(type)) {
        known.set(type, { id: type, worker_name: 'unknown', description: null })
      }
    }

    // Types whose only registrations are plumbing carry no operator-facing
    // rows; drop the heading too unless the type itself is worth knowing.
    for (const id of [...known.keys()]) {
      if (id.startsWith('console:')) known.delete(id)
    }

    const groups = [...known.values()].map((type) => ({
      type,
      bindings: (byType.get(type.id) ?? []).sort((a, b) =>
        summarize(a).localeCompare(summarize(b)),
      ),
    }))
    plumbing.sort((a, b) => a.function_id.localeCompare(b.function_id))
    return { groups, plumbing }
  }, [catalog.data])
  const allGroups = partitioned.groups

  // Groups that just fired float to the top: during a harness turn the page
  // reads as what the agent is doing, not an alphabetical index.
  const lastFiredOf = useMemo(() => {
    return (group: TypeGroup): number => {
      let latest = 0
      for (const binding of group.bindings) {
        const span = activity.lastCall.get(binding.function_id)
        if (span && span.atMs > latest) latest = span.atMs
      }
      return latest
    }
  }, [activity.lastCall])

  const familyCounts = useMemo(() => {
    const counts = new Map<Family, number>()
    for (const group of allGroups) {
      const key = familyOf(group.type.id).family
      counts.set(
        key,
        (counts.get(key) ?? 0) + Math.max(1, group.bindings.length),
      )
    }
    return counts
  }, [allGroups])

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    return allGroups
      .filter((group) => {
        if (family && familyOf(group.type.id).family !== family) return false
        if (!needle) return true
        if (
          group.type.id.toLowerCase().includes(needle) ||
          group.type.worker_name.toLowerCase().includes(needle) ||
          (group.type.description ?? '').toLowerCase().includes(needle)
        ) {
          return true
        }
        return group.bindings.some(
          (b) =>
            b.function_id.toLowerCase().includes(needle) ||
            b.worker_name.toLowerCase().includes(needle) ||
            summarize(b).toLowerCase().includes(needle) ||
            (b.config_summary ?? '').toLowerCase().includes(needle),
        )
      })
      .sort((a, b) => {
        const fired = lastFiredOf(b) - lastFiredOf(a)
        if (fired !== 0) return fired
        const count = b.bindings.length - a.bindings.length
        if (count !== 0) return count
        return a.type.id.localeCompare(b.type.id)
      })
  }, [allGroups, family, search, lastFiredOf])

  const boundCount = groups.reduce((n, g) => n + g.bindings.length, 0)

  // Most of the catalogue is unbound types; expanding all of them buries the
  // ones that actually fire, so a type opens by default only when something
  // is bound to it.
  const bindingCounts = useMemo(
    () => new Map(groups.map((g) => [g.type.id, g.bindings.length])),
    [groups],
  )
  const groupState = useGroupToggle((id) => (bindingCounts.get(id) ?? 0) > 0)

  return (
    <CatalogShell
      side={side}
      head={
        <CatalogHead
          title="triggers"
          count={`${groups.length} types · ${boundCount} registered`}
          search={search}
          onSearch={setSearch}
          searchPlaceholder="search types, functions, paths, topics, schedules…"
          onRefresh={catalog.reload}
          loading={catalog.loading}
          onRequestClose={onRequestClose}
          below={
            <>
              <FilterChips
                counts={familyCounts}
                selected={family}
                onSelect={setFamily}
              />
              <NowStrip
                activity={activity}
                onSelect={(functionId) => {
                  const hit = allGroups
                    .flatMap((g) => g.bindings)
                    .find((b) => b.function_id === functionId)
                  if (hit) setSelected({ kind: 'binding', binding: hit })
                }}
              />
            </>
          }
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
        catalog.error ? (
          <ErrorNote call="engine::triggers::list" message={catalog.error} />
        ) : catalog.data === null ? (
          <Note>loading triggers…</Note>
        ) : groups.length === 0 ? (
          <EmptyState
            title={
              search.trim() || family ? 'nothing matches' : 'no trigger types'
            }
            description={
              search.trim() || family
                ? 'no type, bound function, path, topic, or schedule contains that.'
                : 'workers publish their trigger types on connect — start one and it lists here.'
            }
          />
        ) : (
          groups.map((group) => {
            const spec = familyOf(group.type.id)
            return (
              <div key={group.type.id} className="console-catalog-section">
                <GroupHeader
                  label={group.type.id}
                  meta={
                    group.bindings.length === 0
                      ? `${group.type.worker_name} · none registered`
                      : `${group.type.worker_name} · ${group.bindings.length}`
                  }
                  tone={spec.tone}
                  toneLabel={spec.label}
                  open={groupState.isOpen(group.type.id)}
                  onToggle={() => groupState.toggle(group.type.id)}
                />
                {groupState.isOpen(group.type.id) ? (
                  <>
                    <CatalogRow
                      primary="type detail"
                      secondary={group.type.description ?? undefined}
                      selected={
                        selected?.kind === 'type' &&
                        selected.id === group.type.id
                      }
                      onClick={() =>
                        setSelected((prev) =>
                          prev?.kind === 'type' && prev.id === group.type.id
                            ? null
                            : { kind: 'type', id: group.type.id },
                        )
                      }
                    />
                    {group.bindings.map((binding) => (
                      <CatalogRow
                        key={binding.id}
                        primary={summarize(binding)}
                        meta={
                          <LastCallMeta
                            span={activity.lastCall.get(binding.function_id)}
                          />
                        }
                        secondary={
                          binding.function_id
                            ? `${binding.function_id}${
                                describeFunction(binding.function_id)
                                  ? ` — ${describeFunction(binding.function_id)}`
                                  : ''
                              }`
                            : '(no target function)'
                        }
                        selected={
                          selected?.kind === 'binding' &&
                          selected.binding.id === binding.id
                        }
                        flash={activity.pulsing.has(binding.function_id)}
                        onClick={() =>
                          setSelected((prev) =>
                            prev?.kind === 'binding' &&
                            prev.binding.id === binding.id
                              ? null
                              : { kind: 'binding', binding },
                          )
                        }
                      />
                    ))}
                  </>
                ) : null}
              </div>
            )
          })
        )
      }
      footer={
        partitioned.plumbing.length > 0 && !search.trim() && !family ? (
          <div className="console-catalog-section console-catalog-plumbing">
            <GroupHeader
              label="plumbing"
              meta={`${partitioned.plumbing.length} console + config internals`}
              open={groupState.isOpen('__plumbing')}
              onToggle={() => groupState.toggle('__plumbing')}
            />
            {groupState.isOpen('__plumbing')
              ? partitioned.plumbing.map((binding) => (
                  <CatalogRow
                    key={binding.id}
                    primary={summarize(binding)}
                    secondary={`${binding.trigger_type} → ${binding.function_id}`}
                    selected={
                      selected?.kind === 'binding' &&
                      selected.binding.id === binding.id
                    }
                    onClick={() =>
                      setSelected((prev) =>
                        prev?.kind === 'binding' &&
                        prev.binding.id === binding.id
                          ? null
                          : { kind: 'binding', binding },
                      )
                    }
                  />
                ))
              : null}
          </div>
        ) : null
      }
      detail={
        selected === null ? null : selected.kind === 'type' ? (
          <TypeDetailPane
            host={host}
            typeId={selected.id}
            onClose={() => setSelected(null)}
          />
        ) : (
          <BindingDetailPane
            host={host}
            binding={selected.binding}
            description={describeFunction(selected.binding.function_id)}
            onClose={() => setSelected(null)}
          />
        )
      }
    />
  )
}

function TypeDetailPane({
  host,
  typeId,
  onClose,
}: {
  host: Host
  typeId: string
  onClose: () => void
}) {
  const load = useCallback(() => triggerTypeInfo(host, typeId), [host, typeId])
  const detail = useResource(load)

  return (
    <>
      <DetailHead
        title={typeId}
        subtitle={
          detail.data ? (
            <>
              <Chip k="worker" v={detail.data.worker_name} />
              {detail.data.instance_count !== undefined ? (
                <Chip k="registered" v={String(detail.data.instance_count)} />
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
        <CopyButton value={typeId} label="copy id" />
      </DetailHead>
      {detail.error ? (
        <ErrorNote call="engine::triggers::info" message={detail.error} />
      ) : detail.data === null ? (
        <Note>loading detail…</Note>
      ) : (
        <Tabs defaultValue="config" className="console-catalog-tabs">
          <TabsList>
            <TabsTrigger value="config">config schema</TabsTrigger>
            <TabsTrigger value="payload">payload schema</TabsTrigger>
          </TabsList>
          <TabsContent value="config">
            {detail.data.configuration_schema === undefined ? (
              <Note>
                this type takes no config — bindings register with an empty
                object.
              </Note>
            ) : (
              <JsonHighlight
                code={pretty(detail.data.configuration_schema)}
                className="console-catalog-json"
                wrap
              />
            )}
          </TabsContent>
          <TabsContent value="payload">
            {detail.data.request_schema === undefined ? (
              <Note>this type publishes no payload schema.</Note>
            ) : (
              <JsonHighlight
                code={pretty(detail.data.request_schema)}
                className="console-catalog-json"
                wrap
              />
            )}
          </TabsContent>
        </Tabs>
      )}
    </>
  )
}

/** The engine sends `config_summary` as a JSON STRING — parse it so the
 * fallback renders as structured JSON, not one quoted escaped line. */
function parsedSummary(raw: string | null | undefined): unknown {
  if (!raw) return undefined
  try {
    return JSON.parse(raw)
  } catch {
    return raw
  }
}

function BindingDetailPane({
  host,
  binding,
  description,
  onClose,
}: {
  host: Host
  binding: RegisteredTrigger
  description?: string
  onClose: () => void
}) {
  // The payload schema belongs to the TYPE, so a direct call opens on the
  // shape this binding's function actually receives when the trigger fires.
  const load = useCallback(
    () => triggerTypeInfo(host, binding.trigger_type),
    [host, binding.trigger_type],
  )
  const type = useResource(load)

  const spec = familyOf(binding.trigger_type)
  const http = httpBinding(binding)
  const topic = queueTopic(binding)
  const chips = configChips(binding)

  // A title must read as a name. summarize() avoids raw JSON already, but if
  // a config defeats it the type id is the honest fallback.
  const title = (() => {
    const s = summarize(binding)
    return s.startsWith('{') ? binding.trigger_type : s
  })()

  return (
    <>
      <DetailHead
        title={title}
        subtitle={
          <>
            <Chip k={spec.label} v={binding.trigger_type} tone={spec.tone} />
            <Chip k="worker" v={binding.worker_name} />
            {chips.map((chip) => (
              <Chip key={chip.label} k={chip.label} v={chip.value} />
            ))}
          </>
        }
        onClose={onClose}
      >
        <CopyButton value={binding.id} label="copy id" />
      </DetailHead>

      <div className="console-catalog-tabs">
        <FamilyFacts binding={binding} />

        <div className="console-catalog-target">
          <span className="console-catalog-field-label">target function</span>
          <code>{binding.function_id || '(none)'}</code>
          {description ? (
            <span className="console-catalog-desc">{description}</span>
          ) : null}
        </div>

        <Tabs defaultValue="fire">
          <TabsList>
            <TabsTrigger value="fire">
              {spec.family === 'http'
                ? 'send request'
                : spec.family === 'queue'
                  ? 'publish'
                  : spec.family === 'cron'
                    ? 'run now'
                    : 'call target'}
            </TabsTrigger>
            <TabsTrigger value="config">config</TabsTrigger>
          </TabsList>

          <TabsContent value="fire">
            {http ? (
              <HttpTester host={host} binding={http} />
            ) : topic ? (
              <QueuePublish host={host} topic={topic} />
            ) : binding.function_id ? (
              <InvokePanel
                host={host}
                functionId={binding.function_id}
                requestSchema={type.data?.request_schema}
                label={spec.family === 'cron' ? 'run now' : 'call'}
                runningLabel={spec.family === 'cron' ? 'running…' : 'calling…'}
                hint={
                  spec.family === 'cron'
                    ? `calls ${binding.function_id} with a cron-shaped payload. The schedule is untouched, and the next scheduled firing still happens.`
                    : `calls ${binding.function_id} directly with this payload. The trigger itself is not fired, so its config filters do not apply.`
                }
              />
            ) : (
              <Note>
                this binding carries no target function — nothing to call.
              </Note>
            )}
          </TabsContent>

          <TabsContent value="config">
            <JsonHighlight
              code={pretty(
                binding.config ?? parsedSummary(binding.config_summary) ?? {},
              )}
              className="console-catalog-json"
              wrap
            />
          </TabsContent>
        </Tabs>
      </div>
    </>
  )
}

/** The stat tiles that only make sense for a given family. */
function FamilyFacts({ binding }: { binding: RegisteredTrigger }) {
  const expression = cronExpression(binding)
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
          <StatTile label="status" value="scheduled" tone="ok" />
        </div>
      </>
    )
  }

  const http = httpBinding(binding)
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

  const topic = queueTopic(binding)
  if (topic) {
    return (
      <div className="console-catalog-tiles">
        <StatTile label="topic" value={topic} />
        <StatTile label="status" value="consuming" tone="ok" />
      </div>
    )
  }
  return null
}
