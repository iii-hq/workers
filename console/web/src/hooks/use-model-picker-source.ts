import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { onHarnessConfigSaved } from '@/lib/harness-config-events'
import {
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
 * shows present-but-unconfigured providers as gear groups). When the engine is
 * unreachable (catalog fetch throws) the list stays empty.
 *
 * `presentProviders` is seeded by one `router::provider::list` read, then kept
 * current by provider lifecycle events. Catalog refreshes never poll the
 * provider list.
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
  const providerEventVersion = useRef(0)
  const [catalogLoading, setCatalogLoading] = useState(
    backendId === 'real' && harnessAvailable,
  )

  const refresh = useCallback(async () => {
    if (backendId !== 'real') {
      setModelOptions([])
      setCatalogLoading(false)
      return
    }
    if (!harnessAvailable) {
      setModelOptions([])
      setCatalogLoading(false)
      return
    }
    setCatalogLoading(true)
    try {
      const rows = await fetchModelsCatalog()
      setModelOptions(catalogRowsToModelOptions(rows))
    } catch {
      setModelOptions([])
    } finally {
      setCatalogLoading(false)
    }
  }, [backendId, harnessAvailable])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // One initial snapshot. Subsequent availability changes are applied from
  // `router::provider::changed`, so model refreshes never re-read this list.
  useEffect(() => {
    if (backendId !== 'real' || !harnessAvailable) {
      setPresentProviders([])
      return
    }
    let cancelled = false
    const snapshotVersion = providerEventVersion.current
    void fetchProviderList()
      .then((providers) => {
        if (!cancelled && providerEventVersion.current === snapshotVersion) {
          setPresentProviders(providers)
        }
      })
      .catch(() => {
        if (!cancelled && providerEventVersion.current === snapshotVersion) {
          setPresentProviders([])
        }
      })
    return () => {
      cancelled = true
    }
  }, [backendId, harnessAvailable])

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
      providerEventVersion.current += 1
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
