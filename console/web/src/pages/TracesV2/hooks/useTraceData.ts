import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { startTraceActivityFeed } from '@/lib/traces-activity'
import {
  fetchTraces,
  type TracesFilterParams,
  type TracesResponse,
} from '../api/traces'
import { createRefetchCoalescer } from '../lib/refetchCoalescer'
import { withHiddenFunctionExclusions } from '../lib/traceFilters'
import {
  fingerprintTraceList,
  mapTraceSummaryToListItem,
} from '../lib/traceListItem'

const DEFAULT_TRACE_PAGE_SIZE = 50

/** Client-side debounce over activity ticks before refetching. The engine
 *  already coalesces to ~one tick per 300ms window; this only collapses the
 *  invalidate → POST round-trips of a burst of consecutive windows. */
const ACTIVITY_REFETCH_DEBOUNCE_MS = 250
/** Floor between two consecutive activity-driven refetch STARTS of the list
 *  and of the group aggregate. Under sustained activity the engine sees one
 *  scan per surface per second at most, and the rows update at a steady
 *  cadence instead of in bursts (see `createRefetchCoalescer`). */
export const ACTIVITY_REFETCH_MIN_INTERVAL_MS = 1_000
/** Minimum gap between activity-driven reseeds of a filtered/searched list.
 *  The response is compact, but child-span search still scans the engine's
 *  store; riding the short tick debounce would re-run that scan back-to-back. */
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
   * `faas.invoked_name` match on the row). Sent to the engine so `total`
   * and server pages describe the same result set; retained client-side as
   * a defensive check for older engines.
   */
  hiddenFunctions?: string[]
  /** Arbitrary attributes required by the active row-label view. */
  attributeProjection?: string[]
}

