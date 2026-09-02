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

import {
  AlertCircle,
  ArrowUp,
  GitBranch,
  MessageSquare,
  RefreshCw,
  X,
} from 'lucide-react'
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
import { PageHeader } from '@/components/ui/PageChrome'
import { Pagination } from '@/components/ui/Pagination'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import { requestChatMessageFocus } from '@/lib/trace-links'
import { startTraceActivityFeed } from '@/lib/traces-activity'
import { cn } from '@/lib/utils'
import type { PageCommandsApi } from '@/types/injectable-ui'
import { fetchTraceSpans, type StoredSpan } from './api/traces'
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
import { useFollowTurns } from './hooks/useFollowTurns'
import {
  showsWithoutProbe,
  useSpanFilteredTraceRows,
} from './hooks/useSpanFilteredTraceRows'
import { useSpanFilterSelection } from './hooks/useSpanFilterSelection'
import { useSpanPanelResize } from './hooks/useSpanPanelResize'
import { useTraceActivity } from './hooks/useTraceActivity'
import { useTraceData } from './hooks/useTraceData'
import { useTraceFilters } from './hooks/useTraceFilters'
import { useTraceViews } from './hooks/useTraceViews'
import { isTraceLive } from './lib/timelineSpans'
import { type TraceChatLink, traceChatLink } from './lib/traceChatLink'
import { collectTraceDetailSpans } from './lib/traceDetailPages'
import { withSessionScope } from './lib/traceFilters'
import type { RowLabelConfig } from './lib/traceListItem'
import { buildTracePageStats, tracePageCount } from './lib/tracePagination'
import {
  applyViewConfig,
  captureViewConfig,
  type TracesView,
  viewConfigEquals,
} from './lib/tracesViews'
import { traceSpanGroupKey } from './lib/traceTimelineFilters'
import {
  toWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from './lib/traceTransform'

const PAGE_SIZES = [25, 50, 100]

// Sentinel a superseded trace seed throws to abandon its own page sweep.
const superseded = new Error('trace seed superseded')

export interface TracesV2Props {
  /** mount with a trace's detail already expanded (stories/deep links) */
  initialTraceId?: string
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar: the page's verbs and keys. */
  commands?: PageCommandsApi
}

export function TracesV2({
  initialTraceId,
  onRequestClose,
  commands,
}: TracesV2Props) {
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
  const rootRef = useRef<HTMLElement>(null)
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
  // Non-null while the paged seed still sweeps: the detail is painted but
  // incomplete. Drives the header's live span counter so "still filling in"
  // is legible after the skeleton is gone.
  const [seedProgress, setSeedProgress] = useState<{
    loaded: number
    total: number
  } | null>(null)

  const {
    filters: filterState,
    updateFilter,
    resetFilters,
    replaceFilters,
    getApiParams,
    validationWarnings,
    clearValidationWarnings,
  } = useTraceFilters()

  // ── Automatic session scope ──
  // The trace list FOLLOWS the active chat: selecting a conversation scopes
  // the list to its `iii.session.id`, server-side via `withSessionScope`
  // (the identity attrs live on child spans, so the scope needs the
  // search-all-spans wire shape). Drafts have no session server-side yet and
  // never scope; outside the app shell (Storybook) there is no chat to
  // follow. Dismissable per session via the chip next to the views dropdown
  // — selecting another conversation re-scopes.
  const conversationsCtx = useConversationsCtxOptional()
  const activeConversation = conversationsCtx?.active ?? null
  const [scopeDismissedFor, setScopeDismissedFor] = useState<string | null>(
    null,
  )
  const sessionScope =
    activeConversation &&
    !activeConversation.draft &&
    activeConversation.id !== scopeDismissedFor
      ? activeConversation.id
      : null

  const filterParams = useMemo(
    () => withSessionScope(getApiParams(), sessionScope),
    [getApiParams, sessionScope],
  )

  const {
    traceGroups,
    totalTraceCount,
    newTraceIds,
    setNewTraceIds,
    hasOtelConfigured,
    isQueryLoading,
    holdRef,
    heldTraces,
    flushPendingTraces,
  } = useTraceData({
    filterParams,
    showSystem,
    debouncedSearch,
    isPaused,
    hiddenFunctions: filterState.hiddenFunctions,
    attributeProjection:
      filterState.labelMode === 'attribute' && filterState.labelAttribute
        ? [filterState.labelAttribute]
        : undefined,
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

  // Scoped to one session, grouping would collapse to a single group (and
  // the group-by aggregate path doesn't consult the scope) — render the
  // flat list instead. The view's row label (e.g. `iii.tag.message`) still
  // applies, so scoped rows read as the session's turns.
  const groupByAttribute =
    sessionScope === null &&
    filterState.groupBy &&
    filterState.groupBy !== 'none'
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
  // `harness.turn` steps; sub-agent turns never steal the view. The strip's
  // toggle hides outside the app shell (no conversationsCtx in Storybook).
  const { followTurns, toggleFollowTurns } = useFollowTurns()

  // Span-filter fallout on the LIST: a row hides while its trace has no
  // visible span left under the funnel selection (composition read from the
  // all-spans feed — see the hook). Everything list-shaped below (paging,
  // stats, empty state) works off the filtered rows; the raw `traceGroups`
  // keep feeding the group-by key suggestions above.
  const visibleTraceGroups = useSpanFilteredTraceRows(
    traceGroups,
    allSpans,
    spanFilter,
  )

  const totalPages = tracePageCount(totalTraceCount, filterState.pageSize)
  // `traceGroups` is already one server page. The local span-visibility pass
  // may hide rows from that page, but it must never slice/paginate again.
  const paged = visibleTraceGroups

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
    if (!isQueryLoading && filterState.page > totalPages) {
      updateFilter('page', totalPages)
    }
  }, [filterState.page, totalPages, updateFilter, isQueryLoading])

  const stats = useMemo(
    () => buildTracePageStats(visibleTraceGroups, totalTraceCount),
    [visibleTraceGroups, totalTraceCount],
  )

  const containerRef = useRef<HTMLDivElement>(null)
  const spanPanel = useSpanPanelResize(containerRef)

  // Live detail: the selected trace's spans are seeded on explicit open,
  // paged so a very large trace never becomes one oversized RPC response;
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

  // One seed at a time: selecting another trace supersedes the sweep in
  // flight, which must neither clobber the new trace's spans nor surface
  // its own late error. Progressive rendering makes this guard load-bearing
  // — a stale sweep now touches state on every page, not just at the end.
  const seedSeqRef = useRef(0)
  const loadTraceSpans = useCallback(
    async (traceId: string, opts?: { silent?: boolean }) => {
      const seq = ++seedSeqRef.current
      const stale = () => seedSeqRef.current !== seq
      const silent = opts?.silent ?? false
      // Newest seed owns the progress chip from the start — a superseded
      // sweep's last reading must not survive the switch.
      setSeedProgress(null)
      if (!silent) {
        setIsLoadingSpans(true)
        setSpansError(null)
        setWaterfallData(null)
      }
      try {
        // Pages sized by response bytes, not span count — an oversized page
        // hangs forever instead of erroring. See lib/traceDetailPages.
        let revealed = false
        let lastPaint = 0
        const detailSpans = await collectTraceDetailSpans(
          (offset, limit) => {
            if (stale()) throw superseded
            return fetchTraceSpans({
              trace_id: traceId,
              search_all_spans: true,
              include_internal: false,
              sort_by: 'start_time',
              sort_order: 'asc',
              offset,
              limit,
            })
          },
          {
            // Paint from the first page: a large trace fills in page by page
            // (the shape live appends already have) instead of holding the
            // skeleton through the whole multi-second sweep.
            onPage: (accumulated, total) => {
              if (stale()) return
              detailSpansRef.current = accumulated
              setSeedProgress({ loaded: accumulated.size, total })
              // Rebuilding the waterfall on EVERY page saturates the main
              // thread on fat traces (measured live: no frame painted
              // between the first page and the sweep's end). Repaint at
              // most ~1/600ms; the chip above still counts every page, and
              // the final rebuild after the sweep always runs.
              const now = Date.now()
              if (revealed && now - lastPaint < 600) return
              const wf = rebuildDetail(traceId)
              if (wf) lastPaint = now
              if (!silent && !revealed && wf) {
                revealed = true
                setIsLoadingSpans(false)
              }
            },
          },
        )
        if (stale()) return

        detailSpansRef.current = detailSpans
        const wf = rebuildDetail(traceId)
        if (!wf && !silent) {
          setSpansError('no span data available for this trace')
        }
      } catch (err) {
        if (stale() || err === superseded) return
        if (!silent) {
          setSpansError(
            err instanceof Error ? err.message : 'failed to load trace',
          )
        }
      } finally {
        if (!stale()) {
          setSeedProgress(null)
          if (!silent) setIsLoadingSpans(false)
        }
      }
    },
    [rebuildDetail],
  )

  // Silent refresh of the OPEN trace on each activity tick that touches it:
  // no loading state (ticks keep landing while the trace runs), one reload in
  // flight at a time with one trailing rerun so the final tick of a burst is
  // never lost, and a stale response for a since-deselected trace is dropped
  // by `loadTraceSpans` keying on the trace id it was called with.
  useEffect(() => {
    if (!selectedTraceId) return
    let stop: (() => void) | undefined
    let active = true
    let inFlight = false
    let rerun = false
    const refresh = () => {
      if (inFlight) {
        rerun = true
        return
      }
      inFlight = true
      void loadTraceSpans(selectedTraceId, { silent: true }).finally(() => {
        inFlight = false
        if (rerun && active && !isPausedRef.current) {
          rerun = false
          refresh()
        }
      })
    }
    void (async () => {
      const client = await getIiiClient()
      if (!active) return
      stop = startTraceActivityFeed(client, (traceIds) => {
        if (!active || isPausedRef.current) return
        if (!traceIds.includes(selectedTraceId)) return
        refresh()
      })
    })()
    return () => {
      active = false
      stop?.()
    }
  }, [selectedTraceId, loadTraceSpans])

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

  // ── Hold live changes while the user is reading ──
  // A live newest-first list that inserts rows the moment they arrive is
  // unreadable under load: everything below the insertion point moves. So
  // structural changes (rows entering or leaving, order) are HELD while the
  // user is evidently reading — pointer over the rows, scrolled away from
  // the top, on a later page, or a detail expanded — and a pill at the top
  // counts what is waiting. Rows on screen keep updating in place. The hold
  // releases (and the held answer lands) when every reason clears: pointer
  // out, back at the top, back on page 1, detail closed.
  const listHoverRef = useRef(false)
  const [listScrolledAway, setListScrolledAway] = useState(false)
  const listHeldByState =
    listScrolledAway || filterState.page !== 1 || selectedTraceId !== null
  const listHeldByStateRef = useRef(listHeldByState)
  listHeldByStateRef.current = listHeldByState
  const syncListHold = useCallback(() => {
    const held = listHoverRef.current || listHeldByStateRef.current
    holdRef.current = held
    if (!held) flushPendingTraces()
  }, [holdRef, flushPendingTraces])
  useEffect(() => {
    holdRef.current = listHoverRef.current || listHeldByState
    if (!holdRef.current) flushPendingTraces()
  }, [holdRef, listHeldByState, flushPendingTraces])
  const handleListScroll = useCallback((event: React.UIEvent<HTMLElement>) => {
    const away = event.currentTarget.scrollTop > 0
    setListScrolledAway((prev) => (prev === away ? prev : away))
  }, [])
  // What the pill promises: held rows not on screen that will actually
  // show once landed — a hidden-rooted bookkeeping trace that the span
  // filter would drop must not be counted as "new".
  const heldNewCount = useMemo(() => {
    if (heldTraces.length === 0) return 0
    const shown = new Set(traceGroups.map((t) => t.traceId))
    let count = 0
    for (const trace of heldTraces) {
      if (!shown.has(trace.traceId) && showsWithoutProbe(trace, spanFilter)) {
        count++
      }
    }
    return count
  }, [heldTraces, traceGroups, spanFilter])
  // The pill: back to the top of page 1 with everything that arrived.
  const showHeldTraces = useCallback(() => {
    if (filterState.page !== 1) updateFilter('page', 1)
    listScrollRef.current?.scrollTo({ top: 0 })
    flushPendingTraces()
  }, [filterState.page, updateFilter, flushPendingTraces])

  // The follow toggle's engine: opens the newest live turn trace of the
  // active conversation (once per trace — closing it mid-turn is respected).
  // Following lands the held rows first, so the opened trace's detail
  // renders under its own row instead of pinned above a stale list.
  const openFollowedTrace = useCallback(
    (traceId: string | null) => {
      flushPendingTraces()
      selectTrace(traceId)
    },
    [flushPendingTraces, selectTrace],
  )
  useFollowLiveTurn({
    enabled: followTurns && !isPaused,
    activeSessionId: conversationsCtx?.activeId ?? null,
    spans: allSpans,
    selectedTraceId,
    onOpenTrace: openFollowedTrace,
  })

  // Chat linkage of the OPEN trace (session + turn), resolved from the row's
  // merged trace tags with a span-attribute fallback for details opened
  // without their row (timeline-strip click, deep link). The row tags come
  // with the LIST fetch, so the link resolves — and the jump-to-chat works —
  // while the paged detail is still loading; only the fallback path waits
  // for spans. The buttons only render inside the app shell — outside it
  // (Storybook) there is no conversation surface to open.
  const chatLink = useMemo(() => {
    if (selectedTraceId === null) return null
    const tags = traceGroups.find(
      (t) => t.traceId === selectedTraceId,
    )?.traceTags
    return traceChatLink(tags, waterfallData?.spans ?? [])
  }, [waterfallData, traceGroups, selectedTraceId])

  const openConversation = conversationsCtx?.openConversation
  const handleOpenMessage = useMemo(() => {
    if (!openConversation) return undefined
    return (link: TraceChatLink) => {
      // Focus first, open second: the store holds the request before the
      // chat pane mounts, so a freshly placed ChatView reads it on render.
      if (link.turnId) {
        requestChatMessageFocus({
          sessionId: link.sessionId,
          turnId: link.turnId,
        })
      }
      openConversation(link.sessionId)
    }
  }, [openConversation])

  // Deep-link seed: mount with a trace already expanded (used by stories).
  const initialAppliedRef = useRef(false)
  useEffect(() => {
    if (initialAppliedRef.current || !initialTraceId) return
    initialAppliedRef.current = true
    selectTrace(initialTraceId)
  }, [initialTraceId, selectTrace])

  // Esc walks back out: span panel first, then the expanded detail. A
  // pane-scoped command when the host offers one, so it only fires while
  // this pane has the focus; the raw listener stays for older hosts.
  const back = useCallback(() => {
    if (selectedSpan) setSelectedSpan(null)
    else selectTrace(null)
  }, [selectedSpan, selectTrace])
  useEffect(() => {
    if (commands || !selectedTraceId) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      back()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [commands, selectedTraceId, back])
  useEffect(
    () =>
      commands?.register([
        {
          id: 'search',
          title: 'Search traces',
          detail: 'Put the caret in the trace search',
          keywords: ['filter', 'find'],
          shortcut: '/',
          run: () =>
            rootRef.current
              ?.querySelector<HTMLElement>('[placeholder="search traces..."]')
              ?.focus(),
        },
        {
          id: 'follow',
          title: followTurns ? 'Stop following turns' : 'Follow turns',
          detail: 'Jump to each new turn as it arrives',
          keywords: ['live', 'tail', 'auto'],
          shortcut: 'F',
          run: toggleFollowTurns,
        },
        {
          id: 'clear-filters',
          title: 'Clear the trace filters',
          detail: 'Back to every trace',
          keywords: ['reset', 'filters'],
          run: resetFilters,
        },
        {
          id: 'back',
          title: 'Close the trace detail',
          detail: 'Span panel first, then the expanded trace',
          keywords: ['escape', 'close', 'collapse'],
          shortcut: 'Escape',
          enabled: () => selectedTraceId !== null,
          run: back,
        },
      ]),
    [
      commands,
      followTurns,
      toggleFollowTurns,
      resetFilters,
      selectedTraceId,
      back,
    ],
  )

  // The expanded detail, rendered as an accordion item under the selected
  // row (or pinned at the top of the scroll area when that row isn't in the
  // current page — timeline-strip clicks, deep links). The left accent
  // border continues the selected row's marker.
  const traceDetail =
    selectedTraceId !== null ? (
      <div className="bg-panel-raised shadow-raised">
        {isLoadingSpans && (
          <TraceDetailSkeleton
            onClose={() => selectTrace(null)}
            chatLink={chatLink}
            onOpenMessage={handleOpenMessage}
          />
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
                <RefreshCw className="size-4" />
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
              chatLink={chatLink}
              onOpenMessage={handleOpenMessage}
              loadingSpans={seedProgress}
            />
            <div className="border-b border-rule-2 px-3 py-2">
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
            <div className="border-t border-rule-2">
              <WorkerBreakdown data={waterfallData} />
            </div>
          </>
        )}
      </div>
    ) : null

  const selectedInPage =
    selectedTraceId !== null && paged.some((t) => t.traceId === selectedTraceId)

  // The scope chip: names the followed conversation; ✕ shows every session
  // again (until another conversation is selected).
  const sessionScopeChip = sessionScope ? (
    <span
      className="flex items-center gap-1 rounded-xs bg-surface py-0.5 pr-0.5 pl-2 font-mono text-[11px] text-ink-faint"
      title="the list follows the active chat — showing only this session's traces"
    >
      <MessageSquare className="size-4 flex-shrink-0" />
      <span className="max-w-[140px] truncate lowercase">
        {activeConversation?.title ?? sessionScope}
      </span>
      <button
        type="button"
        onClick={() => setScopeDismissedFor(sessionScope)}
        aria-label="show all sessions"
        title="show all sessions"
        className="p-0.5 transition-colors hover:text-ink"
      >
        <X className="size-4" />
      </button>
    </span>
  ) : null

  return (
    <section
      ref={rootRef}
      aria-label="traces"
      className="flex-1 flex flex-col overflow-hidden"
    >
      <PageHeader
        icon={<GitBranch />}
        title="Traces"
        onClose={onRequestClose}
      />
      <TimelineStrip
        spans={allSpans}
        spanFilter={spanFilter}
        isPaused={isPaused}
        onTraceClick={toggleTrace}
        followTurns={followTurns}
        onToggleFollowTurns={conversationsCtx ? toggleFollowTurns : undefined}
      />

      <div className="px-3 py-2 border-b border-rule-2">
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
              <>
                {viewsAvailable ? (
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
                ) : null}
                {sessionScopeChip}
              </>
            }
          />
        </ErrorBoundary>
      </div>

      <ErrorBoundary>
        {/* Only the engine's definitive answer earns this message — while
            the seed is still unanswered (null) the list area renders its
            loading skeleton instead. */}
        {hasOtelConfigured === false ? (
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
                {isQueryLoading && visibleTraceGroups.length === 0 ? (
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
                        className="px-3 py-3 border-b border-rule-2"
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
                ) : visibleTraceGroups.length === 0 ? (
                  <div className="p-9">
                    <EmptyState
                      icon={GitBranch}
                      title={
                        sessionScope
                          ? 'no stored traces for this session'
                          : 'no traces recorded'
                      }
                      description={
                        sessionScope
                          ? // The client cannot tell "never ran" from "already
                            // expired", so the copy owns both honestly.
                            'the list follows the active chat, and nothing is stored for this conversation — no work recorded yet, or its traces already expired. new activity appears here as it runs, or clear the session chip above to see every trace.'
                          : 'traces appear here when functions execute. fire a request to your engine and refresh.'
                      }
                    />
                  </div>
                ) : (
                  <div className="flex-1 flex flex-col overflow-hidden">
                    {/* The hover hold wraps the pill too: moving the pointer
                        onto the pill must not release the hold and pull the
                        pill away from under the click. */}
                    {/* biome-ignore lint/a11y/noStaticElementInteractions: hover detection for pause/resume of live updates */}
                    <div
                      className="relative flex-1 min-h-0 flex flex-col"
                      onMouseEnter={() => {
                        listHoverRef.current = true
                        syncListHold()
                      }}
                      onMouseLeave={() => {
                        listHoverRef.current = false
                        syncListHold()
                      }}
                    >
                      {heldNewCount > 0 ? (
                        <Button
                          type="button"
                          variant="pill"
                          size="sm"
                          className="iii-ui-motion-overlay absolute top-3 left-1/2 z-10 h-8 -translate-x-1/2 rounded-full bg-panel-raised/80 pr-3 pl-2.5 font-medium text-[0.8125rem] shadow-raised backdrop-blur-md"
                          onClick={showHeldTraces}
                          aria-label={
                            filterState.page === 1
                              ? `show ${heldNewCount} new ${heldNewCount === 1 ? 'trace' : 'traces'}`
                              : 'show the latest traces on page 1'
                          }
                          data-trace-list-held={heldNewCount}
                        >
                          <ArrowUp
                            aria-hidden="true"
                            className="stroke-current"
                          />
                          <span>
                            {filterState.page === 1
                              ? `${heldNewCount} new ${heldNewCount === 1 ? 'trace' : 'traces'}`
                              : 'latest traces'}
                          </span>
                        </Button>
                      ) : null}
                      <div
                        className="flex-1 overflow-y-auto"
                        ref={listScrollRef}
                        onScroll={handleListScroll}
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
                    </div>
                    <div className="flex-shrink-0 border-t border-rule-2 px-3 py-2">
                      <Pagination
                        currentPage={filterState.page}
                        totalPages={totalPages}
                        totalItems={totalTraceCount}
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
                  className="w-[3px] flex-shrink-0 cursor-col-resize bg-transparent hover:bg-accent/60 active:bg-accent"
                />
                <div
                  style={{ width: spanPanel.width }}
                  className={cn(
                    'bg-panel-raised border-l border-rule-2 flex-shrink-0 h-full overflow-hidden',
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
