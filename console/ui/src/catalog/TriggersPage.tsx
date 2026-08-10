/**
 * The Triggers page (`#/ext/triggers`): the same sidebar + workspace shell
 * as the functions page. The sidebar lists every trigger type with its live
 * bindings indented beneath it, each row led by the family's glyph (globe,
 * clock, layers…); the workspace is always present — a hero when nothing is
 * selected, the type or binding document when something is.
 *
 * Two lists make one view. `engine::triggers::list` is the catalogue of
 * TYPES (what can fire); `engine::registered-triggers::list` is the set of
 * live REGISTERED TRIGGERS (what will fire, and into which function). A type
 * with none still lists — knowing a type exists is half of what the page is
 * for — and a binding whose type is not in the catalogue lists under its own
 * heading rather than disappearing. The type itself is selected by clicking
 * its heading; only bindings get rows.
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
  PageHeader,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { describeCron, nextCronRun, untilLabel } from './cron'
import {
  type FunctionSummary,
  listFunctions,
  listRegisteredTriggers,
  listTriggerTypes,
  type RegisteredTrigger,
  type TriggerTypeDetail,
  type TriggerTypeSummary,
  triggerTypeInfo,
  useLiveSignals,
  useResource,
} from './engine'
import { HttpTester } from './HttpTester'
import { InvokePanel } from './InvokePanel'
import { QueuePublish } from './QueuePublish'
import { SchemaTable } from './SchemaTable'
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
  SearchField,
  SideCount,
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

function BoltIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M8.75 1.5L3.5 9H7l-.75 5.5L11.5 7H8z" />
    </svg>
  )
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

  const familyCounts = useMemo(() => {
    const counts = new Map<Family, number>()
    for (const group of allGroups) {
      const key = familyOf(group.type.id).family
      counts.set(key, (counts.get(key) ?? 0) + 1)
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
        // Bound types stay easy to find without moving around as calls arrive.
        // Live function spans cannot prove which trigger fired, so activity is
        // deliberately not used as a sorting signal here.
        const count = b.bindings.length - a.bindings.length
        if (count !== 0) return count
        return a.type.id.localeCompare(b.type.id)
      })
  }, [allGroups, family, search])

  const boundCount = groups.reduce((n, g) => n + g.bindings.length, 0)
  const totalBindings = allGroups.reduce((n, g) => n + g.bindings.length, 0)

  // Most of the catalogue is unbound types; expanding all of them buries the
  // ones that actually fire, so a type opens by default only when something
  // is bound to it.
  const bindingCounts = useMemo(
    () => new Map(groups.map((g) => [g.type.id, g.bindings.length])),
    [groups],
  )
  const groupState = useGroupToggle((id) => (bindingCounts.get(id) ?? 0) > 0)

  const selectType = (id: string) =>
    setSelected((prev) =>
      prev?.kind === 'type' && prev.id === id ? null : { kind: 'type', id },
    )
  const selectBinding = (binding: RegisteredTrigger) =>
    setSelected((prev) =>
      prev?.kind === 'binding' && prev.binding.id === binding.id
        ? null
        : { kind: 'binding', binding },
    )

  // Keep an open document tied to the live registry. Bindings may be updated
  // in place or disappear when their worker disconnects.
  useEffect(() => {
    if (!catalog.data || !selected) return
    if (selected.kind === 'type') {
      const exists =
        catalog.data.types.some((type) => type.id === selected.id) ||
        catalog.data.bindings.some(
          (binding) => binding.trigger_type === selected.id,
        )
      if (!exists) setSelected(null)
      return
    }
    const current = catalog.data.bindings.find(
      (binding) => binding.id === selected.binding.id,
    )
    if (!current) setSelected(null)
    else if (current !== selected.binding) {
      setSelected({ kind: 'binding', binding: current })
    }
  }, [catalog.data, selected])

  return (
    <CatalogShell
      side={side}
      hasSelection={selected !== null}
      header={
        <PageHeader
          icon={<BoltIcon />}
          title="triggers"
          description={
            <span className="console-catalog-header-desc">
              trigger types and registered bindings
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
              value={search}
              onChange={setSearch}
              placeholder="search triggers…"
            />
            <Button
              variant="pill"
              size="sm"
              type="button"
              className="console-catalog-header-toggle"
              onClick={() => setShowInternal((v) => !v)}
              aria-pressed={showInternal}
              title="include internal trigger registrations"
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
            ? 'loading triggers…'
            : search.trim() || family
              ? `showing ${groups.length} of ${allGroups.length} types · ${boundCount} bindings`
              : `${allGroups.length} types · ${totalBindings} bindings`}
        </SideCount>
      }
      list={
        catalog.error ? (
          <ErrorNote
            title="couldn't load triggers"
            call="engine::triggers::list"
            message={catalog.error}
            onRetry={catalog.reload}
          />
        ) : catalog.data === null ? (
          <CatalogListSkeleton label="loading triggers" />
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
              const spec = familyOf(group.type.id)
              return (
                <div key={group.type.id} className="console-catalog-section">
                  <GroupHeader
                    label={group.type.id}
                    meta={group.type.worker_name}
                    count={group.bindings.length}
                    countLabel="binding"
                    tone={spec.tone}
                    toneLabel={spec.label}
                    open={groupState.isOpen(group.type.id)}
                    onToggle={() => groupState.toggle(group.type.id)}
                    collapsible={group.bindings.length > 0}
                    onSelect={() => selectType(group.type.id)}
                    selected={
                      selected?.kind === 'type' && selected.id === group.type.id
                    }
                  />
                  {groupState.isOpen(group.type.id)
                    ? group.bindings.map((binding) => (
                        <CatalogRow
                          key={binding.id}
                          icon={
                            <FamilyGlyph
                              family={spec.family}
                              tone={spec.tone}
                            />
                          }
                          primary={summarize(binding)}
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
                          onClick={() => selectBinding(binding)}
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
                  meta="console + configuration internals"
                  count={partitioned.plumbing.length}
                  countLabel="binding"
                  open={groupState.isOpen('__plumbing')}
                  onToggle={() => groupState.toggle('__plumbing')}
                />
                {groupState.isOpen('__plumbing')
                  ? partitioned.plumbing.map((binding) => (
                      <CatalogRow
                        key={binding.id}
                        icon={
                          <FamilyGlyph
                            family={familyOf(binding.trigger_type).family}
                            tone="ink"
                          />
                        }
                        primary={summarize(binding)}
                        secondary={`${binding.trigger_type} → ${binding.function_id}`}
                        selected={
                          selected?.kind === 'binding' &&
                          selected.binding.id === binding.id
                        }
                        onClick={() => selectBinding(binding)}
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
            eyebrow="trigger catalog"
            title="understand what starts work"
            body="select a type to inspect the contract it defines, or select a binding nested beneath it to see the live route, schedule, or subscription connected to a function."
            items={[
              {
                label: 'type',
                value: 'the reusable trigger definition and its schemas',
              },
              {
                label: 'binding',
                value: 'a registered route, schedule, topic, or hook',
              },
              {
                label: 'target',
                value: 'the function that receives the trigger payload',
              },
            ]}
          />
        ) : selected.kind === 'type' ? (
          <TypeDocument
            host={host}
            typeId={selected.id}
            onBack={() => setSelected(null)}
          />
        ) : (
          <BindingDocument
            host={host}
            binding={selected.binding}
            description={describeFunction(selected.binding.function_id)}
            onBack={() => setSelected(null)}
          />
        )
      }
    />
  )
}

function TypeDocument({
  host,
  typeId,
  onBack,
}: {
  host: Host
  typeId: string
  onBack: () => void
}) {
  const load = useCallback(() => triggerTypeInfo(host, typeId), [host, typeId])
  const detail = useResource(load)
  const spec = familyOf(typeId)

  return (
    <CatalogWorkspace
      context={detail.data ? <TypeContext detail={detail.data} /> : undefined}
    >
      <div className="console-catalog-doc">
        <Crumb
          trail={['triggers', detail.data?.worker_name, typeId]}
          onBack={onBack}
        />
        <IdentityHead
          glyph={
            <FamilyGlyph family={spec.family} tone={spec.tone} size="lg" />
          }
          title={typeId}
          status="trigger type"
          description={
            detail.data
              ? detail.data.description || 'no description provided.'
              : undefined
          }
          chips={
            detail.data ? (
              <>
                <Chip k={spec.label} v="trigger type" tone={spec.tone} />
                <Chip k="worker" v={detail.data.worker_name} />
                {detail.data.instance_count !== undefined ? (
                  <Chip k="registered" v={String(detail.data.instance_count)} />
                ) : null}
              </>
            ) : null
          }
          actions={<CopyButton value={typeId} label="copy id" />}
        />
        {detail.error ? (
          <ErrorNote
            title="couldn't load trigger type"
            call="engine::triggers::info"
            message={detail.error}
            onRetry={detail.reload}
          />
        ) : detail.data === null ? (
          <Note>loading detail…</Note>
        ) : (
          <Tabs defaultValue="config" className="console-catalog-tabs">
            <TabsList>
              <TabsTrigger value="config">configuration</TabsTrigger>
              <TabsTrigger value="payload">event payload</TabsTrigger>
            </TabsList>
            <TabsContent value="config">
              <SchemaTable
                schema={detail.data.configuration_schema}
                empty="this type takes no config — bindings register with an empty object."
              />
            </TabsContent>
            <TabsContent value="payload">
              <SchemaTable
                schema={detail.data.request_schema}
                empty="this type publishes no payload schema."
              />
            </TabsContent>
          </Tabs>
        )}
      </div>
    </CatalogWorkspace>
  )
}

function TypeContext({ detail }: { detail: TriggerTypeDetail }) {
  const spec = familyOf(detail.id)
  return (
    <>
      <ContextPanel title="trigger type details">
        <Facts
          items={[
            { label: 'type id', value: <code>{detail.id}</code> },
            { label: 'family', value: spec.label },
            { label: 'worker', value: detail.worker_name },
            {
              label: 'registered',
              value: String(detail.instance_count ?? 0),
            },
          ]}
        />
      </ContextPanel>
      <ContextPanel
        title="contracts"
        description="Schemas published by this trigger type."
      >
        <Facts
          items={[
            {
              label: 'configuration',
              value:
                detail.configuration_schema !== undefined ? 'defined' : 'none',
            },
            {
              label: 'event payload',
              value: detail.request_schema !== undefined ? 'defined' : 'none',
            },
          ]}
        />
      </ContextPanel>
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

function BindingDocument({
  host,
  binding,
  description,
  onBack,
}: {
  host: Host
  binding: RegisteredTrigger
  description?: string
  onBack: () => void
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
    <CatalogWorkspace
      context={<BindingContext binding={binding} description={description} />}
    >
      <div className="console-catalog-doc">
        <Crumb
          trail={['triggers', binding.trigger_type, title]}
          onBack={onBack}
        />
        <IdentityHead
          glyph={
            <FamilyGlyph family={spec.family} tone={spec.tone} size="lg" />
          }
          title={title}
          status="registered binding"
          description={
            binding.function_id
              ? `Delivers ${spec.label} events to ${binding.function_id}.`
              : 'This binding has no target function.'
          }
          chips={
            <>
              <Chip k={spec.label} v={binding.trigger_type} tone={spec.tone} />
              <Chip k="worker" v={binding.worker_name} />
              {chips.map((chip) => (
                <Chip key={chip.label} k={chip.label} v={chip.value} />
              ))}
            </>
          }
          actions={<CopyButton value={binding.id} label="copy id" />}
        />

        <div className="console-catalog-tabs">
          {type.error ? (
            <ErrorNote
              title="couldn't load the payload schema"
              call="engine::triggers::info"
              message={type.error}
              onRetry={type.reload}
            />
          ) : null}

          <Tabs defaultValue="fire">
            <TabsList>
              <TabsTrigger value="fire">
                {spec.family === 'http'
                  ? 'send request'
                  : spec.family === 'queue'
                    ? 'publish'
                    : spec.family === 'cron'
                      ? 'run now'
                      : 'trigger function'}
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
                  label={
                    spec.family === 'cron' ? 'run now' : 'trigger function'
                  }
                  runningLabel={
                    spec.family === 'cron' ? 'running…' : 'triggering…'
                  }
                  hint={
                    spec.family === 'cron'
                      ? `Triggers ${binding.function_id} with a cron-shaped payload. The schedule is untouched, and the next scheduled firing still happens.`
                      : `Triggers ${binding.function_id} directly with this payload. The binding itself is bypassed, so its config filters do not apply.`
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
      </div>
    </CatalogWorkspace>
  )
}

function BindingContext({
  binding,
  description,
}: {
  binding: RegisteredTrigger
  description?: string
}) {
  const spec = familyOf(binding.trigger_type)
  const hasDeliveryFacts = Boolean(
    cronExpression(binding) || httpBinding(binding) || queueTopic(binding),
  )
  return (
    <>
      <ContextPanel
        title="target function"
        description="Events from this binding are delivered here."
        wide
      >
        {binding.function_id ? (
          <ContextItem
            glyph={<FnGlyph />}
            title={binding.function_id}
            description={description || 'No function description provided.'}
            meta={binding.worker_name}
          />
        ) : (
          <span className="console-catalog-context-empty">
            No target function is registered.
          </span>
        )}
      </ContextPanel>

      <ContextPanel title="binding details">
        <Facts
          items={[
            { label: 'binding id', value: <code>{binding.id}</code> },
            { label: 'type', value: <code>{binding.trigger_type}</code> },
            { label: 'family', value: spec.label },
            { label: 'worker', value: binding.worker_name },
          ]}
        />
      </ContextPanel>

      {hasDeliveryFacts ? (
        <ContextPanel title="delivery">
          <FamilyFacts binding={binding} />
        </ContextPanel>
      ) : null}
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
