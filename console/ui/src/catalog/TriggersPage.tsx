/**
 * The Triggers page (`#/ext/triggers`): every trigger type published on the
 * bus, each with its live bindings underneath.
 *
 * Two lists make one view. `engine::triggers::list` is the catalogue of
 * TYPES (what can fire); `engine::registered-triggers::list` is the set of
 * live BINDINGS (what will fire, and into which function). A type with no
 * bindings still lists — knowing a type exists is half of what the page is
 * for — and a binding whose type is not in the catalogue lists under its own
 * heading rather than disappearing.
 *
 * Selecting a type shows its config and payload schemas
 * (`engine::triggers::info`, fetched per selection). Selecting a binding
 * shows its config and lets the operator call the bound function with a
 * payload shaped like the one the trigger delivers. There is no engine call
 * that synthesizes a firing, so the page says what it is doing: it invokes
 * the target function directly.
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
  listRegisteredTriggers,
  listTriggerTypes,
  type RegisteredTrigger,
  type TriggerTypeSummary,
  triggerTypeInfo,
  useFleetChanges,
  useResource,
} from './engine'
import { InvokePanel } from './InvokePanel'
import { pretty } from './schema'
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

type Selection =
  | { kind: 'type'; id: string }
  | { kind: 'binding'; binding: RegisteredTrigger }

interface TypeGroup {
  type: TriggerTypeSummary
  bindings: RegisteredTrigger[]
}

export function TriggersPage({ host }: { host: Host }) {
  const [showInternal, setShowInternal] = useState(false)
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<Selection | null>(null)

  // Both lists are independent reads; the page needs them together, so it
  // pays for one round trip, not two.
  const loadCatalog = useCallback(async () => {
    const [types, bindings] = await Promise.all([
      listTriggerTypes(host, { includeInternal: showInternal }),
      listRegisteredTriggers(host, { includeInternal: showInternal }),
    ])
    return { types, bindings }
  }, [host, showInternal])
  const catalog = useResource(loadCatalog)
  useFleetChanges(host, catalog.reload)

  const groups = useMemo<TypeGroup[]>(() => {
    if (!catalog.data) return []
    const byType = new Map<string, RegisteredTrigger[]>()
    for (const binding of catalog.data.bindings) {
      const bucket = byType.get(binding.trigger_type)
      if (bucket) bucket.push(binding)
      else byType.set(binding.trigger_type, [binding])
    }

    const known = new Map(catalog.data.types.map((t) => [t.id, t]))
    // A binding whose type the catalogue does not carry still needs a home:
    // synthesize a heading for it rather than dropping the row.
    for (const type of byType.keys()) {
      if (!known.has(type)) {
        known.set(type, { id: type, worker_name: 'unknown', description: null })
      }
    }

    const needle = search.trim().toLowerCase()
    const matches = (group: TypeGroup) => {
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
          (b.config_summary ?? '').toLowerCase().includes(needle),
      )
    }

    return [...known.values()]
      .map((type) => ({
        type,
        bindings: (byType.get(type.id) ?? []).sort((a, b) =>
          a.function_id.localeCompare(b.function_id),
        ),
      }))
      .filter(matches)
      .sort((a, b) => a.type.id.localeCompare(b.type.id))
  }, [catalog.data, search])

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
      head={
        <CatalogHead
          title="triggers"
          count={`${groups.length} types · ${boundCount} bound`}
          search={search}
          onSearch={setSearch}
          searchPlaceholder="search types, functions, config…"
          onRefresh={catalog.reload}
          loading={catalog.loading}
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
            title={search.trim() ? 'nothing matches' : 'no trigger types'}
            description={
              search.trim()
                ? 'no type, bound function, or config contains that text.'
                : 'workers publish their trigger types on connect — start one and it lists here.'
            }
          />
        ) : (
          groups.map((group) => (
            <div key={group.type.id} className="console-catalog-section">
              <GroupHeader
                label={group.type.id}
                meta={
                  group.bindings.length === 0
                    ? `${group.type.worker_name} · unbound`
                    : `${group.type.worker_name} · ${group.bindings.length}`
                }
                open={groupState.isOpen(group.type.id)}
                onToggle={() => groupState.toggle(group.type.id)}
              />
              {groupState.isOpen(group.type.id) ? (
                <>
                  <CatalogRow
                    primary="type detail"
                    secondary={group.type.description ?? undefined}
                    selected={
                      selected?.kind === 'type' && selected.id === group.type.id
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
                      primary={binding.function_id || '(no target function)'}
                      secondary={binding.config_summary}
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
                  ))}
                </>
              ) : null}
            </div>
          ))
        )
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
                <Chip k="bound" v={String(detail.data.instance_count)} />
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

function BindingDetailPane({
  host,
  binding,
  onClose,
}: {
  host: Host
  binding: RegisteredTrigger
  onClose: () => void
}) {
  // The payload schema belongs to the TYPE, so the call panel opens on the
  // shape this binding's function actually receives when the trigger fires.
  const load = useCallback(
    () => triggerTypeInfo(host, binding.trigger_type),
    [host, binding.trigger_type],
  )
  const type = useResource(load)

  return (
    <>
      <DetailHead
        title={binding.function_id || '(no target function)'}
        subtitle={
          <>
            <Chip k="type" v={binding.trigger_type} />
            <Chip k="worker" v={binding.worker_name} />
            <span className="console-catalog-id">{binding.id}</span>
          </>
        }
        onClose={onClose}
      />
      <Tabs defaultValue="config" className="console-catalog-tabs">
        <TabsList>
          <TabsTrigger value="config">config</TabsTrigger>
          <TabsTrigger value="call">
            call target
            {binding.function_id ? null : <Badge variant="warn">n/a</Badge>}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="config">
          <JsonHighlight
            code={pretty(binding.config ?? binding.config_summary ?? {})}
            className="console-catalog-json"
            wrap
          />
        </TabsContent>
        <TabsContent value="call">
          {binding.function_id ? (
            <InvokePanel
              host={host}
              functionId={binding.function_id}
              requestSchema={type.data?.request_schema}
              label="call"
              runningLabel="calling…"
              hint={`calls ${binding.function_id} directly with this payload. The trigger itself is not fired, so its config filters do not apply.`}
            />
          ) : (
            <Note>
              this binding carries no target function — nothing to call.
            </Note>
          )}
        </TabsContent>
      </Tabs>
    </>
  )
}
