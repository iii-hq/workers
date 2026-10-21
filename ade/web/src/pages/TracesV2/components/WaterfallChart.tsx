/**
 * The waterfall: one row per span, a tree on the left and a bar on the
 * right against a shared time axis — an observability card's trace view.
 *
 * Data flow is unchanged from the previous chart:
 *
 *   props.data → applyHiddenSpanFilters (the funnel menu's selection,
 *   shared with the timeline views) → buildSpanTree → flattenTree
 *   (expand/collapse, engine-routing spans always folded away) →
 *   FlatSpanRow[] → virtualised rows (@tanstack/react-virtual).
 *
 * What a row says: indent rails, a disclosure for parents, a glyph tile in
 * the worker's colour, the span's name and its worker, then the bar. A
 * parent's bar is a hollow track spanning its children; a leaf's bar is a
 * filled gradient in the worker's colour (errors red, a running span
 * striped and still growing). The duration sits just after the bar, the
 * slowest leaf wears a tag, and a live trace carries a playhead at the
 * leading edge of the axis.
 *
 * Motion: bars grow in from the left and rows fade up as they mount, each
 * a few milliseconds after the one above (capped, so a long trace is not a
 * slow one). Reduced motion paints everything at rest.
 */

import { useVirtualizer } from '@tanstack/react-virtual'
import {
  ArrowDownLeft,
  ArrowUpRight,
  Box,
  ChevronRight,
  ChevronsDown,
  ChevronsUp,
  Inbox,
  Loader2,
  Send,
} from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/shadcn/tooltip'
import { cn } from '@/lib/utils'
import type { SpanFilterControls } from '../lib/spanFilters'
import { formatSpanLabel, internalFamilyOf } from '../lib/spanLabel'
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
  /**
   * Show expand-all / collapse-all. Pass `false` when the trace is still
   * arriving: every change to `data.spans` re-expands the tree, so a
   * collapse taken mid-stream is undone by the next span to land.
   */
  showExpandControls?: boolean
}

const SPAN_COL_WIDTH_KEY = 'iii-span-col-width'
const DEFAULT_SPAN_COL_WIDTH = 300
const MIN_SPAN_COL_WIDTH = 180
const MAX_SPAN_COL_WIDTH = 600
const ROW_HEIGHT = 36
const INDENT = 18
/** Rows stagger their entrance by this much, up to a cap. */
const STAGGER_MS = 18
const STAGGER_CAP = 24
const GRID_TICKS = [0, 25, 50, 75, 100] as const

/** The glyph a span wears, from its OpenTelemetry kind. */
function kindGlyph(kind: string | undefined) {
  switch (kind?.toLowerCase()) {
    case 'server':
      return { Icon: ArrowDownLeft, label: 'server (handles incoming)' }
    case 'client':
      return { Icon: ArrowUpRight, label: 'client (outgoing call)' }
    case 'producer':
      return { Icon: Send, label: 'producer (sends to queue)' }
    case 'consumer':
      return { Icon: Inbox, label: 'consumer (reads from queue)' }
    default:
      return { Icon: Box, label: 'internal' }
  }
}

interface WaterfallRowProps {
  span: FlatSpanRow
  index: number
  isSelected: boolean
  isExpanded: boolean
  isSlowest: boolean
  reduceMotion: boolean
  onSpanClick: (span: VisualizationSpan) => void
  onToggleExpand: (spanId: string) => void
  onHoverMove: (span: VisualizationSpan, e: React.MouseEvent) => void
  onHoverEnd: (spanId: string) => void
}

