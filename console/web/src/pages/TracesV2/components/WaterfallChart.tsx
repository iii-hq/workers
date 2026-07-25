/**
 * Waterfall view for a single trace's spans.
 *
 * Data flow:
 *
 *   props.data (WaterfallData)
 *        │
 *        ▼
 *   applyHiddenSpanGroups(data)     ← span-group filter (funnel menu), shared
 *        │                            with the timeline views
 *        ▼
 *   buildSpanTree(spans)            ← parent/child linking, marks critical path
 *        │
 *        ▼
 *   useReducer<DisplayState>        ← expand/collapse
 *        │
 *        ▼
 *   flattenTree(tree, opts)         ← respects expandedIds; engine-routing
 *        │                            spans are always hidden, with depth-offset
 *        │                            adjusted rows so hidden parents don't
 *        │                            leave gaps
 *        ▼
 *   visibleSpans: FlatSpanRow[]
 *        │
 *        ▼
 *   Per-row render: indent guides, kind indicator, status dot, name,
 *                   duration, bar (status-colored), merged-routing `+1`
 *                   chip when applicable
 *
 * Hovering a row raises the shared SpanHoverCard (same card as the
 * timeline views) with the worker name, duration, start offset, status
 * and %-of-trace — the row itself stays lean (no worker tag).
 *
 * Filtering: the toolbar hosts the same funnel menu as the timeline view
 * (`SpanFilterMenu` over `deriveSpanGroups` / `applyHiddenSpanFilters`),
 * with workers, span-groups, and internal sections — hiding an entry
 * removes ONLY its spans (children re-attach to the hidden span's parent)
 * without rescaling the time window.
 * What a span group IS comes from the caller via `spanGroupKey` (the page
 * groups by owning function id); the SELECTION comes from the caller via
 * `spanFilter`, shared with the timeline and persisted in the console
 * configuration. Engine-routing spans (`handle_invocation` / `call`
 * pairs) are always hidden; the old reveal + critical-path-only toggles
 * are gone.
 *
 * Persistent state (localStorage):
 *   iii-span-col-width                  resizer position
 *
 * Coloring: OK bars carry the worker's chromatic identity color
 * (`getWorkerColor`, shared with the timelines and the worker breakdown);
 * error bars collapse onto `--color-alert`, unset/pending onto ghost.
 *
 * Engine-routing heuristics live in `../lib/spanLabel`.
 */

import { useVirtualizer } from '@tanstack/react-virtual'
import { ChevronRight, ChevronsDown, ChevronsUp } from 'lucide-react'
import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import type { SpanFilterControls } from '../lib/spanFilters'
import {
  formatSpanLabel,
  getSpanKindIndicator,
  internalFamilyOf,
} from '../lib/spanLabel'
import { buildSpanTree, type FlatSpanRow, flattenTree } from '../lib/spanTree'
import { getWorkerColor } from '../lib/traceColors'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import { formatDuration, getWorkerName } from '../lib/traceUtils'
import type { TimelineSpan } from './timeline/layout'
import { SpanFilterMenu } from './timeline/SpanFilterMenu'
import { SpanHoverCard } from './timeline/SpanHoverCard'
import {
  applyHiddenSpanFilters,
  deriveSpanGroups,
  type SpanGroup,
  type SpanGroupKey,
  workerGroupKey,
} from './timeline/spanVisibility'

interface WaterfallChartProps {
  data: WaterfallData
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string | null
  /**
   * Grouping key for the toolbar's span filter (same contract as the
   * timeline views — see `lib/traceTimelineFilters.ts`).
   */
  spanGroupKey?: SpanGroupKey
  /**
   * Selection + mutations behind the toolbar's funnel menu. Owned by the
   * caller so the same selection is shared with the timeline view and
   * persisted (see `hooks/useSpanFilterSelection.ts`). The menu renders
   * only when BOTH this and `spanGroupKey` are provided.
   */
  spanFilter?: SpanFilterControls
}

