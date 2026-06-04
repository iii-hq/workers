import { useQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { fetchTraces, type TracesFilterParams } from '../api/traces'
import {
  dedupeToTraceRoots,
  fingerprintTraceList,
  mapSpanToListItem,
} from '../lib/traceListItem'

const DEFAULT_TRACE_LIMIT = 500

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
  services: string[]
}

export interface UseTraceDataOptions {
  filterParams: TracesFilterParams
  showSystem: boolean
  debouncedSearch: string
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
    // Live updates arrive via `useTracesLiveRefresh` (the engine `trace`
    // trigger), which invalidates the ['traces'] key — no polling interval.
    // Initial mount fetch + manual Refresh + signal-driven invalidation cover
    // refresh; reconnect/tab-visible re-sync handles cold-start races.
    refetchInterval: false,
    staleTime: 1000,
  })

  useEffect(() => {
    if (!tracesData) return

    if (tracesData.spans && tracesData.spans.length > 0) {
      // Search uses `search_all_spans`, which returns every span of each
      // matching trace; collapse to one row per trace so the flat list stays
      // trace-per-row (no-op for the non-search roots-only response).
      const traces: TraceListItem[] = dedupeToTraceRoots(tracesData.spans).map(
        mapSpanToListItem,
      )

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
  }, [tracesData])

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
