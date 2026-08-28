import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { startTraceActivityFeed } from '@/lib/traces-activity'
import {
  fetchTraces,
  type TracesFilterParams,
  type TracesResponse,
} from '../api/traces'
import { collectRecentSpanWindow } from '../lib/traceDetailPages'
import {
  dedupeToTraceRoots,
  fingerprintTraceList,
  mapSpanToListItem,
} from '../lib/traceListItem'

const DEFAULT_TRACE_LIMIT = 500
/**
 * Span budget for `search_all_spans` seeds. Full-fat spans run 30-80KB and
 * the server-side scan costs ~3.4s PER CALL regardless of limit/offset
 * (measured live), so the window has to stay small: 250 recent spans cover a
 * session's recent turns in 2-3 parallel reads. Deeper history stays
 * reachable through the pager and text search.
 */
const SEARCH_SEED_MAX_SPANS = 250

/** Client-side debounce over activity ticks before refetching. The engine
 *  already coalesces to ~one tick per 300ms window; this only collapses the
 *  invalidate → POST round-trips of a burst of consecutive windows. */
const ACTIVITY_REFETCH_DEBOUNCE_MS = 250
/** Minimum gap between activity-driven reseeds of a filtered/searched list.
 *  That seed sweeps parallel multi-MB windows (see the queryFn); riding the
 *  short tick debounce alone would re-run it back-to-back under a busy
 *  session and saturate the main thread with payload parses. */
const FILTERED_RESEED_COOLDOWN_MS = 10_000

export function shouldDeferTraceUpdate(
  isHovered: boolean,
  hadPreviousRows: boolean,
): boolean {
  return isHovered && hadPreviousRows
}

export interface TraceListItem {
  traceId: string
  rootOperation: string
  functionId?: string
  topic?: string
  status: 'ok' | 'error' | 'pending'
  startTime: number
  endTime?: number
  duration?: number
  spanCount: number
  workers: string[]
  /** Root/row span attributes, normalized to a flat object. */
  attributes?: Record<string, unknown>
  /** Trace-level tags merged by `engine::traces::list`. */
  traceTags?: Record<string, string>
}

export interface UseTraceDataOptions {
  filterParams: TracesFilterParams
  showSystem: boolean
  debouncedSearch: string
  isPaused: boolean
  /**
   * Root functions to drop from the list (exact `function_id` /
   * `faas.invoked_name` match on the row). Applied client-side over both
   * the seed read and streamed rows, so toggling is instant and the
   * live-append cache path stays intact.
   */
  hiddenFunctions?: string[]
}

export interface UseTraceDataReturn {
  traceGroups: TraceListItem[]
  newTraceIds: Set<string>
  setNewTraceIds: React.Dispatch<React.SetStateAction<Set<string>>>
  /** `false` ONLY on the engine's definitive "memory exporter not enabled"
   *  answer; `null` while no response has settled yet (render loading, not
   *  the no-observability message); `true` once any response proves the
   *  pipeline — an empty result is an empty list, not missing observability. */
  hasOtelConfigured: boolean | null
  isQueryLoading: boolean
  refetch: () => void
  isHoveredRef: React.RefObject<boolean>
  flushPendingTraces: () => void
}