interface WaterfallRowProps {
  span: FlatSpanRow
  isSelected: boolean
  isExpanded: boolean
  onSpanClick: (span: VisualizationSpan) => void
  onToggleExpand: (spanId: string) => void
  onHoverMove: (span: VisualizationSpan, e: React.MouseEvent) => void
  onHoverEnd: (spanId: string) => void
}

const WaterfallRow = memo(function WaterfallRow({
  span,
  isSelected,
  isExpanded,
  onSpanClick,
  onToggleExpand,
  onHoverMove,
  onHoverEnd,
}: WaterfallRowProps) {
  const hasChildren = span.children.length > 0
  const kindIndicator = getSpanKindIndicator(span.kind)
  const displayLabel = formatSpanLabel(span)
  const isError = span.status === 'error'

  // Color mapping: OK bars carry the worker's chromatic identity color
  // (same hash the timelines and worker breakdown use); errors collapse
  // onto alert, unset/pending stays ghost. The critical-path toggle
  // filters the visible rows rather than re-coloring them.
  const barColor = isError
    ? 'var(--color-alert)'
    : span.status === 'ok'
      ? getWorkerColor(getWorkerName(span))
      : 'var(--color-ink-ghost)'

  // Hover is pure CSS (`hover:bg-surface-hover`). Selected/error chrome
  // takes priority over hover via CSS specificity (more-specific
  // bg classes).
  const rowChrome = isSelected
    ? 'bg-surface-selected'
    : isError
      ? 'bg-alert-muted'
      : 'hover:bg-surface-hover'

  return (
    // biome-ignore lint/a11y/useSemanticElements: this clickable row contains a nested expand/collapse <button>, so it can't be a native <button> (nested interactive content is invalid HTML); div + role="button" + key handlers is the accessible fallback
    <div
      role="button"
      tabIndex={0}
      className={cn(
        'grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-1 items-center cursor-pointer w-full text-left',
        rowChrome,
      )}
      onClick={() => onSpanClick(span)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSpanClick(span)
        }
      }}
      onMouseEnter={(e) => onHoverMove(span, e)}
      onMouseMove={(e) => onHoverMove(span, e)}
      onMouseLeave={() => onHoverEnd(span.span_id)}
    >
      <div className="flex items-center gap-1.5 min-w-0">
        <div
          className="flex-shrink-0 flex"
          style={{ width: span.displayDepth * 16 }}
        >
          {indentKeys(span.span_id, span.displayDepth).map((key) => (
            <div key={key} className="w-4 h-6 border-l border-rule-2" />
          ))}
        </div>

        {hasChildren ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onToggleExpand(span.span_id)
            }}
            className="w-4 h-4 flex items-center justify-center text-ink-faint hover:text-ink flex-shrink-0"
            aria-label={isExpanded ? 'collapse span' : 'expand span'}
          >
            <ChevronRight
              className={cn(
                'w-3 h-3 transition-transform',
                isExpanded && 'rotate-90',
              )}
            />
          </button>
        ) : (
          <div className="w-4 h-4 flex-shrink-0" />
        )}

        <StatusDot tone={statusDotTone(span.status)} />

        {kindIndicator && (
          <span
            className="flex-shrink-0 text-[11px] text-ink-faint leading-none w-3 text-center"
            title={kindIndicator.label}
          >
            {kindIndicator.icon}
          </span>
        )}

        <span
          className={cn(
            'text-[13px] font-mono truncate lowercase',
            isSelected ? 'text-accent' : 'text-ink',
          )}
        >
          {displayLabel}
        </span>

        <span className="font-mono text-[11px] text-ink-faint flex-shrink-0 ml-auto tabular-nums">
          {formatDuration(span.duration_ms)}
          {span.pending && '…'}
        </span>
      </div>

      {/* bar track */}
      <div className="relative h-6 bg-rule-2">
        <div
          className={cn(
            'absolute h-4 top-1 min-w-[3px]',
            isSelected && 'outline outline-2 outline-accent',
            // Still running: elapsed-so-far bar, pulsing until the final
            // span replaces the live snapshot.
            span.pending && 'animate-pulse opacity-70',
          )}
          style={{
            left: `${span.start_percent}%`,
            width: `${Math.max(0.5, span.width_percent)}%`,
            backgroundColor: barColor,
          }}
        />
      </div>
    </div>
  )
})

