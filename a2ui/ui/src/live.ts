import { useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'
import type { JsonValue, LiveBinding, SurfaceRecord } from './types'
import { getPath } from './bindings'
import { applyBinding } from './data'

const EVENTS_FN = 'iii::a2ui-ui::events'
const BINDING_DEBOUNCE_MS = 120
const SURFACE_REFRESH_DEBOUNCE_MS = 50
const MAX_LIVE_VALUE_BYTES = 512 * 1024

interface StateEvent {
  type: 'state'
  event_type: 'state:created' | 'state:updated' | 'state:deleted'
  scope: string
  key: string
}

type ValueListener = (path: string, value: JsonValue, revision: number) => void

interface BindingEntry {
  listeners: Set<ValueListener>
  offHandler: () => void
  offTrigger: () => void
  timer: ReturnType<typeof setTimeout> | null
  pending: JsonValue | undefined
  hasPending: boolean
  inFlight: boolean
  closed: boolean
}

interface SurfaceEventEntry {
  listeners: Set<() => void>
  offHandler: () => void
  offTrigger: () => void
  timer: ReturnType<typeof setTimeout> | null
}

interface SharedRegistries {
  bindings: WeakMap<object, Map<string, BindingEntry>>
  surfaceEvents: WeakMap<object, Map<string, SurfaceEventEntry>>
}

const REGISTRIES_KEY = Symbol.for('iii.a2ui.live-registries.v1')
const registryHost = globalThis as unknown as Record<PropertyKey, unknown>
const sharedRegistries = (() => {
  const existing = registryHost[REGISTRIES_KEY] as SharedRegistries | undefined
  if (existing) return existing
  const created: SharedRegistries = {
    bindings: new WeakMap(),
    surfaceEvents: new WeakMap(),
  }
  registryHost[REGISTRIES_KEY] = created
  return created
})()

function registryFor<T>(registries: WeakMap<object, Map<string, T>>, host: Host): Map<string, T> {
  const key = host.iii as unknown as object
  let registry = registries.get(key)
  if (!registry) {
    registry = new Map()
    registries.set(key, registry)
  }
  return registry
}

function bindingKey(surface: SurfaceRecord, binding: LiveBinding): string {
  return JSON.stringify([
    surface.session_id,
    surface.surface_id,
    binding.id,
    binding.trigger_type,
    binding.config,
    binding.target_path,
    binding.event_path ?? null,
  ])
}

function localBindingId(): string {
  return `iii::a2ui-binding::${globalThis.crypto.randomUUID()}`
}

function withinLiveValueLimit(value: JsonValue): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).byteLength <= MAX_LIVE_VALUE_BYTES
  } catch {
    return false
  }
}

export function subscribeLiveBinding(
  host: Host,
  surface: SurfaceRecord,
  binding: LiveBinding,
  listener: ValueListener,
): () => void {
  const registry = registryFor(sharedRegistries.bindings, host)
  const key = bindingKey(surface, binding)
  const existing = registry.get(key)
  if (existing) {
    existing.listeners.add(listener)
    return () => releaseBinding(registry, key, existing, listener)
  }

  const localId = localBindingId()
  const entry: BindingEntry = {
    listeners: new Set([listener]),
    offHandler: () => {},
    offTrigger: () => {},
    timer: null,
    pending: undefined,
    hasPending: false,
    inFlight: false,
    closed: false,
  }

  const schedule = () => {
    if (entry.closed || entry.timer) return
    entry.timer = setTimeout(() => {
      entry.timer = null
      void flushBinding(host, surface, binding, entry, schedule)
    }, BINDING_DEBOUNCE_MS)
  }

  try {
    entry.offHandler = host.iii.on(localId, (event: unknown) => {
      void (async () => {
        try {
          const payload = binding.trigger_type === 'state'
            ? await host.iii.trigger('state::get', binding.config as Record<string, unknown>)
            : event
          const value = binding.event_path
            ? getPath(payload as JsonValue, binding.event_path)
            : payload
          if (value === undefined || !withinLiveValueLimit(value as JsonValue)) return
          entry.pending = value as JsonValue
          entry.hasPending = true
          schedule()
        } catch {
          return
        }
      })()
    })
    entry.offTrigger = host.iii.registerTrigger({
      type: binding.trigger_type,
      function_id: `${localId}::${host.iii.browserId}`,
      config: binding.config as Record<string, unknown>,
    })
  } catch {
    entry.offHandler()
    return () => {}
  }
  registry.set(key, entry)
  return () => releaseBinding(registry, key, entry, listener)
}

