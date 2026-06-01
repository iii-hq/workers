import {
  AlertCircle,
  Eye,
  EyeOff,
  GitBranch,
  Pause,
  Play,
  RefreshCw,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { Cell } from '@/components/ui/Cell'
import { EmptyState } from '@/components/ui/EmptyState'
import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import { Pagination } from '@/components/ui/Pagination'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useTracesLiveRefresh } from '@/lib/devtools-stream'
import { cn } from '@/lib/utils'
import { fetchTraceTree, type TraceGroup } from './api/traces'
import { FlameGraph } from './components/FlameGraph'
import { FlowView } from './components/FlowView'
import { ServiceBreakdown } from './components/ServiceBreakdown'
import { SessionDetailPanel } from './components/SessionDetailPanel'
import { SpanPanel } from './components/SpanPanel'
import { TraceFilters } from './components/TraceFilters'
import { TraceGroupsView } from './components/TraceGroupsView'
import { TraceHeader } from './components/TraceHeader'
import { TraceListRow } from './components/TraceListRow'
import { TraceMap } from './components/TraceMap'
import { ViewSwitcher, type ViewType } from './components/ViewSwitcher'
import { WaterfallChart } from './components/WaterfallChart'
import { useResizablePanels } from './hooks/useResizablePanels'
import { useTraceData } from './hooks/useTraceData'
import { useTraceFilters } from './hooks/useTraceFilters'
import {
  treeToWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from './lib/traceTransform'

const PAGE_SIZES = [25, 50, 100]

export function Traces() {
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
  const [selectedGroup, setSelectedGroup] = useState<TraceGroup | null>(null)
  const [activeView, setActiveView] = useState<ViewType>('waterfall')
  const [waterfallData, setWaterfallData] = useState<WaterfallData | null>(null)
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
    queryError,
    refetch,
    isHoveredRef,
    flushPendingTraces,
  } = useTraceData({
    filterParams,
    showSystem,
    debouncedSearch,
  })

  // Replace polling with iii push: refetch the trace queries when the harness
  // signals new spans (ui::traces::changed), suspended while paused.
  useTracesLiveRefresh({ isPaused })

  const totalPages = Math.max(
    1,
    Math.ceil(traceGroups.length / filterState.pageSize),
  )
  const start = (filterState.page - 1) * filterState.pageSize
  const paged = traceGroups.slice(start, start + filterState.pageSize)

  useEffect(() => {
    if (filterState.page > totalPages) updateFilter('page', totalPages)
  }, [filterState.page, totalPages, updateFilter])

  useEffect(() => {
    if (!filterState.groupBy || filterState.groupBy === 'none') {
      setSelectedGroup(null)
    }
  }, [filterState.groupBy])

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
  const {
    panelWidths,
    panelMaxes,
    panelMin,
    isResizing,
    startResize,
    resetTracePanel,
    resetSpanPanel,
  } = useResizablePanels({
    selectedSpanId: selectedSpan?.span_id ?? null,
    containerRef,
  })

  const loadTraceSpans = useCallback(async (traceId: string) => {
    setIsLoadingSpans(true)
    setSpansError(null)
    setWaterfallData(null)
    try {
      const data = await fetchTraceTree(traceId)
      if (data.roots?.length) {
        const wf = treeToWaterfallData(data.roots)
        if (wf) setWaterfallData(wf)
        else setSpansError('failed to process span data')
      } else {
        setSpansError('no span data available for this trace')
      }
    } catch (err) {
      setSpansError(err instanceof Error ? err.message : 'failed to load trace')
    } finally {
      setIsLoadingSpans(false)
    }
  }, [])

  // Live updates are paused while a detail panel is open so the list doesn't
  // reorder under the user. Track whether WE auto-paused (vs the user manually
  // pausing) so closing the panel restores the prior intent instead of leaving
  // the list frozen forever (the old behaviour: select → pause → close →
  // permanently stuck, since polling is gone and pause is the only throttle).
  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])
  const autoPausedRef = useRef(false)

  const autoPause = useCallback(() => {
    if (!isPausedRef.current) {
      autoPausedRef.current = true
      setIsPaused(true)
    }
  }, [])

  const autoResume = useCallback(() => {
    if (autoPausedRef.current) {
      autoPausedRef.current = false
      setIsPaused(false)
    }
  }, [])

  const togglePause = useCallback(() => {
    // Manual toggle overrides the auto-pause bookkeeping.
    autoPausedRef.current = false
    setIsPaused((v) => !v)
  }, [])

  const selectTrace = useCallback(
    (traceId: string | null) => {
      setSelectedGroup(null)
      setSelectedTraceId(traceId)
      setSelectedSpan(null)
      setWaterfallData(null)
      setSpansError(null)
      if (traceId) {
        autoPause()
        loadTraceSpans(traceId)
      } else {
        autoResume()
      }
    },
    [loadTraceSpans, autoPause, autoResume],
  )

  // Group-row selection: open the session detail panel ONLY. The panel
  // (SessionDetailPanel) owns its own per-trace tree fetches, so we must not
  // route through `selectTrace` here — doing so fired a wasted
  // `engine::traces::tree` RPC for the group's first trace and read trace
  // detail from the flat-list query (a different data source).
  const selectGroup = useCallback(
    (group: TraceGroup) => {
      setSelectedGroup(group)
      setSelectedTraceId(null)
      setSelectedSpan(null)
      setWaterfallData(null)
      setSpansError(null)
      autoPause()
    },
    [autoPause],
  )

  const closeDetail = useCallback(() => {
    setSelectedGroup(null)
    setSelectedTraceId(null)
    setSelectedSpan(null)
    setWaterfallData(null)
    setSpansError(null)
    autoResume()
  }, [autoResume])

  const groupAttribute =
    filterState.groupBy && filterState.groupBy !== 'none'
      ? filterState.groupBy
      : null

  const selectedTrace = traceGroups.find((t) => t.traceId === selectedTraceId)

  return (
    <section className="flex-1 flex flex-col overflow-hidden">
      <div className="px-9 py-6 border-b border-rule flex items-end justify-between flex-wrap gap-4">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-3">
            <span className="text-accent">$</span>
            <span className="text-ink ml-2">traces</span>
          </div>
          <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase inline-flex items-center gap-3">
            traces
            {isPaused ? (
              <Badge variant="warn">
                <Pause className="w-3 h-3" />
                paused
              </Badge>
            ) : null}
          </h1>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant={showSystem ? 'pill' : 'ghost'}
            size="sm"
            onClick={() => setShowSystem((v) => !v)}
          >
            {showSystem ? (
              <Eye className="w-3.5 h-3.5" />
            ) : (
              <EyeOff className="w-3.5 h-3.5" />
            )}
            <span className={cn(showSystem ? '' : 'line-through opacity-60')}>
              system
            </span>
          </Button>
          <Button
            variant={isPaused ? 'pill' : 'ghost'}
            size="sm"
            onClick={togglePause}
          >
            {isPaused ? (
              <Play className="w-3.5 h-3.5" />
            ) : (
              <Pause className="w-3.5 h-3.5" />
            )}
            <span>{isPaused ? 'resume' : 'pause'}</span>
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => refetch()}
            disabled={isQueryLoading}
          >
            <RefreshCw
              className={cn('w-3.5 h-3.5', isQueryLoading && 'animate-spin')}
            />
            <span>refresh</span>
          </Button>
        </div>
      </div>

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

      <ErrorBoundary>
        {queryError ? (
          <div className="p-9 flex flex-col gap-3">
            <StatusPanel
              variant="alert"
              icon={<AlertCircle className="w-full h-full" />}
              headline="failed to load traces"
              detail={queryError.message}
            />
            <div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => refetch()}
                disabled={isQueryLoading}
              >
                <RefreshCw
                  className={cn('w-3 h-3', isQueryLoading && 'animate-spin')}
                />
                retry
              </Button>
            </div>
          </div>
        ) : !hasOtelConfigured && !isQueryLoading ? (
          <div className="p-9">
            <Cell title="no observability">
              this engine does not have the trace exporter registered. configure
              the engine with the otel/memory exporter to start capturing
              traces.
            </Cell>
          </div>
        ) : (
          <div className="flex-1 flex overflow-hidden" ref={containerRef}>
            <div className="flex flex-col flex-1 overflow-hidden">
              {groupAttribute ? (
                <TraceGroupsView
                  attribute={groupAttribute}
                  showSystem={showSystem}
                  selectedGroupValue={selectedGroup?.value ?? null}
                  onSelectGroup={selectGroup}
                />
              ) : isQueryLoading && traceGroups.length === 0 ? (
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
                    <div key={sk} className="px-4 py-3 border-b border-rule-2">
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

            {selectedTrace || selectedGroup ? (
              <>
                {/* biome-ignore lint/a11y/useSemanticElements: <hr> is not draggable; this is a separator drag handle */}
                <div
                  role="separator"
                  aria-orientation="vertical"
                  aria-valuenow={panelWidths.trace}
                  aria-valuemin={panelMin}
                  aria-valuemax={panelMaxes.trace}
                  tabIndex={0}
                  aria-label="resize trace panel"
                  onMouseDown={(e) => startResize(e, 'trace')}
                  onDoubleClick={resetTracePanel}
                  className="w-[3px] flex-shrink-0 cursor-col-resize bg-rule hover:bg-accent active:bg-accent"
                />
                <div
                  style={{ width: panelWidths.trace }}
                  className={cn(
                    'bg-bg flex flex-col h-full overflow-hidden flex-shrink-0 border-l border-rule',
                    isResizing && 'pointer-events-none select-none',
                  )}
                >
                  {selectedGroup ? (
                    <SessionDetailPanel
                      group={selectedGroup}
                      groupAttribute={
                        filterState.groupBy && filterState.groupBy !== 'none'
                          ? filterState.groupBy
                          : undefined
                      }
                      onClose={closeDetail}
                      onSpanClick={(span, wf) => {
                        // Carry the clicked span's OWN trace data up so the
                        // SpanPanel renders with the right neighbours — in
                        // group mode there is no single waterfallData loaded.
                        setSelectedSpan(span)
                        setWaterfallData(wf)
                      }}
                      selectedSpanId={selectedSpan?.span_id}
                    />
                  ) : selectedTrace ? (
                    <>
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
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() =>
                              loadTraceSpans(selectedTrace.traceId)
                            }
                          >
                            <RefreshCw className="w-3 h-3" />
                            retry
                          </Button>
                        </div>
                      )}
                      {!isLoadingSpans && !spansError && waterfallData && (
                        <>
                          <TraceHeader
                            data={waterfallData}
                            traceId={selectedTrace.traceId}
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
                            {activeView === 'waterfall' && (
                              <WaterfallChart
                                data={waterfallData}
                                onSpanClick={setSelectedSpan}
                                selectedSpanId={selectedSpan?.span_id}
                              />
                            )}
                            {activeView === 'flamegraph' && (
                              <FlameGraph
                                data={waterfallData}
                                onSpanClick={setSelectedSpan}
                                selectedSpanId={selectedSpan?.span_id}
                              />
                            )}
                            {activeView === 'map' && (
                              <TraceMap
                                data={waterfallData}
                                onSpanClick={setSelectedSpan}
                                selectedSpanId={selectedSpan?.span_id}
                              />
                            )}
                            {activeView === 'flow' && (
                              <FlowView
                                data={waterfallData}
                                onSpanClick={setSelectedSpan}
                                selectedSpanId={selectedSpan?.span_id}
                              />
                            )}
                          </div>
                          {activeView !== 'flow' && (
                            <div className="border-t border-rule flex-shrink-0">
                              <ServiceBreakdown data={waterfallData} />
                            </div>
                          )}
                        </>
                      )}
                    </>
                  ) : null}
                </div>

                {selectedSpan && waterfallData ? (
                  <>
                    {/* biome-ignore lint/a11y/useSemanticElements: <hr> is not draggable; this is a separator drag handle */}
                    <div
                      role="separator"
                      aria-orientation="vertical"
                      aria-valuenow={panelWidths.span}
                      aria-valuemin={panelMin}
                      aria-valuemax={panelMaxes.span}
                      tabIndex={0}
                      aria-label="resize span panel"
                      onMouseDown={(e) => startResize(e, 'span')}
                      onDoubleClick={resetSpanPanel}
                      className="w-[3px] flex-shrink-0 cursor-col-resize bg-rule hover:bg-accent active:bg-accent"
                    />
                    <div
                      style={{ width: panelWidths.span }}
                      className={cn(
                        'bg-bg border-l border-rule flex-shrink-0 h-full overflow-hidden',
                        isResizing && 'pointer-events-none select-none',
                      )}
                    >
                      <SpanPanel
                        span={selectedSpan}
                        traceData={waterfallData}
                        onClose={() => setSelectedSpan(null)}
                        onNavigateToSpan={setSelectedSpan}
                        onNavigateToTrace={selectTrace}
                      />
                    </div>
                  </>
                ) : null}
              </>
            ) : null}
          </div>
        )}
      </ErrorBoundary>
    </section>
  )
}