export function useTraceData({
  filterParams,
  showSystem,
  debouncedSearch,
  isPaused,
  hiddenFunctions,
}: UseTraceDataOptions): UseTraceDataReturn {
  const [traceGroups, setTraceListItems] = useState<TraceListItem[]>([])
  const [hasOtelConfigured, setHasOtelConfigured] = useState<boolean | null>(
    null,
  )
  const [newTraceIds, setNewTraceIds] = useState<Set<string>>(new Set())

  const fingerprintRef = useRef<string>('')
  const prevTraceIdsRef = useRef<Set<string>>(new Set())

  const isHoveredRef = useRef(false)
  const pendingTracesRef = useRef<TraceListItem[] | null>(null)

  const {
    data: tracesData,
    isLoading: isQueryLoading,
    refetch,
  } = useQuery({
    queryKey: ['traces', filterParams, showSystem, debouncedSearch],
    queryFn: async (): Promise<TracesResponse> => {
      const params = {
        ...filterParams,
        ...(debouncedSearch && !filterParams.name
          ? { name: debouncedSearch, search_all_spans: true }
          : {}),
        include_internal: showSystem,
      }
      if (params.search_all_spans !== true) {
        // Roots-only responses carry thin spans — one read is light & safe.
        return fetchTraces({ ...params, offset: 0, limit: DEFAULT_TRACE_LIMIT })
      }
      // A search_all_spans seed (session scope, text search) returns FULL
      // spans: one big read can exceed the transport's ~16MiB delivery cap
      // and hang forever — no error. Collect a byte-priced recency window
      // instead, windows in parallel because the server-side scan costs
      // seconds per call regardless of limit/offset.
      let exporterDisabled = false
      const { spans, total } = await collectRecentSpanWindow(
        async (offset, limit) => {
          const page = await fetchTraces({ ...params, offset, limit })
          if (page.memoryExporterDisabled) exporterDisabled = true
          return page
        },
        SEARCH_SEED_MAX_SPANS,
      )
      return exporterDisabled
        ? {
            spans: [],
            total: 0,
            offset: 0,
            limit: SEARCH_SEED_MAX_SPANS,
            memoryExporterDisabled: true,
          }
        : { spans, total, offset: 0, limit: SEARCH_SEED_MAX_SPANS }
    },
    // This query is the SEED read, re-run on demand: the `trace` trigger's
    // coalesced `{trace_ids}` tick invalidates it (notify-then-query, see
    // the effect below) — no polling interval. Reconnect / tab-visible
    // re-seed once to recover anything missed while away.
    refetchInterval: false,
    staleTime: 1000,
    // The collector already ladders timeouts internally; stacking the
    // default 3 retries on top would turn a failed seed into minutes.
    retry: 1,
  })

  const hiddenKey = hiddenFunctions?.join(',') ?? ''

  // A scope/filter change is a DIFFERENT question — the previous answer's
  // rows must not linger under the new scope (measured live: the old
  // session's trace stayed on screen, under the new session's chip, for the
  // whole slow fetch). Dropping them also re-arms the loading skeleton.
  const scopeKey = JSON.stringify([filterParams, showSystem, debouncedSearch])
  const scopeKeyRef = useRef(scopeKey)
  useEffect(() => {
    if (scopeKeyRef.current === scopeKey) return
    scopeKeyRef.current = scopeKey
    setTraceListItems([])
    fingerprintRef.current = ''
    prevTraceIdsRef.current = new Set()
    pendingTracesRef.current = null
    setNewTraceIds(new Set())
  }, [scopeKey])

  useEffect(() => {
    if (!tracesData) return
    if (tracesData.memoryExporterDisabled) {
      setTraceListItems([])
      setHasOtelConfigured(false)
      return
    }

    if (tracesData.spans && tracesData.spans.length > 0) {
      // Search uses `search_all_spans`, which returns every span of each
      // matching trace; collapse to one row per trace so the flat list stays
      // trace-per-row (no-op for the non-search roots-only response).
      let traces: TraceListItem[] = dedupeToTraceRoots(tracesData.spans).map(
        mapSpanToListItem,
      )

      // Hidden functions: root-match only, applied after mapping so both the
      // seed read and streamed cache merges pass through the same gate.
      const hidden = hiddenKey ? hiddenKey.split(',') : []
      if (hidden.length > 0) {
        traces = traces.filter(
          (t) => !(t.functionId && hidden.includes(t.functionId)),
        )
      }

      traces.sort((a, b) => b.startTime - a.startTime)

      // The same rows can answer two different questions (for example the
      // unfiltered list followed by a text search that matches its only
      // row). Scope changes clear the rendered list, so a fingerprint from
      // the previous scope must never suppress the new scope's first answer.
      const fingerprint = `${scopeKey}\0${fingerprintTraceList(traces)}`
      if (fingerprint === fingerprintRef.current) return
      fingerprintRef.current = fingerprint

      const currentIds = new Set(traces.map((t) => t.traceId))
      const hadPreviousRows = prevTraceIdsRef.current.size > 0
      if (hadPreviousRows) {
        const freshIds = new Set<string>()
        for (const id of currentIds) {
          if (!prevTraceIdsRef.current.has(id)) freshIds.add(id)
        }
        if (freshIds.size > 0) setNewTraceIds(freshIds)
      }
      prevTraceIdsRef.current = currentIds

      // Freeze churn only when there is already a rendered list whose row
      // positions must stay stable under the pointer. A scope/search change
      // clears the list first; deferring that scope's first answer while the
      // user is still hovering the search field would leave an empty screen
      // until the pointer happened to leave the traces pane.
      if (shouldDeferTraceUpdate(isHoveredRef.current, hadPreviousRows)) {
        pendingTracesRef.current = traces
        return
      }

      setTraceListItems(traces)
      setHasOtelConfigured(true)
    } else {
      // An empty answer from a working exporter is an empty list — the
      // no-observability message is reserved for the marker above.
      setTraceListItems([])
      setHasOtelConfigured(true)
    }
  }, [tracesData, hiddenKey, scopeKey])

  // ── Trigger-driven refetch (notify-then-query) ──────────────────────────
  // The engine coalesces span activity into one `{ trace_ids }` tick per
  // ~300ms window on the `trace` trigger; each tick re-runs the seeded,
  // FILTERED queries — the engine stays the single owner of filter and tag
  // semantics, an idle engine produces zero traffic, and there is no
  // client-side append/merge to drift from the server's view. Pause /
  // tab-hidden freeze it; reconnect and tab-visible re-seed once to recover
  // anything missed while away. Subscribes once for the hook's lifetime.
  const qc = useQueryClient()
  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])
  // Whether the CURRENT seed is the expensive search_all_spans sweep —
  // read through a ref inside the once-per-lifetime subscription below.
  const isSearchAllSeed =
    filterParams.search_all_spans === true ||
    Boolean(debouncedSearch && !filterParams.name)
  const searchAllSeedRef = useRef(isSearchAllSeed)
  useEffect(() => {
    searchAllSeedRef.current = isSearchAllSeed
  }, [isSearchAllSeed])

  useEffect(() => {
    let stop: (() => void) | undefined
    let disposed = false

    const isHidden = () =>
      typeof document !== 'undefined' && document.visibilityState === 'hidden'
    // The search_all_spans seed is EXPENSIVE (parallel multi-MB windows —
    // see the queryFn): its reseed rides a trailing cooldown instead of the
    // short tick debounce, so a busy session cannot re-run it back-to-back.
    let reseedCooldownTimer: ReturnType<typeof setTimeout> | undefined
    let lastTracesReseed = 0
    const reseedTraces = () => {
      lastTracesReseed = Date.now()
      qc.invalidateQueries({ queryKey: ['traces'] })
    }
    const throttledFilteredReseed = () => {
      const wait = Math.max(
        0,
        lastTracesReseed + FILTERED_RESEED_COOLDOWN_MS - Date.now(),
      )
      if (wait === 0) {
        reseedTraces()
        return
      }
      if (reseedCooldownTimer !== undefined) return
      reseedCooldownTimer = setTimeout(() => {
        reseedCooldownTimer = undefined
        reseedTraces()
      }, wait)
    }
    const refetchGroups = () => {
      qc.invalidateQueries({ queryKey: ['traceGroups'] })
      qc.invalidateQueries({ queryKey: ['traceGroupMembers'] })
    }
    const refetchAll = () => {
      reseedTraces()
      refetchGroups()
    }

    let refetchTimer: ReturnType<typeof setTimeout> | undefined
    const scheduleRefetch = () => {
      if (refetchTimer !== undefined) return
      refetchTimer = setTimeout(() => {
        refetchTimer = undefined
        if (disposed || isPausedRef.current || isHidden()) return
        if (searchAllSeedRef.current) {
          throttledFilteredReseed()
          refetchGroups()
        } else {
          refetchAll()
        }
      }, ACTIVITY_REFETCH_DEBOUNCE_MS)
    }

    void (async () => {
      const client = await getIiiClient()
      if (disposed) return

      const offActivity = startTraceActivityFeed(client, () => {
        if (isPausedRef.current || isHidden()) return
        scheduleRefetch()
      })

      const offConn = client.addConnectionStateListener((state) => {
        if (state === 'connected' && !isPausedRef.current) refetchAll()
      })

      let offVisibility: (() => void) | undefined
      if (typeof document !== 'undefined') {
        const onVisible = () => {
          if (document.visibilityState === 'visible' && !isPausedRef.current) {
            refetchAll()
          }
        }
        document.addEventListener('visibilitychange', onVisible)
        offVisibility = () =>
          document.removeEventListener('visibilitychange', onVisible)
      }

      stop = () => {
        offActivity()
        offConn()
        offVisibility?.()
        if (refetchTimer !== undefined) {
          clearTimeout(refetchTimer)
          refetchTimer = undefined
        }
        if (reseedCooldownTimer !== undefined) {
          clearTimeout(reseedCooldownTimer)
          reseedCooldownTimer = undefined
        }
      }
    })()

    return () => {
      disposed = true
      stop?.()
    }
  }, [qc])

  const flushPendingTraces = () => {
    if (pendingTracesRef.current) {
      setTraceListItems(pendingTracesRef.current)
      setHasOtelConfigured(true)
      pendingTracesRef.current = null
    }
  }

  return {
    traceGroups,
    newTraceIds,
    setNewTraceIds,
    hasOtelConfigured,
    isQueryLoading,
    refetch,
    isHoveredRef,
    flushPendingTraces,
  }
}
