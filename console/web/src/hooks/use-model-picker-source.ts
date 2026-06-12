import { useCallback, useEffect, useMemo, useState } from 'react'
import { onHarnessConfigSaved } from '@/lib/harness-config-events'
import {
  catalogRowsToModelOptions,
  fetchModelsCatalog,
  fetchProviderList,
  type ProviderListEntry,
  subscribeModelChanges,
} from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'

/**
 * Populate model picker options from `models::list` when the real backend is
 * active; mock / playground keeps an empty option list.
 *
 * Models come exclusively from providers, so on the real backend a
 * successful-but-empty catalog yields an empty option list (the picker then
 * shows present-but-unconfigured providers as gear groups). When the engine is
 * unreachable (catalog fetch throws) the list stays empty.
 *
 * `presentProviders` comes from `router::provider::list` so the picker can
 * surface providers that exist as workers but aren't configured yet.
 * `refresh()` re-reads both on demand (after a provider `refresh_models`).
 *
 * `harnessAvailable` gates harness-owned RPCs until the worker is connected.
 */
export function useModelPickerSource(
  backendId: string,
  harnessAvailable = true,
): {
  modelOptions: ModelOption[]
  catalogKeys: string[]
  catalogLoading: boolean
  presentProviders: ProviderListEntry[]
  refresh: () => Promise<void>
} {
  const [modelOptions, setModelOptions] = useState<ModelOption[]>([])
  const [presentProviders, setPresentProviders] = useState<ProviderListEntry[]>(
    [],
  )
  const [catalogLoading, setCatalogLoading] = useState(
    backendId === 'real' && harnessAvailable,
  )

  const refresh = useCallback(async () => {
    if (backendId !== 'real') {
      setModelOptions([])
      setPresentProviders([])
      setCatalogLoading(false)
      return
    }
    if (!harnessAvailable) {
      setModelOptions([])
      setPresentProviders([])
      setCatalogLoading(false)
      return
    }
    setCatalogLoading(true)
    try {
      // A provider-list failure shouldn't blank the model list, so it has its
      // own fallback; a models::list throw signals an unreachable engine and
      // leaves the option list empty.
      const [rows, providers] = await Promise.all([
        fetchModelsCatalog(),
        fetchProviderList().catch(() => [] as ProviderListEntry[]),
      ])
      setModelOptions(catalogRowsToModelOptions(rows))
      setPresentProviders(providers)
    } catch {
      setModelOptions([])
      setPresentProviders([])
    } finally {
      setCatalogLoading(false)
    }
  }, [backendId, harnessAvailable])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // Live updates: re-pull the catalog when the harness signals a model change
  // (provider configured/cleared, refresh_models, CLI edits). The harness
  // coalesces bursts; the short trailing debounce here collapses any remaining
  // back-to-back pushes into a single re-read.
  useEffect(() => {
    if (backendId !== 'real' || !harnessAvailable) return
    let disposed = false
    let dispose: (() => void) | undefined
    let timer: ReturnType<typeof setTimeout> | null = null

    const onChange = () => {
      if (timer !== null) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = null
        void refresh()
      }, 150)
    }

    void subscribeModelChanges(onChange).then((d) => {
      if (disposed) {
        d()
        return
      }
      dispose = d
    })

    return () => {
      disposed = true
      if (timer !== null) clearTimeout(timer)
      dispose?.()
    }
  }, [backendId, harnessAvailable, refresh])

  useEffect(() => {
    if (backendId !== 'real' || !harnessAvailable) return
    return onHarnessConfigSaved(() => {
      void refresh()
    })
  }, [backendId, harnessAvailable, refresh])

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
