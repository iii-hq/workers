import { useQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import {
  fetchTraces,
  type TracesFilterParams,
  type TracesResponse,
} from '../api/traces'
import { fingerprintTraceList, mapSpanToListItem } from '../lib/traceListItem'

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
  /** Set when the flat-list fetch threw a non-exporter error (network, RPC,
   *  timeout). Lets the page show a real error state instead of silently
   *  falling back to the misleading "no observability" empty state. */
  queryError: Error | null
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
    error: queryError,
    refetch,
  } = useQuery<TracesResponse, Error>({
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
    // Live updates arrive via `useTracesLiveRefresh` (ui::traces::changed
    // push) — no polling interval. Initial mount fetch + manual Refresh +
    // signal-driven invalidation cover refresh.
    refetchInterval: false,
    staleTime: 1000,
  })

  useEffect(() => {
    if (!tracesData) return

    // Observability is configured unless the engine explicitly reports the
    // exporter disabled. An empty span list is a normal "no matching traces"
    // result and must NOT flip the page to the "no observability" state.
    // Set this first, before the fingerprint early-return, so an unchanged
    // payload still keeps the flag correct.
    setHasOtelConfigured(!tracesData.exporterDisabled)

    if (tracesData.spans && tracesData.spans.length > 0) {
      const traces: TraceListItem[] = tracesData.spans.map(mapSpanToListItem)

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
    } else {
      setTraceListItems([])
      // Reset the dedup state so a later non-empty fetch is detected as
      // fresh (otherwise the fingerprint/new-trace diff would be stale).
      fingerprintRef.current = ''
      prevTraceIdsRef.current = new Set()
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
    queryError: queryError ?? null,
    refetch,
    isHoveredRef,
    flushPendingTraces,
  }
}
