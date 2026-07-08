import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import {
  isAppendableTraceList,
  mergeTraceListSpans,
  startTraceActivityFeed,
  startTraceListStream,
} from '@/lib/traces-stream'
import {
  fetchTraces,
  type TracesFilterParams,
  type TracesResponse,
} from '../api/traces'
import {
  dedupeToTraceRoots,
  fingerprintTraceList,
  mapSpanToListItem,
} from '../lib/traceListItem'

const DEFAULT_TRACE_LIMIT = 500

/** Coalesce window for the trace-tags refresh: batches the trace ids that
 *  arrived from the rows stream / activity feed into one `trace_ids` read. */
const TAG_REFRESH_DEBOUNCE_MS = 1000
/** Ids per refresh read. Overflow is dropped, not queued — a still-active
 *  trace re-enters via its next activity window. */
const TAG_REFRESH_MAX_IDS = 100

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
  /** Trace-level tags merged by `engine::traces::list` (absent on
   *  live-streamed rows until the next seed read). */
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
  hasOtelConfigured: boolean
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
  const [hasOtelConfigured, setHasOtelConfigured] = useState(false)
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
    queryFn: () =>
      fetchTraces({
        ...filterParams,
        ...(debouncedSearch && !filterParams.name
          ? { name: debouncedSearch, search_all_spans: true }
          : {}),
        offset: 0,
        limit: DEFAULT_TRACE_LIMIT,
        include_internal: showSystem,
      }),
    // This query is the one-time SEED read. Live updates arrive by APPEND over
    // the engine `iii:devtools:trace-rows` stream (see the stream effect
    // below), which merges new rows straight into this cache — no polling
    // interval. Reconnect / tab-visible re-seed once to self-heal dropped
    // frames; manual Refresh re-reads on demand.
    refetchInterval: false,
    staleTime: 1000,
  })

  const hiddenKey = hiddenFunctions?.join(',') ?? ''
  useEffect(() => {
    if (!tracesData) return

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

      const fingerprint = fingerprintTraceList(traces)
      if (fingerprint === fingerprintRef.current) return
      fingerprintRef.current = fingerprint

      const currentIds = new Set(traces.map((t) => t.traceId))
      if (prevTraceIdsRef.current.size > 0) {
        const freshIds = new Set<string>()
        for (const id of currentIds) {
          if (!prevTraceIdsRef.current.has(id)) freshIds.add(id)
        }
        if (freshIds.size > 0) setNewTraceIds(freshIds)
      }
      prevTraceIdsRef.current = currentIds

      if (isHoveredRef.current) {
        pendingTracesRef.current = traces
        return
      }

      setTraceListItems(traces)
      setHasOtelConfigured(true)
    } else {
      setTraceListItems([])
      setHasOtelConfigured(false)
    }
  }, [tracesData, hiddenKey])

  // ── Live append over the engine `trace-rows` stream ──────────────────────
  // The engine pushes new root rows as spans close. The UNFILTERED list merges
  // them directly into the seed query's cache (pure append, no refetch); a
  // filtered/searched list — and the group-by aggregate, which can't be
  // appended — refetches on activity instead (the engine already coalesces to
  // ~one push per window, so this is not a poll). Pause / tab-hidden freeze it;
  // reconnect and tab-visible re-seed once to recover anything dropped while
  // away. Subscribes once for the hook's lifetime (params are read via refs).
  const qc = useQueryClient()
  const mergeKeyRef = useRef<{
    key: unknown[]
    unfiltered: boolean
    includeInternal: boolean
  }>({
    key: [],
    unfiltered: false,
    includeInternal: false,
  })
  mergeKeyRef.current = {
    key: ['traces', filterParams, showSystem, debouncedSearch],
    unfiltered: isAppendableTraceList(filterParams, debouncedSearch),
    includeInternal: showSystem,
  }
  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])

  useEffect(() => {
    let stop: (() => void) | undefined
    let disposed = false

    const isHidden = () =>
      typeof document !== 'undefined' && document.visibilityState === 'hidden'
    const reseed = () => {
      qc.invalidateQueries({ queryKey: ['traces'] })
      qc.invalidateQueries({ queryKey: ['traceGroups'] })
      qc.invalidateQueries({ queryKey: ['traceGroupMembers'] })
    }

    // ── Trace-tags refresh ────────────────────────────────────────────────
    // Streamed rows arrive WITHOUT `trace_tags` (only `traces::list` merges
    // them), and a row's tags can change after it exists — the tag-bearing
    // span (e.g. the harness turn step carrying `iii.session.name`) closes
    // well after the root row was pushed. Both funnel into one debounced
    // `trace_ids` read whose `trace_tags` are merged back into the cache; a
    // filtered list can't be patched row-wise, so it refetches instead.
    const pendingTagIds = new Set<string>()
    let tagFlushTimer: ReturnType<typeof setTimeout> | undefined

    const tagsEqual = (
      a: Record<string, string> | undefined,
      b: Record<string, string> | undefined,
    ): boolean => {
      const ea = Object.entries(a ?? {})
      const eb = b ?? {}
      return (
        ea.length === Object.keys(eb).length &&
        ea.every(([k, v]) => eb[k] === v)
      )
    }

    const flushTagRefresh = async () => {
      const ids = [...pendingTagIds].slice(0, TAG_REFRESH_MAX_IDS)
      pendingTagIds.clear()
      if (ids.length === 0 || isPausedRef.current || isHidden()) return
      const { key, unfiltered, includeInternal } = mergeKeyRef.current
      if (!unfiltered) {
        // Row-wise patching is only sound for the append cache; a filtered
        // list re-evaluates its filters (tags may move rows in/out of it).
        qc.invalidateQueries({ queryKey: ['traces'] })
        return
      }
      try {
        const res = await fetchTraces({
          trace_ids: ids,
          include_internal: includeInternal,
          limit: ids.length,
        })
        if (disposed) return
        const tagsByTrace = new Map(
          res.spans
            .filter((s) => s.trace_tags)
            .map((s) => [s.trace_id, s.trace_tags]),
        )
        if (tagsByTrace.size === 0) return
        qc.setQueryData<TracesResponse>(key, (old) => {
          if (!old) return old
          let changed = false
          const spans = old.spans.map((s) => {
            const tags = tagsByTrace.get(s.trace_id)
            if (!tags || tagsEqual(tags, s.trace_tags)) return s
            changed = true
            return { ...s, trace_tags: tags }
          })
          return changed ? { ...old, spans } : old
        })
      } catch {
        // Transient read failure — the next activity window retries.
      }
    }

    const scheduleTagRefresh = (ids: ReadonlyArray<string>) => {
      for (const id of ids) pendingTagIds.add(id)
      if (pendingTagIds.size === 0 || tagFlushTimer !== undefined) return
      tagFlushTimer = setTimeout(() => {
        tagFlushTimer = undefined
        void flushTagRefresh()
      }, TAG_REFRESH_DEBOUNCE_MS)
    }

    void (async () => {
      const client = await getIiiClient()
      if (disposed) return

      const offStream = startTraceListStream(client, (spans) => {
        if (isPausedRef.current || isHidden()) return
        const { key, unfiltered } = mergeKeyRef.current
        if (unfiltered) {
          qc.setQueryData<TracesResponse>(key, (old) => {
            const merged = mergeTraceListSpans(
              old?.spans ?? [],
              spans,
              DEFAULT_TRACE_LIMIT,
            )
            return {
              spans: merged,
              total: merged.length,
              offset: 0,
              limit: DEFAULT_TRACE_LIMIT,
            }
          })
          // The appended rows carry no tags yet; fetch them once the trace's
          // spans settle. (The filtered branch refetches, which re-reads tags.)
          scheduleTagRefresh(spans.map((s) => s.trace_id))
        } else {
          qc.invalidateQueries({ queryKey: ['traces'] })
        }
        // The group-by aggregate (and any expanded group's member list)
        // can't be appended; refetch them on activity.
        qc.invalidateQueries({ queryKey: ['traceGroups'] })
        qc.invalidateQueries({ queryKey: ['traceGroupMembers'] })
      })

      // Span activity on traces we already list = their merged tags may have
      // just changed (late tag-bearing span, renamed session). Only known
      // rows schedule — activity for traces outside the list is noise here.
      const offActivity = startTraceActivityFeed(client, (traceIds) => {
        if (isPausedRef.current || isHidden()) return
        const cached = qc.getQueryData<TracesResponse>(mergeKeyRef.current.key)
        if (!cached || cached.spans.length === 0) return
        const known = new Set(cached.spans.map((s) => s.trace_id))
        const relevant = traceIds.filter((id) => known.has(id))
        if (relevant.length > 0) scheduleTagRefresh(relevant)
      })

      const offConn = client.addConnectionStateListener((state) => {
        if (state === 'connected' && !isPausedRef.current) reseed()
      })

      let offVisibility: (() => void) | undefined
      if (typeof document !== 'undefined') {
        const onVisible = () => {
          if (document.visibilityState === 'visible' && !isPausedRef.current) {
            reseed()
          }
        }
        document.addEventListener('visibilitychange', onVisible)
        offVisibility = () =>
          document.removeEventListener('visibilitychange', onVisible)
      }

      stop = () => {
        offStream()
        offActivity()
        offConn()
        offVisibility?.()
        if (tagFlushTimer !== undefined) {
          clearTimeout(tagFlushTimer)
          tagFlushTimer = undefined
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