interface DisplayState {
  expandedIds: Set<string>
}

type DisplayAction =
  | { type: 'TOGGLE_SPAN'; spanId: string }
  | { type: 'SET_ALL_EXPANDED'; ids: Set<string> }

const initialDisplayState: DisplayState = {
  expandedIds: new Set(),
}

// Note: `hoveredSpanId` used to live in this reducer, but mouse-sweep
// over the chart fired one dispatch per row entered/exited — each one a
// full re-render of every visible row. For a 2000+-span trace that froze
// the page on mouse-move. Row-background styling stays pure CSS
// (`hover:bg-surface-hover`); the hover DETAIL card (SpanHoverCard, shared with
// the timelines) is plain `useState` on the chart component instead:
// rows are memo'd with stable callbacks, so a per-mousemove state update
// re-renders only the shallow wrapper map and the card — never the rows.

interface HoverState {
  id: string
  /** viewport coords of the cursor, fed to the fixed-position card */
  x: number
  y: number
}

function displayReducer(
  state: DisplayState,
  action: DisplayAction,
): DisplayState {
  switch (action.type) {
    case 'TOGGLE_SPAN': {
      const next = new Set(state.expandedIds)
      if (next.has(action.spanId)) {
        next.delete(action.spanId)
      } else {
        next.add(action.spanId)
      }
      return { ...state, expandedIds: next }
    }
    case 'SET_ALL_EXPANDED':
      return { ...state, expandedIds: action.ids }
  }
}

const SPAN_COL_WIDTH_KEY = 'iii-span-col-width'
const DEFAULT_SPAN_COL_WIDTH = 300
const MIN_SPAN_COL_WIDTH = 150
const MAX_SPAN_COL_WIDTH = 600
const MINIMAP_HEIGHT = 80
const ROW_HEIGHT = 32

function statusDotTone(
  status: VisualizationSpan['status'],
): 'accent' | 'alert' | 'warn' {
  switch (status) {
    case 'ok':
      return 'accent'
    case 'error':
      return 'alert'
    default:
      return 'warn'
  }
}

/**
 * Build stable React keys for decorative indent rails. Each rail is
 * `${span_id}-indent-${i}` — these are identity-stable per-span and only
 * change when the span's `displayDepth` does (re-rendering the whole row).
 */
function indentKeys(spanId: string, depth: number): string[] {
  const keys: string[] = []
  for (let i = 0; i < depth; i++) {
    keys.push(`${spanId}-indent-${i}`)
  }
  return keys
}

// Compact icon-driven toolbar: expand/collapse-all as plain icon
// buttons, then the span-group funnel menu (shared with the timeline
// views) behind a divider. The funnel hides itself when the trace has
// no groups to offer and nothing is filtered.
interface ToolbarProps {
  expandAll: () => void
  collapseAll: () => void
  spanGroups: readonly SpanGroup[]
  workerGroups: readonly SpanGroup[]
  internalGroups: readonly SpanGroup[]
  spanFilter?: SpanFilterControls
  hiddenSpanCount: number
  visibleCount: number
  totalCount: number
}

