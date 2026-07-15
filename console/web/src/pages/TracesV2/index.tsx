/**
 * TracesV2 — the isolated "core experience" copy of the Traces page shell.
 *
 * Composition (this pass): the live TimelineStrip is the masthead — it
 * replaced the `$ traces` page header and carries the system/pause/refresh
 * actions in its header row. Below it: filter bar → trace list → pagination.
 *
 * The trace detail is an ACCORDION item inside the list: clicking a row
 * expands the detail (header, view switcher, timeline/waterfall, workers
 * footer) right beneath it, no animation; clicking another row closes the
 * previous detail and opens the new one. The scroll position is anchored to
 * the clicked row across that swap, so collapsing a tall detail above it
 * doesn't yank the list. A trace opened without a visible row (timeline
 * strip click, deep link before rows land) renders the same detail pinned
 * at the top of the scroll area instead. Clicking a span opens the
 * resizable span panel on the right; Esc walks back (span panel first,
 * then the expanded detail). The strip stays live up top.
 *
 * The session-detail panel and the map/flow visualizations are NOT copied
 * here (out of scope). The live-detail stream wiring is faithful — in
 * Storybook the fake iii-client makes the stream a no-op and serves the
 * seed read from fixtures.
 */

import { AlertCircle, GitBranch, RefreshCw } from 'lucide-react'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/ui/Button'
import { Cell } from '@/components/ui/Cell'
import { EmptyState } from '@/components/ui/EmptyState'
import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import { Pagination } from '@/components/ui/Pagination'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import { loadFollowTurns, saveFollowTurns } from '@/lib/storage'
import { startTraceSpansStream } from '@/lib/traces-stream'
import { cn } from '@/lib/utils'
import { fetchTraces, type StoredSpan } from './api/traces'
import { GroupedTraceList } from './components/GroupedTraceList'
import { SpanPanel } from './components/SpanPanel'
import { TraceDetailSkeleton } from './components/TraceDetailSkeleton'
import { TraceFilters } from './components/TraceFilters'
import { TraceHeader } from './components/TraceHeader'
import { TraceListRow } from './components/TraceListRow'
import { TimelineStrip } from './components/timeline/TimelineStrip'
import { TraceTimeline } from './components/timeline/TraceTimeline'
import { ViewSwitcher, type ViewType } from './components/ViewSwitcher'
import { ViewsDropdown } from './components/ViewsDropdown'
import { WaterfallChart } from './components/WaterfallChart'
import { WorkerBreakdown } from './components/WorkerBreakdown'
import { useAllSpans } from './hooks/useAllSpans'
import { useFollowLiveTurn } from './hooks/useFollowLiveTurn'
import { useSpanFilterSelection } from './hooks/useSpanFilterSelection'
import { useSpanPanelResize } from './hooks/useSpanPanelResize'
import { useTraceActivity } from './hooks/useTraceActivity'
import { useTraceData } from './hooks/useTraceData'
import { useTraceFilters } from './hooks/useTraceFilters'
import { useTraceViews } from './hooks/useTraceViews'
import { isTraceLive } from './lib/timelineSpans'
import type { RowLabelConfig } from './lib/traceListItem'
import {
  applyViewConfig,
  captureViewConfig,
  type TracesView,
  viewConfigEquals,
} from './lib/tracesViews'
import { traceSpanGroupKey } from './lib/traceTimelineFilters'
import {
  mergeDetailSpan,
  toWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from './lib/traceTransform'

const PAGE_SIZES = [25, 50, 100]

export interface TracesV2Props {
  /** mount with a trace's detail already expanded (stories/deep links) */
  initialTraceId?: string
}

export function TracesV2({ initialTraceId }: TracesV2Props) {
  // The strip's system/pause/refresh controls were removed; both stay at
  // their defaults (streams live, internal spans hidden) until some other
  // surface grows a toggle.
  const [showSystem] = useState(false)
  const [isPaused] = useState(false)
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
    replaceFilters,
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
    isHoveredRef,
    flushPendingTraces,
  } = useTraceData({
    filterParams,
    showSystem,
    debouncedSearch,
    isPaused,
    hiddenFunctions: filterState.hiddenFunctions,
  })

  // ── Saved views (server-persisted via the `console` configuration entry) ──
  const {
    views,
    available: viewsAvailable,
    activeViewId,
    setActiveViewId,
    saveView,
    updateView,
    renameView,
    deleteView,
  } = useTraceViews()

  // ── Detail-view span filter (hidden span groups + workers) ──
  // One selection shared by the timeline AND waterfall tabs, persisted in
  // the console configuration alongside the saved views.
  const spanFilter = useSpanFilterSelection()

  const activeSavedView = views.find((v) => v.id === activeViewId) ?? null
  const activeViewModified = useMemo(
    () =>
      activeSavedView
        ? !viewConfigEquals(captureViewConfig(filterState), activeSavedView)
        : false,
    [activeSavedView, filterState],
  )

  const handleSelectView = useCallback(
    (view: TracesView | null) => {
      setActiveViewId(view?.id ?? null)
      if (view) replaceFilters(applyViewConfig(view))
      else resetFilters()
    },
    [setActiveViewId, replaceFilters, resetFilters],
  )

  // Re-apply the remembered view once its definition arrives after a page
  // load — filters are still pristine at that point, so this can't clobber
  // anything the user already changed.
  const appliedInitialViewRef = useRef(false)
  useEffect(() => {
    if (appliedInitialViewRef.current) return
    if (!activeViewId || views.length === 0) return
    appliedInitialViewRef.current = true
    const view = views.find((v) => v.id === activeViewId)
    if (view) replaceFilters(applyViewConfig(view))
    else setActiveViewId(null)
  }, [activeViewId, views, replaceFilters, setActiveViewId])

  const hideFunction = useCallback(
    (functionId: string) => {
      const current = filterState.hiddenFunctions ?? []
      if (current.includes(functionId)) return
      updateFilter('hiddenFunctions', [...current, functionId])
    },
    [filterState.hiddenFunctions, updateFilter],
  )

  const rowLabel: RowLabelConfig | undefined = useMemo(
    () =>
      filterState.labelMode && filterState.labelMode !== 'function'
        ? {
            mode: filterState.labelMode,
            attribute: filterState.labelAttribute,
          }
        : undefined,
    [filterState.labelMode, filterState.labelAttribute],
  )

  // Every attribute key observed on loaded traces (row attributes + merged
  // trace tags) feeds the group-by picker — grouping accepts ANY key, these
  // just make the known ones one click. iii.* identity/tag keys sort first.
  const attributeKeySuggestions = useMemo(() => {
    const keys = new Set<string>()
    for (const trace of traceGroups) {
      for (const key of Object.keys(trace.traceTags ?? {})) keys.add(key)
      for (const key of Object.keys(trace.attributes ?? {})) keys.add(key)
    }
    return [...keys].sort((a, b) => {
      const aIii = a.startsWith('iii.') ? 0 : 1
      const bIii = b.startsWith('iii.') ? 0 : 1
      return aIii - bIii || a.localeCompare(b)
    })
  }, [traceGroups])

  const groupByAttribute =
    filterState.groupBy && filterState.groupBy !== 'none'
      ? filterState.groupBy
      : null

  // Span activity per trace (start + close with engine live_spans on): keeps
  // the strip's bars live/growing while a trace is still doing work (the rows
  // above only carry the root span, which for queue-triggered traces ends
  // instantly).
  const traceActivity = useTraceActivity(isPaused)

  // The masthead strip's data: every span across all traces (seed + the
  // engine's all-spans stream), one bar per span. Internal spans that are
  // part of a real trace (built-in calls inside a turn) are included —
  // hiding them is the funnel/trace_hidden layer's job; `showSystem` only
  // governs the trace LIST's internal ROOT rows.
  const allSpans = useAllSpans(isPaused)

  // Follow toggle (strip header): auto-open the trace of the ACTIVE chat's
  // live turn, so sending a message lands the canvas on the work as it runs.
  // Scoped to user interactions — the active conversation's top-level
  // `harness.turn` steps; sub-agent turns never steal the view. Null outside
  // the app shell (Storybook), which also hides the strip's toggle.
  const conversationsCtx = useConversationsCtxOptional()
  const [followTurns, setFollowTurns] = useState(loadFollowTurns)
  const toggleFollowTurns = useCallback(() => {
    setFollowTurns((on) => {
      saveFollowTurns(!on)
      return !on
    })
  }, [])

  const totalPages = Math.max(
    1,
    Math.ceil(traceGroups.length / filterState.pageSize),
  )
  const start = (filterState.page - 1) * filterState.pageSize
  const paged = traceGroups.slice(start, start + filterState.pageSize)

  // Liveness evaluation instant for the list rows' pulsing dot — only ticked
  // while a visible row is actually live, mirroring the strip's clock
  // (TimelineStrip.tsx) so an idle list renders exactly as often as its data
  // changes.
  const [listNow, setListNow] = useState(() => Date.now())
  const liveTraceIds = useMemo(() => {
    const ids = new Set<string>()
    for (const t of paged) {
      if (isTraceLive(t, { activity: traceActivity, now: listNow })) {
        ids.add(t.traceId)
      }
    }
    return ids
  }, [paged, traceActivity, listNow])
  const anyRowLive = liveTraceIds.size > 0
  useEffect(() => {
    if (!anyRowLive) return
    const id = setInterval(() => setListNow(Date.now()), 500)
    return () => clearInterval(id)
  }, [anyRowLive])

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

  // ── Scroll anchoring across accordion swaps ──
  // Selecting a trace while another's detail is expanded ABOVE it removes a
  // tall block from the scroll flow; without compensation the clicked row
  // jumps up (or out of view). Before the state change we record the target
  // row's offset inside the scroll container; after React commits, the
  // layout effect scrolls by however far the row moved, so the row the user
  // clicked stays put on screen. Closing anchors to the closing row.
  const listScrollRef = useRef<HTMLDivElement | null>(null)
  const scrollAnchorRef = useRef<{ traceId: string; top: number } | null>(null)

  const captureScrollAnchor = useCallback((traceId: string | null) => {
    scrollAnchorRef.current = null
    const container = listScrollRef.current
    if (!container || !traceId) return
    const row = container.querySelector(
      `[data-trace-row-id="${CSS.escape(traceId)}"]`,
    )
    if (!row) return
    scrollAnchorRef.current = {
      traceId,
      top:
        row.getBoundingClientRect().top - container.getBoundingClientRect().top,
    }
  }, [])

  // No dependency array: the pending anchor is consumed on the first commit
  // after `selectTrace` sets it (the selection re-render), and every other
  // commit exits on the null check.
  useLayoutEffect(() => {
    const anchor = scrollAnchorRef.current
    if (!anchor) return
    scrollAnchorRef.current = null
    const container = listScrollRef.current
    if (!container) return
    const row = container.querySelector(
      `[data-trace-row-id="${CSS.escape(anchor.traceId)}"]`,
    )
    if (!row) return
    const delta =
      row.getBoundingClientRect().top -
      container.getBoundingClientRect().top -
      anchor.top
    if (delta !== 0) container.scrollTop += delta
  })

  const selectedTraceIdRef = useRef(selectedTraceId)
  selectedTraceIdRef.current = selectedTraceId

  const selectTrace = useCallback(
    (traceId: string | null) => {
      // opening/switching anchors the clicked row; closing anchors the row
      // whose detail is about to collapse
      captureScrollAnchor(traceId ?? selectedTraceIdRef.current)
      setSelectedTraceId(traceId)
      setSelectedSpan(null)
      setWaterfallData(null)
      setSpansError(null)
      detailSpansRef.current = new Map()
      if (traceId) {
        loadTraceSpans(traceId)
      }
    },
    [loadTraceSpans, captureScrollAnchor],
  )

  const toggleTrace = useCallback(
    (traceId: string) => {
      selectTrace(traceId === selectedTraceIdRef.current ? null : traceId)
    },
    [selectTrace],
  )

  // The follow toggle's engine: opens the newest live turn trace of the
  // active conversation (once per trace — closing it mid-turn is respected).
  useFollowLiveTurn({
    enabled: followTurns && !isPaused,
    activeSessionId: conversationsCtx?.activeId ?? null,
    spans: allSpans,
    selectedTraceId,
    onOpenTrace: selectTrace,
  })

  // Deep-link seed: mount with a trace already expanded (used by stories).
  const initialAppliedRef = useRef(false)
  useEffect(() => {
    if (initialAppliedRef.current || !initialTraceId) return
    initialAppliedRef.current = true
    selectTrace(initialTraceId)
  }, [initialTraceId, selectTrace])

  // Esc walks back out: span panel first, then the expanded detail.
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

  // The expanded detail, rendered as an accordion item under the selected
  // row (or pinned at the top of the scroll area when that row isn't in the
  // current page — timeline-strip clicks, deep links). The left accent
  // border continues the selected row's marker.
  const traceDetail =
    selectedTraceId !== null ? (
      <div className="border-b border-rule-2 border-l-2 border-l-accent">
        {isLoadingSpans && (
          <TraceDetailSkeleton onClose={() => selectTrace(null)} />
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
                close
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
            {/* the timeline sizes itself to its packed lines (fitContent);
                the waterfall is virtualized and needs a bounded scrollport */}
            {activeView === 'timeline' && (
              <TraceTimeline
                data={waterfallData}
                onSpanClick={setSelectedSpan}
                selectedSpanId={selectedSpan?.span_id}
                spanGroupKey={traceSpanGroupKey}
                spanFilter={spanFilter}
                fitContent
              />
            )}
            {activeView === 'waterfall' && (
              <div className="h-[420px]">
                <WaterfallChart
                  data={waterfallData}
                  onSpanClick={setSelectedSpan}
                  selectedSpanId={selectedSpan?.span_id}
                  spanGroupKey={traceSpanGroupKey}
                  spanFilter={spanFilter}
                />
              </div>
            )}
            <div className="border-t border-rule">
              <WorkerBreakdown data={waterfallData} />
            </div>
          </>
        )}
      </div>
    ) : null

  const selectedInPage =
    selectedTraceId !== null && paged.some((t) => t.traceId === selectedTraceId)

  return (
    <section className="flex-1 flex flex-col overflow-hidden">
      <TimelineStrip
        spans={allSpans}
        spanFilter={spanFilter}
        isPaused={isPaused}
        onTraceClick={toggleTrace}
        followTurns={followTurns}
        onToggleFollowTurns={conversationsCtx ? toggleFollowTurns : undefined}
      />

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
            attributeKeySuggestions={attributeKeySuggestions}
            leading={
              viewsAvailable ? (
                <ViewsDropdown
                  views={views}
                  activeViewId={activeViewId}
                  activeModified={activeViewModified}
                  onSelectView={handleSelectView}
                  onSaveNew={(name) => {
                    void saveView(name, captureViewConfig(filterState)).then(
                      (view) => setActiveViewId(view.id),
                    )
                  }}
                  onUpdateActive={() => {
                    if (activeViewId)
                      void updateView(
                        activeViewId,
                        captureViewConfig(filterState),
                      )
                  }}
                  onRenameActive={(name) => {
                    if (activeViewId) void renameView(activeViewId, name)
                  }}
                  onDeleteActive={() => {
                    // deleteView clears the selection itself, in the same
                    // config write.
                    if (activeViewId) void deleteView(activeViewId)
                  }}
                />
              ) : undefined
            }
          />
        </ErrorBoundary>
      </div>

      <ErrorBoundary>
        {!hasOtelConfigured ? (
          <div className="p-9">
            <Cell title="no observability">
              this engine does not have the trace exporter registered. configure
              the engine with the otel/memory exporter to start capturing
              traces.
            </Cell>
          </div>
        ) : (
          <div className="flex-1 flex overflow-hidden" ref={containerRef}>
            {groupByAttribute ? (
              <div className="flex-1 overflow-y-auto" ref={listScrollRef}>
                <GroupedTraceList
                  attribute={groupByAttribute}
                  showSystem={showSystem}
                  hiddenFunctions={filterState.hiddenFunctions}
                  label={rowLabel}
                  selectedTraceId={selectedTraceId}
                  onSelectTrace={toggleTrace}
                  onHideFunction={hideFunction}
                  expandedContent={traceDetail}
                />
              </div>
            ) : (
              <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
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
                      ref={listScrollRef}
                      onMouseEnter={() => {
                        isHoveredRef.current = true
                      }}
                      onMouseLeave={() => {
                        isHoveredRef.current = false
                        flushPendingTraces()
                      }}
                    >
                      {/* detail whose row is on another page (strip click,
                          deep link) pins to the top of the scroll area */}
                      {selectedTraceId !== null &&
                        !selectedInPage &&
                        traceDetail}
                      {paged.map((trace) => {
                        const isExpanded = selectedTraceId === trace.traceId
                        return (
                          <div
                            key={trace.traceId}
                            data-trace-row-id={trace.traceId}
                          >
                            <TraceListRow
                              trace={trace}
                              isSelected={isExpanded}
                              isNew={newTraceIds.has(trace.traceId)}
                              isLive={liveTraceIds.has(trace.traceId)}
                              label={rowLabel}
                              onHideFunction={hideFunction}
                              onSelect={() => toggleTrace(trace.traceId)}
                              onAnimationEnd={() => {
                                if (newTraceIds.has(trace.traceId))
                                  setNewTraceIds((prev) => {
                                    const next = new Set(prev)
                                    next.delete(trace.traceId)
                                    return next
                                  })
                              }}
                            />
                            {isExpanded && traceDetail}
                          </div>
                        )
                      })}
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
                    spanPanel.isResizing && 'pointer-events-none select-none',
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
          </div>
        )}
      </ErrorBoundary>
    </section>
  )
}
