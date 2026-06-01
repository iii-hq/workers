import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  catalogRowsToModelOptions,
  fetchModelsCatalog,
  fetchProviderList,
  type ProviderListEntry,
} from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'
import { STATIC_MODEL_OPTIONS } from '@/types/chat'

/**
 * Populate model picker options from `models::list` when the real backend is
 * active; mock / playground uses `STATIC_MODEL_OPTIONS` only.
 *
 * Models come exclusively from providers, so on the real backend a
 * successful-but-empty catalog yields an empty option list (the picker then
 * shows present-but-unconfigured providers as gear groups). The static
 * fallback is reserved for when the engine is unreachable (the catalog fetch
 * throws) so the picker is never blank in that degraded state.
 *
 * `presentProviders` comes from `harness::provider::list` so the picker can
 * surface providers that exist as workers but aren't configured yet.
 * `refresh()` re-reads both on demand (after a provider `refresh_models`).
 */
export function useModelPickerSource(backendId: string): {
  modelOptions: ModelOption[]
  catalogKeys: string[]
  catalogLoading: boolean
  presentProviders: ProviderListEntry[]
  refresh: () => Promise<void>
} {
  const [modelOptions, setModelOptions] =
    useState<ModelOption[]>(STATIC_MODEL_OPTIONS)
  const [presentProviders, setPresentProviders] = useState<ProviderListEntry[]>(
    [],
  )
  const [catalogLoading, setCatalogLoading] = useState(backendId === 'real')

  const refresh = useCallback(async () => {
    if (backendId !== 'real') {
      setModelOptions(STATIC_MODEL_OPTIONS)
      setPresentProviders([])
      setCatalogLoading(false)
      return
    }
    setCatalogLoading(true)
    try {
      // A provider-list failure shouldn't blank the model list, so it has its
      // own fallback; a models::list throw signals an unreachable engine and
      // drops us to the static fallback below.
      const [rows, providers] = await Promise.all([
        fetchModelsCatalog(),
        fetchProviderList().catch(() => [] as ProviderListEntry[]),
      ])
      setModelOptions(catalogRowsToModelOptions(rows))
      setPresentProviders(providers)
    } catch {
      setModelOptions(STATIC_MODEL_OPTIONS)
      setPresentProviders([])
    } finally {
      setCatalogLoading(false)
    }
  }, [backendId])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const catalogKeys = useMemo(
    () => modelOptions.map((o) => o.id),
    [modelOptions],
  )

  return {
    modelOptions,
    catalogKeys,
    catalogLoading,
    presentProviders,
    refresh,
  }
}
