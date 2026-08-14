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
 * `routerAvailable` gates the router-owned RPCs on the llm-router worker
 * being connected — every read here is `router::*`. When it flips false→true
 * (router installed, restarted, or just slow to boot) every effect below
 * re-runs, so the picker recovers without a page reload. A WebSocket
 * reconnect re-pulls both reads for the same reason.
 */
export function useModelPickerSource(
  backendId: string,
  routerAvailable = true,
  routerRevision = 0,
): {
  modelOptions: ModelOption[]
  catalogKeys: string[]
  catalogLoading: boolean
  presentProviders: ProviderListEntry[]
  refresh: (force?: boolean) => Promise<void>
} {
  const [modelOptions, setModelOptions] = useState<ModelOption[]>([])
  const [presentProviders, setPresentProviders] = useState<ProviderListEntry[]>(
    [],
  )
  const providerEventVersion = useRef(0)
  const catalogRequestVersion = useRef(0)
  const providerRequestVersion = useRef(0)
  const routerRevisionRef = useRef(routerRevision)
  routerRevisionRef.current = routerRevision
  // Mirror of `presentProviders` for event handlers: React state updaters may
  // run deferred, so membership checks must not live inside them.
  const providersRef = useRef<ProviderListEntry[]>([])
  useEffect(() => {
    providersRef.current = presentProviders
  }, [presentProviders])
  const [catalogLoading, setCatalogLoading] = useState(
    backendId === 'real' && routerAvailable,
  )

  const refresh = useCallback(
    async (force = false) => {
      const requestVersion = ++catalogRequestVersion.current
      const revisionAtStart = routerRevision
      if (backendId !== 'real') {
        setModelOptions([])
        setCatalogLoading(false)
        return
      }
      if (!routerAvailable && !force) {
        setModelOptions([])
        setCatalogLoading(false)
        return
      }
      setCatalogLoading(true)
      try {
        const rows = await fetchModelsCatalog()
        if (
          catalogRequestVersion.current === requestVersion &&
          routerRevisionRef.current === revisionAtStart
        ) {
          setModelOptions(catalogRowsToModelOptions(rows))
        }
      } catch {
        // A timeout or a reconnect race is not evidence that the configured
        // catalogue became empty. Preserve the last good snapshot; a successful
        // empty response above still clears it authoritatively.
      } finally {
        if (catalogRequestVersion.current === requestVersion) {
          setCatalogLoading(false)
        }
      }
    },
    [backendId, routerAvailable, routerRevision],
  )

  useEffect(() => {
    void refresh()
  }, [refresh])

  // Re-read `router::provider::list`, dropping the result if a newer provider
  // event (or a newer snapshot) has advanced the version since we started.
  const refreshProviders = useCallback(async () => {
    const requestVersion = ++providerRequestVersion.current
    const revisionAtStart = routerRevision
    if (backendId !== 'real' || !routerAvailable) {
      setPresentProviders([])
      return
    }
    const snapshotVersion = providerEventVersion.current
    try {
      const providers = await fetchProviderList()
      if (
        providerEventVersion.current === snapshotVersion &&
        providerRequestVersion.current === requestVersion &&
        routerRevisionRef.current === revisionAtStart
      ) {
        setPresentProviders(providers)
      }
    } catch {
      // Preserve the last authoritative provider snapshot on transport errors.
    }
  }, [backendId, routerAvailable, routerRevision])

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
