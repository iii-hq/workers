/**
 * Waterfall view for a single trace's spans.
 *
 * Data flow:
 *
 *   props.data (WaterfallData)
 *        │
 *        ▼
 *   buildSpanTree(spans)            ← parent/child linking, marks critical path
 *        │
 *        ▼
 *   useReducer<DisplayState>        ← expand/collapse + critical-path-only toggle
 *        │
 *        ▼
 *   flattenTree(tree, opts)         ← respects expandedIds, hideEngineRouting,
 *        │                            collapseEngineRoutingPairs, onlyCriticalPath;
 *        │                            emits depth-offset adjusted rows so hidden
 *        │                            parents don't leave gaps
 *        ▼
 *   visibleSpans: FlatSpanRow[]
 *        │
 *        ▼
 *   Per-row render: indent guides, kind indicator, status dot, name,
 *                   duration, bar (status-colored), merged-routing `+1`
 *                   chip when applicable
 *
 * Toolbar toggles (driven by the user):
 *   show critical path  →  filter visibleSpans to only the slowest chain.
 *   show engine routing →  reveal `handle_invocation` / `call` pairs (off by
 *                          default). When on, pairs are always merged into
 *                          a single row with a `+1` affordance.
 *
 * Persistent state (localStorage):
 *   iii-trace-show-engine-routing       boolean — shared with FlameGraph
 *   iii-span-col-width                  resizer position
 *
 * Schematic theming: all colors flow from `--color-*` CSS custom properties.
 * Bars use `bg-ink` / `bg-alert` / `bg-ink-ghost` for OK / error / pending.
 *
 * Engine-routing heuristics live in `../lib/spanLabel`.
 */

import { useVirtualizer } from '@tanstack/react-virtual'
import {
  ChevronRight,
  ChevronsDown,
  ChevronsUp,
  Eye,
  Sparkles,
} from 'lucide-react'
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
import { useShowEngineRouting } from '../hooks/useShowEngineRouting'
import { sampleMinimapMarkers } from '../lib/minimapMarkers'
import {
  formatSpanLabel,
  getSpanKindIndicator,
  isEngineRoutingSpan,
} from '../lib/spanLabel'
import { buildSpanTree, type FlatSpanRow, flattenTree } from '../lib/spanTree'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import { formatDuration } from '../lib/traceUtils'
import { IconToggleButton } from './IconToggleButton'

interface WaterfallChartProps {
  data: WaterfallData
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string | null
}

interface WaterfallRowProps {
  span: FlatSpanRow
  isSelected: boolean
  isExpanded: boolean
  onSpanClick: (span: VisualizationSpan) => void
  onToggleExpand: (spanId: string) => void
}

