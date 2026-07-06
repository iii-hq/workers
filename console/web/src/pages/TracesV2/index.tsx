/**
 * TracesV2 — the isolated "core experience" copy of the Traces page shell.
 *
 * Composition (this pass): the live TimelineStrip is the masthead — it
 * replaced the `$ traces` page header and carries the system/pause/refresh
 * actions in its header row. Below it the surface has two modes:
 *
 * - LIST mode (no trace selected): filter bar → trace list → pagination.
 * - DETAIL mode (trace selected): the detail fills the entire canvas — no
 *   list, no filter bar (the chat view keeps horizontal space scarce). The
 *   view switcher offers the lane timeline (default) and the waterfall;
 *   clicking a span opens the resizable span panel on the right. The strip
 *   stays live up top; clicking another bar switches traces, Esc walks back
 *   (span panel first, then the detail).
 *
 * The group-by list, session-detail panel, and the map/flow visualizations
 * are NOT copied here (out of scope). The live-detail stream wiring is
 * faithful — in Storybook the fake iii-client makes the stream a no-op and
 * serves the seed read from fixtures.
 */

import { AlertCircle, GitBranch, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Cell } from '@/components/ui/Cell'
import { EmptyState } from '@/components/ui/EmptyState'
import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import { Pagination } from '@/components/ui/Pagination'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { getIiiClient } from '@/lib/iii-client'
import { startTraceSpansStream } from '@/lib/traces-stream'
import { cn } from '@/lib/utils'
import { fetchTraces, type StoredSpan } from './api/traces'
import { ServiceBreakdown } from './components/ServiceBreakdown'
import { SpanPanel } from './components/SpanPanel'
import { TraceFilters } from './components/TraceFilters'
import { TraceHeader } from './components/TraceHeader'
import { TraceListRow } from './components/TraceListRow'
import { TimelineStrip } from './components/timeline/TimelineStrip'
import { TraceTimeline } from './components/timeline/TraceTimeline'
import { ViewSwitcher, type ViewType } from './components/ViewSwitcher'
import { WaterfallChart } from './components/WaterfallChart'
import { useSpanPanelResize } from './hooks/useSpanPanelResize'
import { useTraceActivity } from './hooks/useTraceActivity'
import { useTraceData } from './hooks/useTraceData'
import { useTraceFilters } from './hooks/useTraceFilters'
import { traceSpanGroupKey } from './lib/traceTimelineFilters'
import {
  mergeDetailSpan,
  toWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from './lib/traceTransform'

const PAGE_SIZES = [25, 50, 100]

export interface TracesV2Props {
  /** mount straight into a trace's full-canvas detail (stories/deep links) */
  initialTraceId?: string
}

export function TracesV2({ initialTraceId }: TracesV2Props) {
  const [showSystem, setShowSystem] = useState(false)
  const [isPaused, setIsPaused] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [debouncedSearch, setDebouncedSearch] = useState('')
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const handleSearchChange = useCallback((value: string) => {
    setSearchQuery(value)
    if (searchTimerRef.current) clearTimeout(searchTimerRef.current)
    searchTimerRef.current = setTimeout(() => setDebouncedSearch(value), 300)
  }, [])

  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null)
  const [selectedSpan, setSelectedSpan] = useState<VisualizationSpan | null>(
    null,
  )
  const [activeView, setActiveView] = useState<ViewType>('timeline')
  const [waterfallData, setWaterfallData] = useState<WaterfallData | null>(null)

  // The clicked span is a snapshot; live rebuilds (streamed finals replacing
  // pending snapshots, growing durations) must reach the open panel, so
  // re-derive the current version from the waterfall by id.
  const liveSelectedSpan = useMemo(() => {
    if (!selectedSpan) return null
    return (
      waterfallData?.spans.find((s) => s.span_id === selectedSpan.span_id) ??
      selectedSpan
    )
  }, [selectedSpan, waterfallData])
  const [isLoadingSpans, setIsLoadingSpans] = useState(false)
  const [spansError, setSpansError] = useState<string | null>(null)

  const {
    filters: filterState,
    updateFilter,
    resetFilters,
    getFilterOnlyParams,
    validationWarnings,
    clearValidationWarnings,
  } = useTraceFilters()

  const filterParams = useMemo(
    () => getFilterOnlyParams(),
    [getFilterOnlyParams],
  )

  const {
    traceGroups,
    newTraceIds,
    setNewTraceIds,
    hasOtelConfigured,
    isQueryLoading,
    refetch,
    isHoveredRef,
    flushPendingTraces,
  } = useTraceData({
    filterParams,
    showSystem,
    debouncedSearch,
    isPaused,
  })

  // Span activity per trace (start + close with engine live_spans on): keeps
  // the strip's bars live/growing while a trace is still doing work (the rows
  // above only carry the root span, which for queue-triggered traces ends
  // instantly).
  const traceActivity = useTraceActivity(isPaused)

  const totalPages = Math.max(
    1,
    Math.ceil(traceGroups.length / filterState.pageSize),
  )
  const start = (filterState.page - 1) * filterState.pageSize
  const paged = traceGroups.slice(start, start + filterState.pageSize)

  useEffect(() => {
    if (filterState.page > totalPages) updateFilter('page', totalPages)
  }, [filterState.page, totalPages, updateFilter])

  const stats = useMemo(
    () => ({
      totalTraces: traceGroups.length,
      errorCount: traceGroups.filter((t) => t.status === 'error').length,
      avgDuration:
        traceGroups.length > 0
          ? traceGroups.reduce((sum, t) => sum + (t.duration ?? 0), 0) /
            traceGroups.length
          : 0,
    }),
    [traceGroups],
  )

  const containerRef = useRef<HTMLDivElement>(null)
  const spanPanel = useSpanPanelResize(containerRef)

  // Live detail: the selected trace's spans are seeded once (one flat read),
  // then APPENDED from the engine `trace-spans` stream and rebuilt into the
  // waterfall via `toWaterfallData`. Accumulated by `span_id` so re-delivered
  // spans dedupe. Frozen while paused.
  const detailSpansRef = useRef<Map<string, StoredSpan>>(new Map())
  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])

  const [hasPendingSpans, setHasPendingSpans] = useState(false)

  const rebuildDetail = useCallback((traceId: string): WaterfallData | null => {
    const wf = toWaterfallData([...detailSpansRef.current.values()], traceId)
    if (wf) setWaterfallData(wf)
    setHasPendingSpans(wf?.spans.some((s) => s.pending) ?? false)
    return wf
  }, [])

  // While any span of the open trace is still running, re-derive the
  // waterfall every second so live bars and elapsed durations keep growing
  // between stream frames (the transform measures pendings against "now").
  useEffect(() => {
    if (!hasPendingSpans || !selectedTraceId || isPaused) return
    const timer = setInterval(() => rebuildDetail(selectedTraceId), 1000)
    return () => clearInterval(timer)
  }, [hasPendingSpans, selectedTraceId, isPaused, rebuildDetail])

  const loadTraceSpans = useCallback(
    async (traceId: string, opts?: { silent?: boolean }) => {
      const silent = opts?.silent ?? false
      if (!silent) {
        setIsLoadingSpans(true)
        setSpansError(null)
        setWaterfallData(null)
      }
      try {
        const { spans } = await fetchTraces({
          trace_id: traceId,
          search_all_spans: true,
          include_internal: false,
          limit: 10000,
        })
        detailSpansRef.current = new Map(spans.map((s) => [s.span_id, s]))
        const wf = rebuildDetail(traceId)
        if (!wf && !silent) {
          setSpansError('no span data available for this trace')
        }
      } catch (err) {
        if (!silent) {
          setSpansError(
            err instanceof Error ? err.message : 'failed to load trace',
          )
        }
      } finally {
        if (!silent) setIsLoadingSpans(false)
      }
    },
    [rebuildDetail],
  )

  const appendDetailSpans = useCallback(
    (traceId: string, spans: StoredSpan[]) => {
      if (spans.length === 0) return
      for (const s of spans) mergeDetailSpan(detailSpansRef.current, s)
      rebuildDetail(traceId)
    },
    [rebuildDetail],
  )

  useEffect(() => {
    if (!selectedTraceId) return
    let stop: (() => void) | undefined
    let active = true
    void (async () => {
      const client = await getIiiClient()
      if (!active) return
      stop = startTraceSpansStream(client, selectedTraceId, (spans) => {
        if (!active || isPausedRef.current) return
        appendDetailSpans(selectedTraceId, spans)
      })
    })()
    return () => {
      active = false
      stop?.()
    }
  }, [selectedTraceId, appendDetailSpans])

  const selectTrace = useCallback(
    (traceId: string | null) => {
      setSelectedTraceId(traceId)
      setSelectedSpan(null)
      setWaterfallData(null)
      setSpansError(null)
      detailSpansRef.current = new Map()
      if (traceId) {
        loadTraceSpans(traceId)
      }
    },
    [loadTraceSpans],
  )

  // Deep-link seed: mount straight into a trace's detail (used by stories).
  const initialAppliedRef = useRef(false)
  useEffect(() => {
    if (initialAppliedRef.current || !initialTraceId) return
    initialAppliedRef.current = true
    selectTrace(initialTraceId)
  }, [initialTraceId, selectTrace])

  // Esc walks back out: span panel first, then the full-canvas detail.
  useEffect(() => {
    if (!selectedTraceId) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (selectedSpan) setSelectedSpan(null)
      else selectTrace(null)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [selectedTraceId, selectedSpan, selectTrace])

  const isDetailOpen = selectedTraceId !== null

  return (
    <section className="flex-1 flex flex-col overflow-hidden">
      <TimelineStrip
        traces={traceGroups}
        activity={traceActivity}
        isPaused={isPaused}
        showSystem={showSystem}
        isLoading={isQueryLoading}
        onTogglePause={() => setIsPaused((v) => !v)}
        onToggleSystem={() => setShowSystem((v) => !v)}
        onRefresh={() => refetch()}
        onTraceClick={(traceId) =>
          selectTrace(traceId === selectedTraceId ? null : traceId)
        }
        selectedTraceId={selectedTraceId}
      />

      {/* the filter bar only steers the list — hidden while a detail fills
          the canvas */}
      {!isDetailOpen && (
        <div className="px-4 py-2.5 border-b border-rule">
          <ErrorBoundary>
            <TraceFilters
              filters={filterState}
              onFilterChange={updateFilter}
              onClear={resetFilters}
              validationWarnings={validationWarnings}
              onClearWarnings={clearValidationWarnings}
              isLoading={isQueryLoading}
              searchQuery={searchQuery}
              onSearchChange={handleSearchChange}
              stats={hasOtelConfigured ? stats : undefined}
            />
          </ErrorBoundary>
        </div>
      )}

      <ErrorBoundary>
        {!hasOtelConfigured && !isDetailOpen ? (
          <div className="p-9">
            <Cell title="no observability">
              this engine does not have the trace exporter registered. configure
              the engine with the otel/memory exporter to start capturing
              traces.
            </Cell>
          </div>
        ) : (
          <div className="flex-1 flex overflow-hidden" ref={containerRef}>
            {selectedTraceId !== null ? (
              <>
                {/* detail fills the entire trace canvas */}
                <div className="flex-1 min-w-0 bg-bg flex flex-col h-full overflow-hidden">
                  {isLoadingSpans && (
                    <div className="p-4 flex flex-col gap-2">
                      {(
                        [
                          'sp-sk-0',
                          'sp-sk-1',
                          'sp-sk-2',
                          'sp-sk-3',
                          'sp-sk-4',
                        ] as const
                      ).map((sk) => (
                        <Skeleton key={sk} className="h-6 w-full" />
                      ))}
                    </div>
                  )}
                  {!isLoadingSpans && spansError && (
                    <div className="p-4 flex flex-col gap-3">
                      <StatusPanel
                        variant="alert"
                        icon={<AlertCircle className="w-full h-full" />}
                        headline="failed to load trace"
                        detail={spansError}
                      />
                      <div className="flex items-center gap-2">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => loadTraceSpans(selectedTraceId)}
                        >
                          <RefreshCw className="w-3 h-3" />
                          retry
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => selectTrace(null)}
                        >
                          back to list
                        </Button>
                      </div>
                    </div>
                  )}
                  {!isLoadingSpans && !spansError && waterfallData && (
                    <>
                      <TraceHeader
                        data={waterfallData}
                        traceId={selectedTraceId}
                        onClose={() => selectTrace(null)}
                        onSpanClick={setSelectedSpan}
                      />
                      <div className="border-b border-rule px-4 py-2.5">
                        <ViewSwitcher
                          currentView={activeView}
                          onViewChange={setActiveView}
                        />
                      </div>
                      <div className="flex-1 overflow-auto min-h-0">
                        {activeView === 'timeline' && (
                          <TraceTimeline
                            data={waterfallData}
                            onSpanClick={setSelectedSpan}
                            selectedSpanId={selectedSpan?.span_id}
                            spanGroupKey={traceSpanGroupKey}
                          />
                        )}
                        {activeView === 'waterfall' && (
                          <WaterfallChart
                            data={waterfallData}
                            onSpanClick={setSelectedSpan}
                            selectedSpanId={selectedSpan?.span_id}
                          />
                        )}
                      </div>
                      <div className="border-t border-rule flex-shrink-0">
                        <ServiceBreakdown data={waterfallData} />
                      </div>
                    </>
                  )}
                </div>

                {selectedSpan && waterfallData ? (
                  <>
                    {/* biome-ignore lint/a11y/useSemanticElements: <hr> is not draggable; this is a separator drag handle */}
                    <div
                      role="separator"
                      aria-orientation="vertical"
                      aria-valuenow={spanPanel.width}
                      aria-valuemin={spanPanel.min}
                      aria-valuemax={spanPanel.max}
                      tabIndex={0}
                      aria-label="resize span panel"
                      onMouseDown={spanPanel.startResize}
                      onDoubleClick={spanPanel.reset}
                      className="w-[3px] flex-shrink-0 cursor-col-resize bg-rule hover:bg-accent active:bg-accent"
                    />
                    <div
                      style={{ width: spanPanel.width }}
                      className={cn(
                        'bg-bg border-l border-rule flex-shrink-0 h-full overflow-hidden',
                        spanPanel.isResizing &&
                          'pointer-events-none select-none',
                      )}
                    >
                      <SpanPanel
                        span={liveSelectedSpan ?? selectedSpan}
                        traceData={waterfallData}
                        onClose={() => setSelectedSpan(null)}
                        onNavigateToSpan={setSelectedSpan}
                        onNavigateToTrace={selectTrace}
                      />
                    </div>
                  </>
                ) : null}
              </>
            ) : (
              <div className="flex flex-col flex-1 overflow-hidden">
                {isQueryLoading && traceGroups.length === 0 ? (
                  <div className="flex flex-col">
                    {(
                      [
                        'tr-sk-0',
                        'tr-sk-1',
                        'tr-sk-2',
                        'tr-sk-3',
                        'tr-sk-4',
                      ] as const
                    ).map((sk) => (
                      <div
                        key={sk}
                        className="px-4 py-3 border-b border-rule-2"
                      >
                        <div className="flex items-center gap-2 mb-2">
                          <Skeleton className="w-1.5 h-1.5 rounded-full" />
                          <Skeleton className="h-3.5 w-48" />
                        </div>
                        <div className="flex items-center gap-3">
                          <Skeleton className="h-3 w-16" />
                          <Skeleton className="h-3 w-12" />
                          <Skeleton className="h-3 w-20" />
                        </div>
                      </div>
                    ))}
                  </div>
                ) : traceGroups.length === 0 ? (
                  <div className="p-9">
                    <EmptyState
                      icon={GitBranch}
                      title="no traces recorded"
                      description="traces appear here when functions execute. fire a request to your engine and refresh."
                    />
                  </div>
                ) : (
                  <div className="flex-1 flex flex-col overflow-hidden">
                    {/* biome-ignore lint/a11y/noStaticElementInteractions: hover detection for pause/resume of live updates */}
                    <div
                      className="flex-1 overflow-y-auto"
                      onMouseEnter={() => {
                        isHoveredRef.current = true
                      }}
                      onMouseLeave={() => {
                        isHoveredRef.current = false
                        flushPendingTraces()
                      }}
                    >
                      {paged.map((trace) => (
                        <TraceListRow
                          key={trace.traceId}
                          trace={trace}
                          isSelected={selectedTraceId === trace.traceId}
                          isNew={newTraceIds.has(trace.traceId)}
                          onSelect={() =>
                            selectTrace(
                              selectedTraceId === trace.traceId
                                ? null
                                : trace.traceId,
                            )
                          }
                          onAnimationEnd={() => {
                            if (newTraceIds.has(trace.traceId))
                              setNewTraceIds((prev) => {
                                const next = new Set(prev)
                                next.delete(trace.traceId)
                                return next
                              })
                          }}
                        />
                      ))}
                    </div>
                    <div className="flex-shrink-0 border-t border-rule px-4 py-2.5">
                      <Pagination
                        currentPage={filterState.page}
                        totalPages={totalPages}
                        totalItems={traceGroups.length}
                        pageSize={filterState.pageSize}
                        onPageChange={(p) => updateFilter('page', p)}
                        onPageSizeChange={(s) => {
                          updateFilter('pageSize', s)
                          updateFilter('page', 1)
                        }}
                        pageSizeOptions={PAGE_SIZES}
                      />
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        )}
      </ErrorBoundary>
    </section>
  )
}