const WaterfallRow = memo(function WaterfallRow({
  span,
  index,
  isSelected,
  isExpanded,
  isSlowest,
  reduceMotion,
  onSpanClick,
  onToggleExpand,
  onHoverMove,
  onHoverEnd,
}: WaterfallRowProps) {
  const hasChildren = span.children.length > 0
  const { Icon, label: kindLabel } = kindGlyph(span.kind)
  const name = formatSpanLabel(span)
  const worker = getWorkerName(span)
  const color = getWorkerColor(worker)
  const isError = span.status === 'error'
  const delay = reduceMotion ? 0 : Math.min(index, STAGGER_CAP) * STAGGER_MS
  const left = Math.min(100, Math.max(0, span.start_percent))
  const width = Math.max(0.4, Math.min(100 - left, span.width_percent))

  return (
    // biome-ignore lint/a11y/useSemanticElements: the row hosts a nested disclosure <button>; a native <button> cannot contain it
    <div
      role="button"
      tabIndex={0}
      aria-pressed={isSelected}
      className={cn(
        'group/row grid h-full w-full cursor-pointer grid-cols-[var(--span-col-width)_1fr] items-center text-left transition-colors duration-[var(--motion-duration-control)]',
        isSelected
          ? 'bg-neutral-200/60 dark:bg-neutral-800/70'
          : 'hover:bg-neutral-200/40 dark:hover:bg-neutral-800/40',
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
      {/* ── the tree column ─────────────────────────────────────────── */}
      <motion.div
        initial={reduceMotion ? false : { opacity: 0, x: -4 }}
        animate={{ opacity: 1, x: 0 }}
        transition={{
          duration: 0.22,
          delay: delay / 1000,
          ease: [0.2, 0, 0, 1],
        }}
        className="flex h-full min-w-0 items-center gap-2 pr-3 pl-3"
        style={{ paddingLeft: 12 + span.displayDepth * INDENT }}
      >
        {/* Indent rails: one hairline per ancestor level. */}
        {span.displayDepth > 0 ? (
          <span
            aria-hidden
            className="pointer-events-none absolute inset-y-0 left-0 flex"
            style={{ paddingLeft: 12 + 9 }}
          >
            {Array.from({ length: span.displayDepth }, (_, level) => (
              <span
                // biome-ignore lint/suspicious/noArrayIndexKey: rails are positional
                key={level}
                className="h-full border-l border-neutral-200 dark:border-neutral-800"
                style={{ width: INDENT }}
              />
            ))}
          </span>
        ) : null}

        {hasChildren ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onToggleExpand(span.span_id)
            }}
            aria-label={isExpanded ? 'collapse span' : 'expand span'}
            aria-expanded={isExpanded}
            className="relative z-10 flex size-5 shrink-0 items-center justify-center rounded-md text-neutral-400 hover:bg-neutral-300/60 hover:text-neutral-900 dark:text-neutral-500 dark:hover:bg-neutral-700 dark:hover:text-neutral-50"
          >
            <ChevronRight
              className={cn(
                'size-4 transition-transform duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)]',
                isExpanded && 'rotate-90',
              )}
              aria-hidden
            />
          </button>
        ) : (
          <span className="size-5 shrink-0" aria-hidden />
        )}

        {/* The glyph tile, tinted in the worker's colour. */}
        <span
          aria-hidden
          title={kindLabel}
          className="relative z-10 flex size-6 shrink-0 items-center justify-center rounded-md"
          style={{
            backgroundColor: isError
              ? 'color-mix(in oklab, var(--color-red-500) 16%, transparent)'
              : `color-mix(in oklab, ${color} 16%, transparent)`,
            color: isError ? 'var(--color-red-500)' : color,
          }}
        >
          <Icon className="size-4" />
        </span>

        <span
          className={cn(
            'relative z-10 min-w-0 truncate text-[13px] leading-5',
            hasChildren ? 'font-semibold' : 'font-medium',
            isError
              ? 'text-red-600 dark:text-red-400'
              : 'text-neutral-900 dark:text-neutral-50',
          )}
          title={span.name}
        >
          {name}
        </span>
        <span className="relative z-10 min-w-0 shrink truncate text-xs text-neutral-500 dark:text-neutral-400">
          {worker}
        </span>
        {span.pending ? (
          <Loader2
            className="relative z-10 size-4 shrink-0 animate-spin text-neutral-400 motion-reduce:animate-none"
            aria-label="running"
          />
        ) : null}
        {span.mergedRouting ? (
          <span
            className="relative z-10 shrink-0 rounded-md bg-neutral-200/80 px-1 font-mono text-[10px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400"
            title="an engine routing span was folded into this one"
          >
            +1
          </span>
        ) : null}
      </motion.div>

      {/* ── the bar ─────────────────────────────────────────────────── */}
      <div className="relative h-full min-w-0 pr-4">
        <div className="relative h-full">
          <motion.div
            initial={reduceMotion ? false : { scaleX: 0 }}
            animate={{ scaleX: 1 }}
            transition={{
              duration: 0.4,
              delay: delay / 1000,
              ease: [0.2, 0, 0, 1],
            }}
            className={cn(
              'absolute top-1/2 h-3.5 origin-left -translate-y-1/2 rounded-md',
              hasChildren && !isError
                ? 'ring-1 ring-inset ring-neutral-300 dark:ring-neutral-700'
                : null,
              isSelected && 'ring-2 ring-neutral-900 dark:ring-neutral-100',
              span.pending && 'animate-pulse',
            )}
            style={{
              left: `${left}%`,
              width: `${width}%`,
              background: isError
                ? 'linear-gradient(90deg, color-mix(in oklab, var(--color-red-500) 70%, transparent), var(--color-red-500))'
                : hasChildren
                  ? 'color-mix(in oklab, var(--color-neutral-500) 14%, transparent)'
                  : `linear-gradient(90deg, color-mix(in oklab, ${color} 55%, transparent), ${color})`,
            }}
          />
          {/* Duration (and the slowest tag) just after the bar; past the
              axis' end it flips to sit inside the bar's tail. */}
          <span
            className={cn(
              'absolute top-1/2 flex -translate-y-1/2 items-center gap-1.5 whitespace-nowrap text-xs tabular-nums',
              left + width > 82 ? '-translate-x-full pr-2' : 'pl-2',
              isError
                ? 'text-red-600 dark:text-red-400'
                : 'text-neutral-500 dark:text-neutral-400',
            )}
            style={{
              left: left + width > 82 ? `${left + width}%` : `${left + width}%`,
            }}
          >
            {formatDuration(span.duration_ms)}
            {span.pending ? '…' : null}
            {isSlowest ? (
              <span className="rounded-md bg-amber-500/15 px-1.5 py-px text-[11px] font-medium text-amber-600 dark:text-amber-400">
                Slowest
              </span>
            ) : null}
          </span>
        </div>
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

function displayReducer(
  state: DisplayState,
  action: DisplayAction,
): DisplayState {
  switch (action.type) {
    case 'TOGGLE_SPAN': {
      const next = new Set(state.expandedIds)
      if (next.has(action.spanId)) next.delete(action.spanId)
      else next.add(action.spanId)
      return { expandedIds: next }
    }
    case 'SET_ALL_EXPANDED':
      return { expandedIds: action.ids }
  }
}

interface HoverState {
  id: string
  x: number
  y: number
}

const TOOL_BUTTON =
  'inline-flex size-7 items-center justify-center rounded-lg text-neutral-500 transition-colors duration-[var(--motion-duration-control)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100'

interface ToolbarProps {
  expandAll: () => void
  collapseAll: () => void
  showExpandControls: boolean
  spanGroups: readonly SpanGroup[]
  workerGroups: readonly SpanGroup[]
  internalGroups: readonly SpanGroup[]
  spanFilter?: SpanFilterControls
  hiddenSpanCount: number
  visibleCount: number
  totalCount: number
}

function Toolbar({
  expandAll,
  collapseAll,
  showExpandControls,
  spanGroups,
  workerGroups,
  internalGroups,
  spanFilter,
  hiddenSpanCount,
  visibleCount,
  totalCount,
}: ToolbarProps) {
  const filterable =
    spanFilter &&
    (spanGroups.length > 0 ||
      workerGroups.length > 0 ||
      internalGroups.length > 0 ||
      hiddenSpanCount > 0)
  return (
    <div className="flex h-10 items-center justify-between px-3">
      <div className="flex items-center gap-1">
        {showExpandControls ? (
          <>
            <Tooltip>
              <TooltipTrigger
                render={
                  <button
                    type="button"
                    onClick={expandAll}
                    aria-label="expand all spans"
                    className={TOOL_BUTTON}
                  />
                }
              >
                <ChevronsDown className="size-4" aria-hidden />
              </TooltipTrigger>
              <TooltipContent side="bottom">Expand all</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger
                render={
                  <button
                    type="button"
                    onClick={collapseAll}
                    aria-label="collapse all spans"
                    className={TOOL_BUTTON}
                  />
                }
              >
                <ChevronsUp className="size-4" aria-hidden />
              </TooltipTrigger>
              <TooltipContent side="bottom">Collapse all</TooltipContent>
            </Tooltip>
          </>
        ) : null}
        {filterable && spanFilter ? (
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
            onClear={() => spanFilter.clear(internalGroups.map((g) => g.key))}
            className="ml-1 h-7"
          />
        ) : null}
      </div>
      <span className="text-xs text-neutral-500 tabular-nums dark:text-neutral-400">
        {visibleCount} of {totalCount} spans
      </span>
    </div>
  )
}

export function WaterfallChart({
  data,
  onSpanClick,
  selectedSpanId,
  spanGroupKey,
  spanFilter,
  showExpandControls = true,
}: WaterfallChartProps) {
  const reduceMotion = useReducedMotion() === true
  const [{ expandedIds }, dispatch] = useReducer(displayReducer, {
    expandedIds: new Set<string>(),
  })
  const containerRef = useRef<HTMLDivElement | null>(null)

  // The hidden-group/worker selection lives with the caller so the
  // waterfall and the timeline share one selection; this component only
  // derives the menu's entries and applies the selection.
  const filterEnabled = !!spanGroupKey && !!spanFilter
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

  // The span column's width: dragged on the axis divider, remembered.
  const [spanColWidth, setSpanColWidth] = useState<number>(() => {
    if (typeof window === 'undefined') return DEFAULT_SPAN_COL_WIDTH
    const saved = window.localStorage.getItem(SPAN_COL_WIDTH_KEY)
    const parsed = saved ? Number.parseInt(saved, 10) : Number.NaN
    return Number.isFinite(parsed)
      ? Math.min(MAX_SPAN_COL_WIDTH, Math.max(MIN_SPAN_COL_WIDTH, parsed))
      : DEFAULT_SPAN_COL_WIDTH
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
    let raf: number | null = null
    const onMove = (e: MouseEvent) => {
      if (!colResizeRef.current || raf !== null) return
      raf = requestAnimationFrame(() => {
        raf = null
        const drag = colResizeRef.current
        if (!drag) return
        setSpanColWidth(
          Math.min(
            MAX_SPAN_COL_WIDTH,
            Math.max(
              MIN_SPAN_COL_WIDTH,
              drag.startWidth + e.clientX - drag.startX,
            ),
          ),
        )
      })
    }
    const onUp = () => {
      if (!colResizeRef.current) return
      colResizeRef.current = null
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      if (raf !== null) cancelAnimationFrame(raf)
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
  const ticks = useMemo(
    () =>
      GRID_TICKS.map((pct) => ({
        pct,
        label: pct === 0 ? '0' : formatDuration((totalMs * pct) / 100),
      })),
    [totalMs],
  )

  const spanTree = useMemo(
    () => buildSpanTree(visibleData.spans),
    [visibleData.spans],
  )
  // Keyed on the RAW spans so only a trace switch re-expands everything.
  useEffect(() => {
    dispatch({
      type: 'SET_ALL_EXPANDED',
      ids: new Set(data.spans.map((s) => s.span_id)),
    })
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
  // The slowest LEAF: a parent's span is the sum of its children's, so
  // tagging it would say nothing a glance at the tree does not.
  const slowestId = useMemo(() => {
    let best: FlatSpanRow | null = null
    for (const row of visibleSpans) {
      if (row.children.length > 0 || row.pending) continue
      if (!best || row.duration_ms > best.duration_ms) best = row
    }
    return best && visibleSpans.length > 1 ? best.span_id : null
  }, [visibleSpans])
  const live = useMemo(() => data.spans.some((s) => s.pending), [data.spans])

  const virtualizer = useVirtualizer({
    count: visibleSpans.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 16,
  })
  const virtualRows = virtualizer.getVirtualItems()
  const contentHeight = virtualizer.getTotalSize()

  const toggleExpand = useCallback((spanId: string) => {
    dispatch({ type: 'TOGGLE_SPAN', spanId })
  }, [])

  // The hover card: plain state on the chart, stable callbacks on the
  // memo'd rows, so a mouse sweep re-renders the card and nothing else.
  const [hover, setHover] = useState<HoverState | null>(null)
  const handleHoverMove = useCallback(
    (span: VisualizationSpan, e: React.MouseEvent) => {
      if (colResizeRef.current) return
      setHover({ id: span.span_id, x: e.clientX, y: e.clientY })
    },
    [],
  )
  const handleHoverEnd = useCallback((spanId: string) => {
    setHover((prev) => (prev?.id === spanId ? null : prev))
  }, [])
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

  const columnStyle = {
    '--span-col-width': `${spanColWidth}px`,
  } as React.CSSProperties

  return (
    <div className="flex h-full flex-col font-inter" style={columnStyle}>
      <Toolbar
        expandAll={() =>
          dispatch({
            type: 'SET_ALL_EXPANDED',
            ids: new Set(data.spans.map((s) => s.span_id)),
          })
        }
        collapseAll={() =>
          dispatch({ type: 'SET_ALL_EXPANDED', ids: new Set() })
        }
        showExpandControls={showExpandControls}
        spanGroups={spanGroups}
        workerGroups={workerGroups}
        internalGroups={internalGroups}
        spanFilter={spanFilter}
        hiddenSpanCount={data.spans.length - visibleData.spans.length}
        visibleCount={visibleSpans.length}
        totalCount={data.span_count}
      />

      {/* The axis: "Span" over the tree, the ticks over the bars. */}
      <div className="grid h-8 shrink-0 grid-cols-[var(--span-col-width)_1fr] items-center border-b border-neutral-200 text-xs text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
        <div className="relative flex h-full items-center pl-3">
          <span className="font-medium">Span</span>
          {/* biome-ignore lint/a11y/useSemanticElements: <hr> cannot host a drag handle */}
          <div
            // biome-ignore lint/a11y/useAriaPropsForRole: a resizable column divider, no keyboard contract
            role="separator"
            aria-label="resize span column"
            aria-orientation="vertical"
            tabIndex={0}
            onMouseDown={startColResize}
            onDoubleClick={() => setSpanColWidth(DEFAULT_SPAN_COL_WIDTH)}
            title="drag to resize, double-click to reset"
            className="group absolute top-0 -right-1 bottom-0 z-10 w-2 cursor-col-resize"
          >
            <span className="absolute top-0 bottom-0 left-[3px] w-px bg-neutral-200 transition-colors group-hover:bg-neutral-500 dark:bg-neutral-800 dark:group-hover:bg-neutral-400" />
          </div>
        </div>
        <div className="relative h-full pr-4">
          {ticks.map(({ pct, label }) => (
            <span
              key={pct}
              className={cn(
                'absolute top-1/2 -translate-y-1/2 tabular-nums',
                pct === 100
                  ? '-translate-x-full'
                  : pct > 0 && '-translate-x-1/2',
              )}
              style={{ left: `${pct}%` }}
            >
              {label}
            </span>
          ))}
        </div>
      </div>

      {/* biome-ignore lint/a11y/noStaticElementInteractions: mouse-leave only dismisses the pointer-driven hover card */}
      <div
        ref={containerRef}
        className="relative min-h-0 flex-1 overflow-y-auto"
        onMouseLeave={() => setHover(null)}
      >
        <div style={{ height: contentHeight }} className="relative">
          {/* Gridlines run the full height behind the rows, one per tick,
              and the playhead on a live trace. */}
          <div
            aria-hidden
            className="pointer-events-none absolute inset-y-0 right-4"
            style={{ left: spanColWidth }}
          >
            {GRID_TICKS.map((pct) => (
              <span
                key={pct}
                className="absolute inset-y-0 w-px bg-neutral-200/80 dark:bg-neutral-800/80"
                style={{ left: `${pct}%` }}
              />
            ))}
            {live ? (
              <span className="absolute inset-y-0 right-0 w-px bg-sky-500">
                <span className="absolute -top-0 -left-[3px] size-[7px] rounded-full bg-sky-500 ring-2 ring-sky-500/30" />
              </span>
            ) : null}
          </div>

          {virtualRows.map((vrow) => {
            const span = visibleSpans[vrow.index]
            if (!span) return null
            return (
              <div
                key={vrow.key}
                data-index={vrow.index}
                className="absolute top-0 left-0 w-full"
                style={{
                  height: ROW_HEIGHT,
                  transform: `translateY(${vrow.start}px)`,
                }}
              >
                <WaterfallRow
                  span={span}
                  index={vrow.index}
                  isSelected={selectedSpanId === span.span_id}
                  isExpanded={expandedIds.has(span.span_id)}
                  isSlowest={slowestId === span.span_id}
                  reduceMotion={reduceMotion}
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

      {hover && hoveredCard ? (
        <SpanHoverCard
          span={hoveredCard.cardSpan}
          now={totalMs}
          x={hover.x}
          y={hover.y}
          relativeStart
          tracePercent={hoveredCard.tracePercent}
        />
      ) : null}
    </div>
  )
}
