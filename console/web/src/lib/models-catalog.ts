import { makeCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import type { ModelOption, ReasoningEffortOption } from '@/types/chat'

/** Wire shape returned by `router::models::list` over the iii bus. */
export interface CatalogModelRow {
  id: string
  provider: string
  display_name: string
  context_window?: number
  supports_thinking?: boolean
  reasoning_efforts?: ReasoningEffortOption[]
}

function parseReasoningEfforts(
  value: unknown,
): ReasoningEffortOption[] | undefined {
  if (!Array.isArray(value)) return undefined
  const seen = new Set<string>()
  const efforts: ReasoningEffortOption[] = []
  for (const raw of value) {
    if (!raw || typeof raw !== 'object') continue
    const row = raw as Record<string, unknown>
    const effort = typeof row.effort === 'string' ? row.effort.trim() : ''
    if (!effort || seen.has(effort)) continue
    seen.add(effort)
    const description =
      typeof row.description === 'string' && row.description.trim()
        ? row.description.trim()
        : undefined
    efforts.push({ effort, description })
  }
  return efforts.length > 0 ? efforts : undefined
}

export async function fetchModelsCatalog(): Promise<CatalogModelRow[]> {
  const client = await getIiiClient()
  const res = await client.trigger<{ models?: unknown }>(
    'router::models::list',
    {},
  )
  const rows = res?.models
  if (!Array.isArray(rows)) return []
  const out: CatalogModelRow[] = []
  for (const raw of rows) {
    if (!raw || typeof raw !== 'object') continue
    const o = raw as Record<string, unknown>
    const id = typeof o.id === 'string' ? o.id : ''
    const provider = typeof o.provider === 'string' ? o.provider : ''
    const display_name =
      typeof o.display_name === 'string' ? o.display_name : id
    const context_window =
      typeof o.context_window === 'number' && o.context_window > 0
        ? o.context_window
        : undefined
    const supports_thinking =
      typeof o.supports_thinking === 'boolean' ? o.supports_thinking : undefined
    const reasoning_efforts = parseReasoningEfforts(o.reasoning_efforts)
    if (!id || !provider) continue
    out.push({
      id,
      provider,
      display_name,
      context_window,
      supports_thinking,
      reasoning_efforts,
    })
  }
  return out
}

export function catalogRowsToModelOptions(
  rows: CatalogModelRow[],
): ModelOption[] {
  const sorted = [...rows].sort((a, b) =>
    a.display_name.localeCompare(b.display_name),
  )
  return sorted.map((m) => ({
    id: makeCatalogModelKey(m.provider, m.id),
    label: m.display_name.toLowerCase(),
    contextWindow: m.context_window,
    supportsThinking: m.supports_thinking === true,
    reasoningEfforts: m.reasoning_efforts,
  }))
}

/**
 * Ask each provider to re-pull its upstream model list into the catalog via
 * `provider::<id>::refresh_models`. Best-effort and parallel — a provider
 * that's offline or has no credential simply registers nothing. Callers
 * re-read `router::models::list` afterwards to pick up the refreshed catalog.
 */
export async function refreshProviderModels(
  providers: readonly string[],
): Promise<void> {
  const client = await getIiiClient()
  await Promise.allSettled(
    providers.map((p) =>
      client
        .trigger(`provider::${p}::refresh_models`, {})
        .catch(() => undefined),
    ),
  )
}

/** iii:: prefix → engine-internal → delivery spans stay out of the Traces view. */
const MODELS_CHANGED_FN = 'iii::console::models_changed'
/** llm-router custom trigger type (worker-owned fan-out, not pubsub). */
const MODELS_CHANGED_TRIGGER = 'router::models::changed'
const PROVIDERS_CHANGED_FN = 'iii::console::providers_changed'
/** Fired on provider availability flips (worker registered / dispatch found it gone). */
const PROVIDERS_CHANGED_TRIGGER = 'router::provider::changed'

async function subscribeRouterTrigger(
  fnId: string,
  triggerType: string,
  onChange: (payload: unknown) => void,
): Promise<() => void> {
  const client = await getIiiClient()
  let offHandler: (() => void) | undefined
  let offTrigger: (() => void) | undefined
  try {
    // `on()` registers `<fn>::<browserId>`; the trigger targets the same id.
    offHandler = client.on(fnId, (payload) => {
      onChange(payload)
    })
    offTrigger = client.registerTrigger({
      type: triggerType,
      function_id: `${fnId}::${client.browserId}`,
      config: {},
    })
  } catch {
    offTrigger?.()
    offHandler?.()
    return () => {}
  }
  let disposed = false
  return () => {
    if (disposed) return
    disposed = true
    offHandler?.()
    try {
      offTrigger?.()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

/**
 * Subscribe to the llm-router `router::models::changed` trigger so the picker
 * re-pulls the catalog when a provider reconciles — credential added/removed,
 * `refresh_models`, or a provider worker added/removed.
 *
 * Registers a browser-local handler plus the router-owned trigger type
 * (`llm-router/README.md` § Events). Returns a disposer; on failure (e.g.
 * llm-router absent) the binding is dropped silently and callers rely on
 * manual refresh / config-save hooks.
 */
export async function subscribeModelChanges(
  onChange: () => void,
): Promise<() => void> {
  return subscribeRouterTrigger(MODELS_CHANGED_FN, MODELS_CHANGED_TRIGGER, () =>
    onChange(),
  )
}

export interface ProviderChangedEvent {
  provider: string
  op: string
}

/**
 * Subscribe to `router::provider::changed` — availability flips (a provider
 * worker (re)registered, or a dispatch discovered it gone). Model catalogs
 * survive a provider going down, so this trigger updates the existing
 * provider snapshot and greys/un-greys the group without another list request.
 */
export async function subscribeProviderChanges(
  onChange: (event: ProviderChangedEvent) => void,
): Promise<() => void> {
  return subscribeRouterTrigger(
    PROVIDERS_CHANGED_FN,
    PROVIDERS_CHANGED_TRIGGER,
    (payload) => {
      if (!payload || typeof payload !== 'object') return
      const row = payload as Record<string, unknown>
      if (typeof row.provider !== 'string' || typeof row.op !== 'string') return
      onChange({ provider: row.provider, op: row.op })
    },
  )
}

/** A provider declared to the router, from `router::provider::list`. */
export interface ProviderListEntry {
  id: string
  display_name: string
  supports_model_listing: boolean
  /**
   * The provider WORKER is loaded/connected. `false` = the router knows the
   * provider (its models may still sit in the catalog) but dispatching to it
   * fails with `router/provider_unavailable` — the picker renders the group
   * as "not loaded" and disables its models.
   */
  available: boolean
}

/**
 * List providers present as worker processes (regardless of whether they have
 * a credential). Used so the picker can surface a present-but-unconfigured
 * provider with a gear that opens its configuration.
 */
export async function fetchProviderList(): Promise<ProviderListEntry[]> {
  const client = await getIiiClient()
  const res = await client.trigger<{ providers?: unknown }>(
    'router::provider::list',
    {},
  )
  const rows = res?.providers
  if (!Array.isArray(rows)) return []
  const out: ProviderListEntry[] = []
  for (const raw of rows) {
    if (!raw || typeof raw !== 'object') continue
    const o = raw as Record<string, unknown>
    const id = typeof o.id === 'string' ? o.id : ''
    if (!id) continue
    out.push({
      id,
      display_name: typeof o.display_name === 'string' ? o.display_name : id,
      supports_model_listing: o.supports_model_listing === true,
      // Absent on older routers — treat as available (previous behavior).
      available: o.available !== false,
    })
  }
  return out
}
