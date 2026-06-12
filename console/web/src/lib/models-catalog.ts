import { makeCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import type { ModelOption } from '@/types/chat'

/** Wire shape returned by `router::models::list` over the iii bus. */
export interface CatalogModelRow {
  id: string
  provider: string
  display_name: string
  context_window?: number
}

export async function fetchModelsCatalog(): Promise<CatalogModelRow[]> {
  const client = await getIiiClient()
  const res = await client.call<{ models?: unknown }>('router::models::list', {})
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
    if (!id || !provider) continue
    out.push({ id, provider, display_name, context_window })
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
      client.call(`provider::${p}::refresh_models`, {}).catch(() => undefined),
    ),
  )
}

/**
 * Subscribe to the harness `ui::models::changed` fanout so the picker can
 * re-pull the catalog the instant a provider's models change — credential
 * added/removed, `refresh_models`, or a provider added/removed via the CLI.
 *
 * Registers a browser-local handler plus `ui::models::subscribe` (separate
 * from the session subscription so the agent-events pump can't evict it) so
 * the harness `models-changed` pump targets this browser. Returns a disposer
 * that unregisters the handler and drops the subscription.
 */
export async function subscribeModelChanges(
  onChange: () => void,
): Promise<() => void> {
  const client = await getIiiClient()
  await client.call('ui::models::subscribe', {
    browser_id: client.browserId,
  })
  const offHandler = client.on('ui::models::changed', () => {
    onChange()
  })
  let disposed = false
  return () => {
    if (disposed) return
    disposed = true
    offHandler()
    void client
      .call('ui::models::unsubscribe', {
        browser_id: client.browserId,
      })
      .catch(() => {})
  }
}

/** A provider declared to the harness, from `router::provider::list`. */
export interface ProviderListEntry {
  id: string
  display_name: string
  supports_model_listing: boolean
}

/**
 * List providers present as harness workers (regardless of whether they have
 * a credential). Used so the picker can surface a present-but-unconfigured
 * provider with a gear that opens its harness configuration.
 */
export async function fetchProviderList(): Promise<ProviderListEntry[]> {
  const client = await getIiiClient()
  const res = await client.call<{ providers?: unknown }>(
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
    })
  }
  return out
}