function Toolbar(props: ToolbarProps) {
  const {
    expandAll,
    collapseAll,
    spanGroups,
    workerGroups,
    internalGroups,
    spanFilter,
    hiddenSpanCount,
    visibleCount,
    totalCount,
  } = props
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-rule-2">
      <div className="flex items-center gap-1.5">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={expandAll}
              aria-label="expand all spans"
              className="inline-flex items-center justify-center w-7 h-7 rounded-sm text-ink-faint hover:text-ink hover:bg-surface-hover transition-colors"
            >
              <ChevronsDown className="w-3.5 h-3.5" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">expand all</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={collapseAll}
              aria-label="collapse all spans"
              className="inline-flex items-center justify-center w-7 h-7 rounded-sm text-ink-faint hover:text-ink hover:bg-surface-hover transition-colors"
            >
              <ChevronsUp className="w-3.5 h-3.5" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">collapse all</TooltipContent>
        </Tooltip>
        {spanFilter &&
          (spanGroups.length > 0 ||
            workerGroups.length > 0 ||
            internalGroups.length > 0 ||
            hiddenSpanCount > 0) && (
            <>
              <div aria-hidden className="w-px h-4 bg-rule-2 mx-1" />
              <SpanFilterMenu
                groups={spanGroups}
                workerGroups={workerGroups}
                internalGroups={internalGroups}
                hiddenKeys={spanFilter.hiddenGroups}
                hiddenWorkerKeys={spanFilter.hiddenWorkers}
                shownInternalKeys={spanFilter.shownInternal}
                hiddenSpanCount={hiddenSpanCount}
                onToggle={spanFilter.toggleGroup}
                onToggleWorker={spanFilter.toggleWorker}
                onToggleInternal={spanFilter.toggleInternal}
                onClear={() =>
                  spanFilter.clear(internalGroups.map((g) => g.key))
                }
                className="h-7"
              />
            </>
          )}
      </div>
      <div className="text-[11px] text-ink-faint tabular-nums lowercase">
        {visibleCount} of {totalCount} spans
      </div>
    </div>
  )
}