export interface UseTraceDataReturn {
  traceGroups: TraceListItem[]
  /** Total matching traces before server pagination. */
  totalTraceCount: number
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

export function hiddenFunctionsKey(hiddenFunctions?: string[]): string {
  return JSON.stringify(hiddenFunctions ?? [])
}

export function filterHiddenTraceRows(
  traces: TraceListItem[],
  serializedHiddenFunctions: string,
): TraceListItem[] {
  const hidden = JSON.parse(serializedHiddenFunctions) as string[]
  if (hidden.length === 0) return traces
  return traces.filter(
    (trace) => !(trace.functionId && hidden.includes(trace.functionId)),
  )
}

/**
 * Rows still playing their arrival flash keep it when the next answer lands:
 * replacing the set would cut the animation short on every refetch during a
 * busy stretch. Ids no longer on the page are dropped so the set stays
 * bounded even where `animationend` never fires (reduced motion).
 */
export function mergeNewTraceIds(
  previous: ReadonlySet<string>,
  fresh: ReadonlySet<string>,
  listed: ReadonlySet<string>,
): Set<string> {
  const next = new Set<string>()
  for (const id of previous) if (listed.has(id)) next.add(id)
  for (const id of fresh) next.add(id)
  return next
}

export function traceTotalForResponse(
  response: TracesResponse,
  filteredTraceCount: number,
): number {
  return response.legacyContract ? filteredTraceCount : response.total
}

export function buildTraceListRequestParams({
  filterParams,
  showSystem,
  debouncedSearch,
  hiddenFunctions,
  attributeProjection,
}: Omit<UseTraceDataOptions, 'isPaused'>): TracesFilterParams {
  const engineFilters = withHiddenFunctionExclusions(
    filterParams,
    hiddenFunctions,
  )
  const params = {
    ...engineFilters,
    ...(debouncedSearch && !engineFilters.name
      ? { name: debouncedSearch, search_all_spans: true }
      : {}),
    include_internal: showSystem,
    attribute_projection: attributeProjection,
  }
  return {
    ...params,
    offset: params.offset ?? 0,
    limit: params.limit ?? DEFAULT_TRACE_PAGE_SIZE,
  }
}

export function useTraceData({
  filterParams,
  showSystem,
  debouncedSearch,
  isPaused,
  hiddenFunctions,
  attributeProjection,
}: UseTraceDataOptions): UseTraceDataReturn {
  const [traceGroups, setTraceListItems] = useState<TraceListItem[]>([])
  const [totalTraceCount, setTotalTraceCount] = useState(0)
  const [hasOtelConfigured, setHasOtelConfigured] = useState<boolean | null>(
    null,
  )
  const [newTraceIds, setNewTraceIds] = useState<Set<string>>(new Set())

  const fingerprintRef = useRef<string>('')
  const prevTraceIdsRef = useRef<Set<string>>(new Set())

  const isHoveredRef = useRef(false)
  const pendingTracesRef = useRef<TraceListItem[] | null>(null)
  const hiddenKey = hiddenFunctionsKey(hiddenFunctions)

  const {
    data: tracesData,
    isLoading: isQueryLoading,
    refetch,
  } = useQuery({
    queryKey: [
      'traces',
      filterParams,
      showSystem,
      debouncedSearch,
      attributeProjection,
      hiddenKey,
    ],
    queryFn: async (): Promise<TracesResponse> => {
      // The engine returns one compact summary per trace even when child
      // spans are searched. Preserve the caller's offset/limit so the list
      // pages in the engine instead of retaining an arbitrary 500-row window.
      return fetchTraces(
        buildTraceListRequestParams({
          filterParams,
          showSystem,
          debouncedSearch,
          hiddenFunctions,
          attributeProjection,
        }),
      )
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

  // A scope/filter change is a DIFFERENT question — the previous answer's
  // rows must not linger under the new scope (measured live: the old
  // session's trace stayed on screen, under the new session's chip, for the
  // whole slow fetch). Dropping them also re-arms the loading skeleton.
  const scopeKey = JSON.stringify([
    filterParams,
    showSystem,
    debouncedSearch,
    attributeProjection,
    hiddenKey,
  ])
  // Pagination changes the page but not the result set. Keep the known total
  // while a new page loads; reset it only when filters/scope actually change.
  const resultSetKey = JSON.stringify([
    Object.entries(filterParams).filter(
      ([key]) => key !== 'offset' && key !== 'limit',
    ),
    showSystem,
    debouncedSearch,
    hiddenKey,
  ])
  const scopeKeyRef = useRef(scopeKey)
  const resultSetKeyRef = useRef(resultSetKey)
  useEffect(() => {
    if (scopeKeyRef.current === scopeKey) return
    const resultSetChanged = resultSetKeyRef.current !== resultSetKey
    scopeKeyRef.current = scopeKey
    resultSetKeyRef.current = resultSetKey
    setTraceListItems([])
    if (resultSetChanged) setTotalTraceCount(0)
    fingerprintRef.current = ''
    prevTraceIdsRef.current = new Set()
    pendingTracesRef.current = null
    setNewTraceIds(new Set())
  }, [scopeKey, resultSetKey])

  useEffect(() => {
    if (!tracesData) return
    if (tracesData.memoryExporterDisabled) {
      setTraceListItems([])
      setTotalTraceCount(0)
      setHasOtelConfigured(false)
      return
    }
    let traces: TraceListItem[] = (tracesData.traces ?? []).map(
      mapTraceSummaryToListItem,
    )

    // Defensive compatibility for engines that ignore exclude_attributes.
    // JSON preserves commas and other valid characters inside function IDs.
    traces = filterHiddenTraceRows(traces, hiddenKey)

    // New Engines already exclude hidden rows before pagination and own the
    // global total. A legacy response was aggregated and filtered locally, so
    // its only honest total is the number of compatible rows on this page.
    setTotalTraceCount(traceTotalForResponse(tracesData, traces.length))

    if (traces.length > 0) {
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
        if (freshIds.size > 0) {
          setNewTraceIds((prev) => mergeNewTraceIds(prev, freshIds, currentIds))
        }
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
  //
  // Ticks never CANCEL a refetch in flight: each surface runs one scan at a
  // time, a tick that lands mid-scan queues exactly one trailing rerun, and
  // consecutive scans keep a 1s floor. Invalidating on every tick used to
  // cancel the in-flight fetch client-side while the engine kept executing
  // it — under a busy session the overlapping scans pushed latency past the
  // tick interval and the list froze until the activity paused, then jumped
  // (MOT-4621). See `createRefetchCoalescer`.
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
    const canRefetch = () => !disposed && !isPausedRef.current && !isHidden()

    // `cancelRefetch: false` keeps react-query from abandoning a fetch that is
    // already running; the coalescer guarantees there is at most one anyway.
    const invalidate = (queryKey: readonly unknown[]) =>
      qc.invalidateQueries({ queryKey }, { cancelRefetch: false })

    // A search_all_spans seed still performs an engine-side scan: its reseed
    // rides a longer cooldown so a busy session cannot re-run it back-to-back.
    const traces = createRefetchCoalescer({
      run: () => invalidate(['traces']),
      debounceMs: ACTIVITY_REFETCH_DEBOUNCE_MS,
      minIntervalMs: () =>
        searchAllSeedRef.current
          ? FILTERED_RESEED_COOLDOWN_MS
          : ACTIVITY_REFETCH_MIN_INTERVAL_MS,
      shouldRun: canRefetch,
    })
    const groups = createRefetchCoalescer({
      run: () =>
        Promise.all([
          invalidate(['traceGroups']),
          invalidate(['traceGroupMembers']),
        ]),
      debounceMs: ACTIVITY_REFETCH_DEBOUNCE_MS,
      minIntervalMs: ACTIVITY_REFETCH_MIN_INTERVAL_MS,
      shouldRun: canRefetch,
    })
    const refetchAll = () => {
      traces.request()
      groups.request()
    }

    void (async () => {
      const client = await getIiiClient()
      if (disposed) return

      const offActivity = startTraceActivityFeed(client, () => {
        if (isPausedRef.current || isHidden()) return
        refetchAll()
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
      }
    })()

    return () => {
      disposed = true
      stop?.()
      traces.dispose()
      groups.dispose()
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
    totalTraceCount,
    newTraceIds,
    setNewTraceIds,
    hasOtelConfigured,
    isQueryLoading,
    refetch,
    isHoveredRef,
    flushPendingTraces,
  }
}
