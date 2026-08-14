import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { onHarnessConfigSaved } from '@/lib/harness-config-events'
import { getIiiClient } from '@/lib/iii-client'
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
 * `routerAvailable` gates the router-owned RPCs on the llm-router worker
 * being connected — every read here is `router::*`. When it flips false→true
 * (router installed, restarted, or just slow to boot) every effect below
 * re-runs, so the picker recovers without a page reload. A WebSocket
 * reconnect re-pulls both reads for the same reason.
 */
export function useModelPickerSource(
  backendId: string,
  routerAvailable = true,
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
  // Mirror of `presentProviders` for event handlers: React state updaters may
  // run deferred, so membership checks must not live inside them.
  const providersRef = useRef<ProviderListEntry[]>([])
  useEffect(() => {
    providersRef.current = presentProviders
  }, [presentProviders])
  const [catalogLoading, setCatalogLoading] = useState(
    backendId === 'real' && routerAvailable,
  )

  const refresh = useCallback(async () => {
    if (backendId !== 'real') {
      setModelOptions([])
      setCatalogLoading(false)
      return
    }
    if (!routerAvailable) {
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
  }, [backendId, routerAvailable])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // Re-read `router::provider::list`, dropping the result if a newer provider
  // event (or a newer snapshot) has advanced the version since we started.
  const refreshProviders = useCallback(async () => {
    if (backendId !== 'real' || !routerAvailable) {
      setPresentProviders([])
      return
    }
    const snapshotVersion = providerEventVersion.current
    try {
      const providers = await fetchProviderList()
      if (providerEventVersion.current === snapshotVersion) {
        setPresentProviders(providers)
      }
    } catch {
      if (providerEventVersion.current === snapshotVersion) {
        setPresentProviders([])
      }
    }
  }, [backendId, routerAvailable])

  // Initial snapshot (re-run when the router (re)appears). Availability flips
  // are applied from `router::provider::changed`; an event for a provider the
  // snapshot has never seen triggers a full re-read instead, so late-arriving
  // providers render with their declared display name and capabilities.
  useEffect(() => {
    void refreshProviders()
  }, [refreshProviders])

  // Live updates: re-pull the catalog when the router signals a model change
  // (provider configured/cleared, refresh_models, CLI edits). The router
  // coalesces bursts; the short trailing debounce here collapses any remaining
  // back-to-back pushes into a single re-read.
  useEffect(() => {
    if (backendId !== 'real' || !routerAvailable) return
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

    void subscribeModelChanges(onModelsChanged)
      .then((dispose) => {
        if (disposed) dispose()
        else disposers.push(dispose)
      })
      // Setup failure degrades to manual refresh; never an unhandled rejection.
      .catch(() => {})

    void subscribeProviderChanges(({ provider, op }) => {
      providerEventVersion.current += 1
      if (op === 'unregister') {
        setPresentProviders((current) =>
          current.filter((entry) => entry.id !== provider),
        )
        return
      }
      // A provider the snapshot never saw: re-read the list rather than
      // inventing a degraded entry (raw id as display name, guessed
      // capabilities).
      if (!providersRef.current.some((entry) => entry.id === provider)) {
        void refreshProviders()
        return
      }
      const available = op !== 'unavailable'
      setPresentProviders((current) =>
        current.map((entry) =>
          entry.id === provider && entry.available !== available
            ? { ...entry, available }
            : entry,
        ),
      )
    })
      .then((dispose) => {
        if (disposed) dispose()
        else disposers.push(dispose)
      })
      // Setup failure degrades to snapshot re-reads; never an unhandled rejection.
      .catch(() => {})

    return () => {
      disposed = true
      if (timer !== null) clearTimeout(timer)
      for (const d of disposers) d()
    }
  }, [backendId, routerAvailable, refresh, refreshProviders])

  // A WebSocket drop loses any change events fired while disconnected;
  // re-pull both reads when the connection comes back.
  useEffect(() => {
    if (backendId !== 'real' || !routerAvailable) return
    let disposed = false
    let offConn: (() => void) | null = null
    getIiiClient()
      .then((client) => {
        if (disposed) return
        offConn = client.addConnectionStateListener((state) => {
          if (state === 'connected') {
            void refresh()
            void refreshProviders()
          }
        })
      })
      .catch(() => {})
    return () => {
      disposed = true
      offConn?.()
    }
  }, [backendId, routerAvailable, refresh, refreshProviders])

  useEffect(() => {
    if (backendId !== 'real' || !routerAvailable) return
    return onHarnessConfigSaved(() => {
      void refresh()
    })
  }, [backendId, routerAvailable, refresh])

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