const WaterfallRow = memo(function WaterfallRow({
  span,
  isSelected,
  isExpanded,
  onSpanClick,
  onToggleExpand,
}: WaterfallRowProps) {
  const effectiveChildren = span.mergedRouting
    ? (span.children[0]?.children ?? [])
    : span.children
  const hasChildren = effectiveChildren.length > 0
  const isEngineDim = !isSelected && isEngineRoutingSpan(span)
  const kindIndicator = getSpanKindIndicator(span.kind)
  const displayLabel = formatSpanLabel(span)
  const isError = span.status === 'error'

  // Schematic mapping: bar fill is monochrome ink for OK,
  // alert for error, ghost for unset/pending. The critical-path
  // toggle now filters the visible rows rather than re-coloring
  // them, so we no longer paint critical rows with the accent.
  const barClass = isError
    ? 'bg-alert'
    : span.status === 'ok'
      ? 'bg-ink'
      : 'bg-ink-ghost'

  // Hover is pure CSS (`hover:bg-panel`). Selected/error chrome
  // takes priority over hover via CSS specificity (more-specific
  // bg classes).
  const rowChrome = isSelected
    ? 'bg-panel border-l-2 border-l-accent'
    : isError
      ? 'bg-alert/5 border-l-2 border-l-alert'
      : 'hover:bg-panel'

  return (
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
    >
      <div
        className={cn(
          'flex items-center gap-1.5 min-w-0',
          isEngineDim && 'opacity-60',
        )}
      >
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

        {span.service_name && (
          <span className="flex-shrink-0 px-1.5 py-0.5 text-[10px] font-mono tracking-[0.06em] border border-rule bg-bg text-ink-faint leading-none lowercase">
            {span.service_name}
          </span>
        )}
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
          title={span.name}
        >
          {displayLabel}
        </span>
        {span.mergedRouting && (
          <span
            className="flex-shrink-0 px-1 py-0.5 text-[9px] font-mono tracking-[0.06em] border border-rule bg-panel text-ink-faint leading-none tabular-nums"
            title="merged: this row hides the engine 'call' child of a handle_invocation pair"
          >
            +1
          </span>
        )}

        <span className="font-mono text-[11px] text-ink-faint flex-shrink-0 ml-auto tabular-nums">
          {formatDuration(span.duration_ms)}
        </span>
      </div>

      {/* bar track */}
      <div className="relative h-6 bg-rule-2">
        <div
          className={cn(
            'absolute h-4 top-1 min-w-[3px]',
            barClass,
            isSelected && 'outline outline-2 outline-accent',
          )}
          style={{
            left: `${span.start_percent}%`,
            width: `${Math.max(0.5, span.width_percent)}%`,
          }}
          title={`${span.name} — ${formatDuration(span.duration_ms)}`}
        />
      </div>
    </div>
  )
})

interface DisplayState {
  expandedIds: Set<string>
  showCriticalPath: boolean
}

type DisplayAction =
  | { type: 'TOGGLE_SPAN'; spanId: string }
  | { type: 'SET_ALL_EXPANDED'; ids: Set<string> }
  | { type: 'SET_CRITICAL_PATH'; value: boolean }

const initialDisplayState: DisplayState = {
  expandedIds: new Set(),
  showCriticalPath: true,
}

// Note: `hoveredSpanId` used to live here, but mouse-sweep over the
// chart fired one dispatch per row entered/exited — each one a full
// re-render of every visible row. For a 2000+-span trace that froze
// the page on mouse-move. The only consumer of the JS hover state was
// row-background styling, which Tailwind's `hover:bg-panel` already
// provides via CSS. Keeping the state was redundant work.

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
    case 'SET_CRITICAL_PATH':
      return { ...state, showCriticalPath: action.value }
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

// Compact icon-driven toolbar. The two view toggles (critical-path-only,
// show engine routing) are square icon buttons whose `pressed` state
// fills with ink; their meaning is exposed via hover tooltips.
// Expand/collapse-all sit at the start as plain icon buttons. Engine
// routing is hidden by default; when shown, routing pairs are always
// merged into a single row.
interface ToolbarProps {
  expandAll: () => void
  collapseAll: () => void
  showCriticalPath: boolean
  setShowCriticalPath: (value: boolean) => void
  showEngineRouting: boolean
  setShowEngineRouting: (value: boolean) => void
  visibleCount: number
  totalCount: number
}