async function flushBinding(
  host: Host,
  surface: SurfaceRecord,
  binding: LiveBinding,
  entry: BindingEntry,
  schedule: () => void,
): Promise<void> {
  if (entry.closed || entry.inFlight || !entry.hasPending) {
    if (entry.hasPending) schedule()
    return
  }
  const value = entry.pending as JsonValue
  entry.pending = undefined
  entry.hasPending = false
  entry.inFlight = true
  try {
    const receipt = await applyBinding(
      host,
      surface.session_id,
      surface.surface_id,
      binding.id,
      value,
    )
    for (const listener of entry.listeners) {
      listener(binding.target_path, value, receipt.revision)
    }
  } catch {
    return
  } finally {
    entry.inFlight = false
    if (entry.hasPending) schedule()
  }
}

function releaseBinding(
  registry: Map<string, BindingEntry>,
  key: string,
  entry: BindingEntry,
  listener: ValueListener,
): void {
  entry.listeners.delete(listener)
  if (entry.listeners.size > 0) return
  entry.closed = true
  if (entry.timer) clearTimeout(entry.timer)
  entry.offTrigger()
  entry.offHandler()
  registry.delete(key)
}

export function subscribeSurfaceEvents(
  host: Host,
  sessionId: string,
  listener: () => void,
): () => void {
  const registry = registryFor(sharedRegistries.surfaceEvents, host)
  const existing = registry.get(sessionId)
  if (existing) {
    existing.listeners.add(listener)
    return () => releaseSurfaceEvents(registry, sessionId, existing, listener)
  }
  const entry: SurfaceEventEntry = {
    listeners: new Set([listener]),
    offHandler: () => {},
    offTrigger: () => {},
    timer: null,
  }
  try {
    entry.offHandler = host.iii.on<StateEvent>(EVENTS_FN, (event) => {
      if (event?.type !== 'state' || event.scope !== 'a2ui' || event.key !== sessionId) return
      if (entry.timer) return
      entry.timer = setTimeout(() => {
        entry.timer = null
        for (const current of entry.listeners) current()
      }, SURFACE_REFRESH_DEBOUNCE_MS)
    })
    entry.offTrigger = host.iii.registerTrigger({
      type: 'state',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: { scope: 'a2ui', key: sessionId },
    })
  } catch {
    entry.offHandler()
    return () => {}
  }
  registry.set(sessionId, entry)
  return () => releaseSurfaceEvents(registry, sessionId, entry, listener)
}

function releaseSurfaceEvents(
  registry: Map<string, SurfaceEventEntry>,
  sessionId: string,
  entry: SurfaceEventEntry,
  listener: () => void,
): void {
  entry.listeners.delete(listener)
  if (entry.listeners.size > 0) return
  if (entry.timer) clearTimeout(entry.timer)
  entry.offTrigger()
  entry.offHandler()
  registry.delete(sessionId)
}

export function useLiveBindings(
  host: Host,
  surface: SurfaceRecord,
  onValue: ValueListener,
): void {
  const handler = useRef(onValue)
  handler.current = onValue
  useEffect(() => {
    const offs = (surface.bindings ?? []).map((binding) =>
      subscribeLiveBinding(host, surface, binding, (path, value, revision) =>
        handler.current(path, value, revision),
      ),
    )
    return () => { for (const off of offs) off() }
  }, [host, surface.session_id, surface.surface_id, surface.bindings])
}

export function useSurfaceEvents(
  host: Host,
  sessionId: string | null | undefined,
  onEvent: () => void,
): void {
  const handler = useRef(onEvent)
  handler.current = onEvent
  useEffect(() => {
    if (!sessionId) return
    return subscribeSurfaceEvents(host, sessionId, () => handler.current())
  }, [host, sessionId])
}
