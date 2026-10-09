import { useCallback, useEffect, useRef, useState } from 'react'
import { onHarnessConfigSaved } from '@/lib/harness-config-events'
import {
  catalogKeysInRouterOrder,
  catalogRowsToModelOptions,
  fetchModelsCatalog,
  fetchProviderList,
  type ProviderListEntry,
  subscribeModelChanges,
  subscribeProviderChanges,
} from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'

/**
 * Populate model picker options from `models::list` when the real backend is
 * active; mock / playground keeps an empty option list.
 *
 * Models come exclusively from providers, so on the real backend a
 * successful-but-empty catalog yields an empty option list (the picker then
 * shows present-but-unconfigured providers as setup rows). When the engine is
 * unreachable (catalog fetch throws) the list stays empty.
 *
 * `presentProviders` is seeded by `router::provider::list`, then kept current
 * by provider/model/configuration events. There is no polling.
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
  // Catalog keys in the router's order (its best model first), unlike the
  // picker's alphabetical list: the fallback for a new chat comes from here.
  const [catalogKeys, setCatalogKeys] = useState<string[]>([])
  const [presentProviders, setPresentProviders] = useState<ProviderListEntry[]>(
    [],
  )
  const providerEventVersion = useRef(0)
  const hasCatalog = useRef(false)
  const [catalogLoading, setCatalogLoading] = useState(
    backendId === 'real' && harnessAvailable,
  )

  const refresh = useCallback(async () => {
    const version = ++providerEventVersion.current
    if (backendId !== 'real') {
      setModelOptions([])
      setCatalogKeys([])
      setCatalogLoading(false)
      return
    }
    if (!harnessAvailable) {
      setPresentProviders([])
      setModelOptions([])
      setCatalogKeys([])
      setCatalogLoading(false)
      return
    }
    // Only the first read shows as loading. A background re-read (a key was
    // stored, a provider registered) keeps the current list on screen: a
    // loading picker is disabled, and a disabled picker closes under the
    // person configuring it.
    if (!hasCatalog.current) setCatalogLoading(true)
    try {
      const [rows, providers] = await Promise.all([
        fetchModelsCatalog(),
        fetchProviderList(),
      ])
      if (version !== providerEventVersion.current) return
      setPresentProviders(providers)
      setModelOptions(catalogRowsToModelOptions(rows))
      setCatalogKeys(catalogKeysInRouterOrder(rows))
      hasCatalog.current = true
    } catch {
      // A transient read failure must not silently replace an existing selection.
      if (!hasCatalog.current) {
        setModelOptions([])
        setCatalogKeys([])
      }
    } finally {
      if (version === providerEventVersion.current) setCatalogLoading(false)
    }
  }, [backendId, harnessAvailable])

  useEffect(() => {
    void refresh()
    return () => {
      providerEventVersion.current += 1
    }
  }, [refresh])

  // Live updates: re-pull the catalog when the harness signals a model change
  // (provider configured/cleared, refresh_models, CLI edits). The harness
  // coalesces bursts; the short trailing debounce here collapses any remaining
  // back-to-back pushes into a single re-read.
  useEffect(() => {
    if (backendId !== 'real' || !harnessAvailable) return
    let disposed = false
    const disposers: (() => void)[] = []
    let timer: ReturnType<typeof setTimeout> | null = null

    const onModelsChanged = () => {
      providerEventVersion.current += 1
      if (timer !== null) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = null
        void refresh()
      }, 150)
    }

    void subscribeModelChanges(onModelsChanged).then((dispose) => {
      if (disposed) dispose()
      else disposers.push(dispose)
    })

    void subscribeProviderChanges(({ provider, op }) => {
      onModelsChanged()
      if (op === 'discovery') return
      setPresentProviders((current) => {
        const available = op !== 'unavailable'
        const existing = current.find((entry) => entry.id === provider)
        if (existing) {
          if (existing.available === available) return current
          return current.map((entry) =>
            entry.id === provider ? { ...entry, available } : entry,
          )
        }
        return [
          ...current,
          {
            id: provider,
            display_name: provider,
            supports_model_listing: true,
            credential_env_var: undefined,
            available,
          },
        ]
      })
    }).then((dispose) => {
      if (disposed) dispose()
      else disposers.push(dispose)
    })

    return () => {
      disposed = true
      if (timer !== null) clearTimeout(timer)
      for (const d of disposers) d()
    }
  }, [backendId, harnessAvailable, refresh])

  useEffect(() => {
    if (backendId !== 'real' || !harnessAvailable) return
    return onHarnessConfigSaved(() => {
      void refresh()
    })
  }, [backendId, harnessAvailable, refresh])

  return {
    modelOptions,
    catalogKeys,
    catalogLoading,
    presentProviders,
    refresh,
  }
}