export function WaterfallChart({
  data,
  onSpanClick,
  selectedSpanId,
  spanGroupKey,
  spanFilter,
}: WaterfallChartProps) {
  const [displayState, dispatch] = useReducer(
    displayReducer,
    initialDisplayState,
  )
  const { expandedIds } = displayState
  const containerRef = useRef<HTMLDivElement | null>(null)
  const thumbRef = useRef<HTMLDivElement | null>(null)

  // containerHeight is kept locally only to decide whether to show
  // the minimap. The library owns the heavy lifting of scroll
  // position + visible-range tracking.
  const [containerHeight, setContainerHeight] = useState(0)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    setContainerHeight(el.clientHeight)
    const obs = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerHeight(entry.contentRect.height)
      }
    })
    obs.observe(el)
    return () => obs.disconnect()
  }, [])

  // The hidden-group/worker selection behind the toolbar's funnel menu
  // lives with the CALLER (`spanFilter`) so the waterfall and timeline
  // share one selection and it persists in the console configuration.
  // This component only derives menu entries and applies the selection.
  const filterEnabled = !!spanGroupKey && !!spanFilter

  // Menu entries against the FULL data (an already-hidden group must
  // keep its row so it can be turned back on), busiest first. The group
  // key gets the whole trace by id so parent-dependent grouping (tag
  // roots) agrees with what `applyHiddenSpanFilters` hides.
  const spansById = useMemo(
    () => new Map(data.spans.map((s) => [s.span_id, s])),
    [data.spans],
  )
  const spanGroups = useMemo(
    () =>
      filterEnabled && spanGroupKey
        ? deriveSpanGroups(data.spans, (s) =>
            internalFamilyOf(s.attributes) ? null : spanGroupKey(s, spansById),
          )
        : [],
    [data.spans, spanGroupKey, spansById, filterEnabled],
  )
  const workerGroups = useMemo(
    () =>
      filterEnabled
        ? deriveSpanGroups(data.spans, (s) =>
            internalFamilyOf(s.attributes) ? null : workerGroupKey(s),
          )
        : [],
    [data.spans, filterEnabled],
  )
  // Call-site-tagged plumbing (`iii.tag.hidden`): its own menu section,
  // hidden by default.
  const internalGroups = useMemo(
    () =>
      filterEnabled
        ? deriveSpanGroups(data.spans, (s) => internalFamilyOf(s.attributes))
        : [],
    [data.spans, filterEnabled],
  )

  const visibleData = useMemo(
    () =>
      filterEnabled
        ? applyHiddenSpanFilters(data, spanGroupKey, spanFilter)
        : data,
    [data, spanGroupKey, spanFilter, filterEnabled],
  )

  // span column resize
  const [spanColWidth, setSpanColWidth] = useState<number>(() => {
    if (typeof window === 'undefined') return DEFAULT_SPAN_COL_WIDTH
    const saved = window.localStorage.getItem(SPAN_COL_WIDTH_KEY)
    return saved ? Number.parseInt(saved, 10) : DEFAULT_SPAN_COL_WIDTH
  })
  const colResizeRef = useRef<{ startX: number; startWidth: number } | null>(
    null,
  )
  const spanColWidthRef = useRef(spanColWidth)
  spanColWidthRef.current = spanColWidth

  useEffect(() => {
    window.localStorage.setItem(SPAN_COL_WIDTH_KEY, String(spanColWidth))
  }, [spanColWidth])

  useEffect(() => {
    let rafId: number | null = null
    const onMouseMove = (e: MouseEvent) => {
      if (!colResizeRef.current) return
      if (rafId !== null) return
      rafId = requestAnimationFrame(() => {
        rafId = null
        if (!colResizeRef.current) return
        const diff = e.clientX - colResizeRef.current.startX
        setSpanColWidth(
          Math.min(
            Math.max(
              colResizeRef.current.startWidth + diff,
              MIN_SPAN_COL_WIDTH,
            ),
            MAX_SPAN_COL_WIDTH,
          ),
        )
      })
    }
    const onMouseUp = () => {
      if (!colResizeRef.current) return
      colResizeRef.current = null
      if (rafId !== null) {
        cancelAnimationFrame(rafId)
        rafId = null
      }
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    window.addEventListener('mousemove', onMouseMove)
    window.addEventListener('mouseup', onMouseUp)
    return () => {
      window.removeEventListener('mousemove', onMouseMove)
      window.removeEventListener('mouseup', onMouseUp)
      if (rafId !== null) cancelAnimationFrame(rafId)
    }
  }, [])

  const startColResize = (e: React.MouseEvent) => {
    e.preventDefault()
    colResizeRef.current = {
      startX: e.clientX,
      startWidth: spanColWidthRef.current,
    }
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
  }

  const totalMs = data.total_duration_ms || 1
  const rulerMarks = useMemo(
    () =>
      [0, 25, 50, 75, 100].map((pct) => ({
        pct,
        label: formatDuration((totalMs * pct) / 100),
      })),
    [totalMs],
  )

  const spanTree = useMemo(
    () => buildSpanTree(visibleData.spans),
    [visibleData.spans],
  )

  // Keyed on the RAW spans so only a trace switch re-expands everything —
  // toggling a filter group must not reset the user's collapse state.
  // (Expanded ids for filtered-out spans are harmless leftovers.)
  useEffect(() => {
    const allIds = new Set(data.spans.map((s) => s.span_id))
    dispatch({ type: 'SET_ALL_EXPANDED', ids: allIds })
  }, [data.spans])

  const visibleSpans = useMemo(
    () =>
      flattenTree(spanTree, {
        expandedIds,
        hideEngineRouting: true,
        collapseEngineRoutingPairs: false,
      }),
    [spanTree, expandedIds],
  )

  // @tanstack/react-virtual owns scroll state, container measurement,
  // and per-row positioning. Per-row absolute positioning via
  // `transform: translateY(...)` lets the browser composite each scroll
  // tick without re-rendering the whole list. See spec
  // `2026-05-19-waterfall-virtualization-lib-swap-design.md`.
  const virtualizer = useVirtualizer({
    count: visibleSpans.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 16,
  })
  const virtualRows = virtualizer.getVirtualItems()
  const contentHeight = virtualizer.getTotalSize()

  // The minimap thumb tracks the scroll container directly so it
  // stays glued to the user's scrollTop without forcing a React
  // re-render of the whole chart on every wheel tick. The
  // virtualizer already exposes contentHeight via getTotalSize(),
  // so we read it on each rAF frame from the same source.
  // biome-ignore lint/correctness/useExhaustiveDependencies: contentHeight/containerHeight are intentional re-measure triggers (re-run update() when rows expand or the viewport resizes), not values read directly in the effect body
  useEffect(() => {
    const el = containerRef.current
    const thumb = thumbRef.current
    if (!el || !thumb) return
    let raf: number | null = null
    const update = () => {
      raf = null
      const scrollHeight = el.scrollHeight
      if (scrollHeight <= 0) return
      const top = (el.scrollTop / scrollHeight) * MINIMAP_HEIGHT
      const height = Math.max(
        20,
        MINIMAP_HEIGHT * (el.clientHeight / scrollHeight),
      )
      thumb.style.top = `${top}px`
      thumb.style.height = `${height}px`
    }
    const onScroll = () => {
      if (raf !== null) return
      raf = requestAnimationFrame(update)
    }
    update()
    el.addEventListener('scroll', onScroll, { passive: true })
    const obs = new ResizeObserver(update)
    obs.observe(el)
    return () => {
      el.removeEventListener('scroll', onScroll)
      obs.disconnect()
      if (raf !== null) cancelAnimationFrame(raf)
    }
  }, [contentHeight, containerHeight])

  const toggleExpand = useCallback((spanId: string) => {
    dispatch({ type: 'TOGGLE_SPAN', spanId })
  }, [])

  // Hover detail card. Stable callbacks keep the memo'd rows from
  // re-rendering on mouse-move (see the note above DisplayState).
  const [hover, setHover] = useState<HoverState | null>(null)
  const handleHoverMove = useCallback(
    (span: VisualizationSpan, e: React.MouseEvent) => {
      // Suppress the card while the span column is being resized — the
      // global drag listeners sweep the cursor across rows.
      if (colResizeRef.current) return
      setHover({ id: span.span_id, x: e.clientX, y: e.clientY })
    },
    [],
  )
  const handleHoverEnd = useCallback((spanId: string) => {
    setHover((prev) => (prev?.id === spanId ? null : prev))
  }, [])

  // Resolved by id against the visible rows so a span that leaves the
  // list (collapse, toggle, live replacement) drops its card instead of
  // going stale. Same offset/label/meta recipe as the timeline bars
  // (`waterfallToTimelineSpans`), so the card reads identically in both
  // views: worker name as the subtitle, trace-relative start, % of trace.
  const hoveredCard = useMemo(() => {
    if (!hover) return null
    const span = visibleSpans.find((s) => s.span_id === hover.id)
    if (!span) return null
    const start = (span.start_percent / 100) * totalMs
    const cardSpan: TimelineSpan = {
      id: span.span_id,
      startTime: start,
      endTime: start + span.duration_ms,
      status: span.pending && span.status !== 'error' ? 'pending' : span.status,
      label: formatSpanLabel(span),
      meta: getWorkerName(span),
    }
    return { cardSpan, tracePercent: Math.min(100, span.width_percent) }
  }, [hover, visibleSpans, totalMs])

  const expandAll = () => {
    const allIds = new Set(data.spans.map((s) => s.span_id))
    dispatch({ type: 'SET_ALL_EXPANDED', ids: allIds })
  }

  const collapseAll = () => {
    dispatch({ type: 'SET_ALL_EXPANDED', ids: new Set() })
  }

  const toolbarProps: ToolbarProps = {
    expandAll,
    collapseAll,
    spanGroups,
    workerGroups,
    internalGroups,
    spanFilter,
    hiddenSpanCount: data.spans.length - visibleData.spans.length,
    visibleCount: visibleSpans.length,
    totalCount: data.span_count,
  }

  return (
    <div className="flex flex-col h-full">
      <Toolbar {...toolbarProps} />

      {/* sticky time axis */}
      <div
        style={
          { '--span-col-width': `${spanColWidth}px` } as React.CSSProperties
        }
        className="grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-2 text-[11px] font-semibold text-ink-ghost uppercase tracking-[0.06em] border-b border-rule-2 bg-panel-raised"
      >
        <div className="flex items-center relative">
          <span>span</span>
          {/* biome-ignore lint/a11y/useSemanticElements: <hr> can't host pointer handlers for a draggable resize affordance */}
          <div
            // biome-ignore lint/a11y/useAriaPropsForRole: separator is the WAI-ARIA pattern for a resizable column divider with no keyboard interaction requirement
            role="separator"
            aria-label="resize span column"
            aria-orientation="vertical"
            tabIndex={0}
            onMouseDown={startColResize}
            onDoubleClick={() => setSpanColWidth(DEFAULT_SPAN_COL_WIDTH)}
            className="absolute right-[-11px] top-0 bottom-0 w-[7px] cursor-col-resize z-10 group"
            title="drag to resize, double-click to reset"
          >
            <div className="absolute left-[3px] top-0 bottom-0 w-[1px] bg-rule-2 group-hover:bg-accent transition-colors" />
          </div>
        </div>
        <div className="flex justify-between font-mono">
          {rulerMarks.map(({ pct, label }) => (
            <span key={pct} className="tabular-nums">
              {label}
            </span>
          ))}
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">
        {/* biome-ignore lint/a11y/noStaticElementInteractions: mouse-leave only dismisses the pointer-driven hover card (a backstop for rows that unmount under the cursor mid-scroll) — there is no interaction to expose to keyboard/AT users */}
        <div
          ref={containerRef}
          style={
            { '--span-col-width': `${spanColWidth}px` } as React.CSSProperties
          }
          className="flex-1 overflow-y-auto"
          onMouseLeave={() => setHover(null)}
        >
          {/* Height spacer keeps the native scrollbar accurate; each
              virtualized row is absolutely positioned via translateY so
              the browser can composite scroll without re-rendering the
              list. The virtualizer drives `virtualRows`. */}
          <div style={{ height: contentHeight, position: 'relative' }}>
            {virtualRows.map((vrow) => {
              const span = visibleSpans[vrow.index]
              if (!span) return null
              return (
                <div
                  key={vrow.key}
                  data-index={vrow.index}
                  style={{
                    position: 'absolute',
                    top: 0,
                    left: 0,
                    width: '100%',
                    height: ROW_HEIGHT,
                    transform: `translateY(${vrow.start}px)`,
                  }}
                >
                  <WaterfallRow
                    span={span}
                    isSelected={selectedSpanId === span.span_id}
                    isExpanded={expandedIds.has(span.span_id)}
                    onSpanClick={onSpanClick}
                    onToggleExpand={toggleExpand}
                    onHoverMove={handleHoverMove}
                    onHoverEnd={handleHoverEnd}
                  />
                </div>
              )
            })}
          </div>
        </div>

        {/* minimap */}
        {contentHeight > containerHeight && (
          <div className="w-16 border-l border-rule-2 flex-shrink-0 relative p-2">
            <div className="text-[9px] text-ink-ghost uppercase tracking-[0.06em] mb-2">
              map
            </div>
            <div
              className="relative bg-rule-2 overflow-hidden"
              style={{ height: MINIMAP_HEIGHT }}
            >
              {visibleData.spans.map((span, i) => {
                const isError = span.status === 'error'
                return (
                  <div
                    key={span.span_id}
                    className={cn(
                      'absolute h-[2px]',
                      isError ? 'bg-alert' : 'bg-ink-ghost',
                    )}
                    style={{
                      opacity: isError ? 0.7 : 0.5,
                      top: `${(i / visibleData.spans.length) * 100}%`,
                      left: `${span.start_percent}%`,
                      width: `${Math.max(2, span.width_percent)}%`,
                    }}
                  />
                )
              })}
              <div
                ref={thumbRef}
                className="absolute left-0 right-0 bg-accent/20 border border-accent"
                style={{ top: 0, height: 20 }}
              />
            </div>
          </div>
        )}
      </div>

      {hover && hoveredCard && (
        <SpanHoverCard
          span={hoveredCard.cardSpan}
          now={totalMs}
          x={hover.x}
          y={hover.y}
          relativeStart
          tracePercent={hoveredCard.tracePercent}
        />
      )}
    </div>
  )
}