function Toolbar(props: ToolbarProps) {
  const {
    expandAll,
    collapseAll,
    showCriticalPath,
    setShowCriticalPath,
    showEngineRouting,
    setShowEngineRouting,
    visibleCount,
    totalCount,
  } = props
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-rule bg-panel">
      <div className="flex items-center gap-1.5">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={expandAll}
              aria-label="expand all spans"
              className="inline-flex items-center justify-center w-7 h-7 border bg-bg text-ink-faint border-rule hover:text-ink hover:border-ink transition-colors"
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
              className="inline-flex items-center justify-center w-7 h-7 border bg-bg text-ink-faint border-rule hover:text-ink hover:border-ink transition-colors"
            >
              <ChevronsUp className="w-3.5 h-3.5" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">collapse all</TooltipContent>
        </Tooltip>
        <div aria-hidden className="w-px h-4 bg-rule-2 mx-1" />
        <IconToggleButton
          active={showCriticalPath}
          onClick={() => setShowCriticalPath(!showCriticalPath)}
          label="only critical path"
        >
          <Sparkles className="w-3.5 h-3.5" />
        </IconToggleButton>
        <IconToggleButton
          active={showEngineRouting}
          onClick={() => setShowEngineRouting(!showEngineRouting)}
          label="show engine routing spans"
        >
          <Eye className="w-3.5 h-3.5" />
        </IconToggleButton>
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
}: WaterfallChartProps) {
  const [displayState, dispatch] = useReducer(
    displayReducer,
    initialDisplayState,
  )
  const { expandedIds, showCriticalPath } = displayState
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

  // Engine routing spans (`handle_invocation X`, `call X` on the `iii`
  // service) are hidden by default; toggling reveals them in their
  // always-merged form (one row per handle/call pair, marked with `+1`).
  // The persisted hook is shared with the flame-graph view so the
  // preference survives view swaps.
  const [showEngineRouting, setShowEngineRouting] = useShowEngineRouting()

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

  const spanTree = useMemo(() => buildSpanTree(data.spans), [data.spans])

  // Bounded set of minimap markers — never one DOM node per span. Memoized
  // so it only recomputes when the span list changes.
  const minimapMarkers = useMemo(
    () => sampleMinimapMarkers(data.spans),
    [data.spans],
  )

  useEffect(() => {
    const allIds = new Set(data.spans.map((s) => s.span_id))
    dispatch({ type: 'SET_ALL_EXPANDED', ids: allIds })
  }, [data.spans])

  const visibleSpans = useMemo(
    () =>
      flattenTree(spanTree, {
        expandedIds,
        hideEngineRouting: !showEngineRouting,
        // Showing engine routing always merges each handle/call pair —
        // there's no separate user toggle for un-merged routing.
        collapseEngineRoutingPairs: showEngineRouting,
        onlyCriticalPath: showCriticalPath,
      }),
    [spanTree, expandedIds, showEngineRouting, showCriticalPath],
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
    showCriticalPath,
    setShowCriticalPath: (value: boolean) =>
      dispatch({ type: 'SET_CRITICAL_PATH', value }),
    showEngineRouting,
    setShowEngineRouting,
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
        className="grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-2 text-[11px] font-semibold text-ink-ghost uppercase tracking-[0.06em] border-b border-rule bg-bg"
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
        <div
          ref={containerRef}
          style={
            { '--span-col-width': `${spanColWidth}px` } as React.CSSProperties
          }
          className="flex-1 overflow-y-auto"
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
                  />
                </div>
              )
            })}
          </div>
        </div>

        {/* minimap */}
        {contentHeight > containerHeight && (
          <div className="w-16 border-l border-rule bg-panel flex-shrink-0 relative p-2">
            <div className="text-[9px] text-ink-ghost uppercase tracking-[0.06em] mb-2">
              map
            </div>
            <div
              className="relative bg-rule-2 overflow-hidden"
              style={{ height: MINIMAP_HEIGHT }}
            >
              {minimapMarkers.map((marker) => (
                <div
                  key={marker.key}
                  className={cn(
                    'absolute h-[2px]',
                    marker.isError ? 'bg-alert' : 'bg-ink-ghost',
                  )}
                  style={{
                    opacity: marker.isError ? 0.7 : 0.5,
                    top: `${marker.topPercent}%`,
                    left: `${marker.leftPercent}%`,
                    width: `${Math.max(2, marker.widthPercent)}%`,
                  }}
                />
              ))}
              <div
                ref={thumbRef}
                className="absolute left-0 right-0 bg-accent/20 border border-accent"
                style={{ top: 0, height: 20 }}
              />
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
