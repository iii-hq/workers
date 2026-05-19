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
 *   useReducer<DisplayState>        ← expand/collapse + critical-path toggle
 *        │
 *        ▼
 *   flattenTree(tree, opts)         ← respects expandedIds, hideEngineRouting,
 *        │                            collapseEngineRoutingPairs; emits depth-offset
 *        │                            adjusted rows so hidden parents don't leave gaps
 *        ▼
 *   visibleSpans: FlatSpanRow[]
 *        │
 *        ▼
 *   Per-row render: indent guides, kind indicator, status dot, name,
 *                   duration, bar (status-colored or critical-path accent),
 *                   merged-routing `+1` chip when applicable
 *
 * Persistent state (localStorage):
 *   iii-trace-hide-engine-routing       boolean checkbox
 *   iii-trace-collapse-engine-pairs     boolean checkbox
 *   iii-span-col-width                  resizer position
 *
 * Schematic theming: all colors flow from `--color-*` CSS custom properties.
 * Bars use `bg-ink` / `bg-alert` / `bg-warn` / `bg-ink-ghost` for OK /
 * error / pending / unset. The critical path collapses onto the accent
 * (orange in light, electric-blue in dark) per DESIGN.md §3 — the single
 * accent moment per visible region.
 *
 * Engine-routing heuristics (hide/collapse) live in `../lib/spanLabel`.
 */

import { ChevronRight } from 'lucide-react'
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
import { cn } from '@/lib/utils'
import {
  formatSpanLabel,
  getSpanKindIndicator,
  isEngineRoutingSpan,
} from '../lib/spanLabel'
import { buildSpanTree, type FlatSpanRow, flattenTree } from '../lib/spanTree'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import { formatDuration } from '../lib/traceUtils'
import { computeVirtualWindow } from '../lib/virtualWindow'

interface WaterfallChartProps {
  data: WaterfallData
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string | null
}

interface WaterfallRowProps {
  span: FlatSpanRow
  isSelected: boolean
  isExpanded: boolean
  isCritical: boolean
  onSpanClick: (span: VisualizationSpan) => void
  onToggleExpand: (spanId: string) => void
}

const WaterfallRow = memo(function WaterfallRow({
  span,
  isSelected,
  isExpanded,
  isCritical,
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
  // alert for error, warn for unset/pending. Critical path
  // collapses onto the single accent moment per region per
  // DESIGN.md §3.
  const barClass = isCritical
    ? 'bg-accent'
    : isError
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
            <div
              key={key}
              className="w-4 h-6 border-l border-rule-2"
            />
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
  scrollPosition: number
}

type DisplayAction =
  | { type: 'TOGGLE_SPAN'; spanId: string }
  | { type: 'SET_ALL_EXPANDED'; ids: Set<string> }
  | { type: 'SET_CRITICAL_PATH'; value: boolean }
  | { type: 'SET_SCROLL'; position: number }

const initialDisplayState: DisplayState = {
  expandedIds: new Set(),
  showCriticalPath: false,
  scrollPosition: 0,
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
    case 'SET_SCROLL':
      // Cheap no-op when nothing changed — avoids a full re-render
      // when the rAF-throttled scroll handler fires the same scrollTop
      // twice in a row (e.g. when the user reaches a scroll boundary).
      if (state.scrollPosition === action.position) return state
      return { ...state, scrollPosition: action.position }
  }
}

const HIDE_ENGINE_KEY = 'iii-trace-hide-engine-routing'
const COLLAPSE_PAIRS_KEY = 'iii-trace-collapse-engine-pairs'
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

export function WaterfallChart({
  data,
  onSpanClick,
  selectedSpanId,
}: WaterfallChartProps) {
  const [displayState, dispatch] = useReducer(
    displayReducer,
    initialDisplayState,
  )
  const { expandedIds, showCriticalPath, scrollPosition } =
    displayState
  const containerRef = useRef<HTMLDivElement>(null)
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

  const [hideEngineRouting, setHideEngineRouting] = useState<boolean>(() => {
    if (typeof window === 'undefined') return false
    return window.localStorage.getItem(HIDE_ENGINE_KEY) === '1'
  })
  const [collapseEngineRoutingPairs, setCollapseEngineRoutingPairs] =
    useState<boolean>(() => {
      if (typeof window === 'undefined') return false
      return window.localStorage.getItem(COLLAPSE_PAIRS_KEY) === '1'
    })

  useEffect(() => {
    window.localStorage.setItem(HIDE_ENGINE_KEY, hideEngineRouting ? '1' : '0')
  }, [hideEngineRouting])
  useEffect(() => {
    window.localStorage.setItem(
      COLLAPSE_PAIRS_KEY,
      collapseEngineRoutingPairs ? '1' : '0',
    )
  }, [collapseEngineRoutingPairs])

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

  useEffect(() => {
    const allIds = new Set(data.spans.map((s) => s.span_id))
    dispatch({ type: 'SET_ALL_EXPANDED', ids: allIds })
  }, [data.spans])

  const visibleSpans = useMemo(
    () =>
      flattenTree(spanTree, {
        expandedIds,
        hideEngineRouting,
        collapseEngineRoutingPairs,
      }),
    [spanTree, expandedIds, hideEngineRouting, collapseEngineRoutingPairs],
  )

  // rAF-throttled scroll. The native `scroll` event fires far above the
  // browser's paint rate (trackpads can emit ~120Hz, freewheel scroll on
  // macOS gets even noisier). Dispatching on every event triggered a full
  // chart re-render each time — on a 2000+-span trace that flooded the
  // main thread and the page felt frozen. Coalescing to one update per
  // animation frame keeps state in sync with what the user can actually
  // see while letting the browser handle the bulk of the visual scroll.
  const scrollRafRef = useRef<number | null>(null)
  const pendingScrollRef = useRef<number>(0)
  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    pendingScrollRef.current = e.currentTarget.scrollTop
    if (scrollRafRef.current !== null) return
    scrollRafRef.current = requestAnimationFrame(() => {
      scrollRafRef.current = null
      dispatch({ type: 'SET_SCROLL', position: pendingScrollRef.current })
    })
  }
  useEffect(() => {
    return () => {
      if (scrollRafRef.current !== null) {
        cancelAnimationFrame(scrollRafRef.current)
        scrollRafRef.current = null
      }
    }
  }, [])

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

  const contentHeight = visibleSpans.length * ROW_HEIGHT
  const viewportRatio =
    containerHeight > 0 && contentHeight > 0
      ? containerHeight / contentHeight
      : 1
  const thumbHeight = Math.max(20, MINIMAP_HEIGHT * viewportRatio)
  const thumbPosition =
    contentHeight > 0 ? (scrollPosition / contentHeight) * MINIMAP_HEIGHT : 0

  // Fixed-height windowing: only render the rows currently visible plus
  // overscan. With ROW_HEIGHT=32 and a typical 800px viewport that is
  // ~25 rows + 16 overscan = ~41 DOM rows regardless of itemCount.
  // 4000-span traces no longer freeze the React commit phase. See
  // `lib/virtualWindow.ts` for the pure math.
  const virtualWindow = computeVirtualWindow({
    scrollTop: scrollPosition,
    containerHeight,
    rowHeight: ROW_HEIGHT,
    itemCount: visibleSpans.length,
  })
  const visibleSlice = visibleSpans.slice(
    virtualWindow.startIndex,
    virtualWindow.endIndex,
  )

  return (
    <div className="flex flex-col h-full">
      {/* sticky toolbar */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-rule bg-panel">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={expandAll}
            className="px-2 py-1 text-[11px] tracking-[0.06em] text-ink-faint hover:text-ink hover:bg-bg transition-colors lowercase"
          >
            expand all
          </button>
          <button
            type="button"
            onClick={collapseAll}
            className="px-2 py-1 text-[11px] tracking-[0.06em] text-ink-faint hover:text-ink hover:bg-bg transition-colors lowercase"
          >
            collapse all
          </button>
          <div aria-hidden className="w-px h-4 bg-rule-2 mx-1" />
          <label className="flex items-center gap-2 text-[11px] text-ink-faint cursor-pointer lowercase">
            <input
              type="checkbox"
              checked={showCriticalPath}
              onChange={(e) =>
                dispatch({ type: 'SET_CRITICAL_PATH', value: e.target.checked })
              }
              className="border-rule bg-bg text-accent focus:ring-accent/30"
            />
            show critical path
          </label>
          <label
            className="flex items-center gap-2 text-[11px] text-ink-faint cursor-pointer lowercase"
            title="merge each engine handle_invocation+call pair into one row"
          >
            <input
              type="checkbox"
              checked={collapseEngineRoutingPairs}
              onChange={(e) => setCollapseEngineRoutingPairs(e.target.checked)}
              className="border-rule bg-bg text-accent focus:ring-accent/30"
            />
            collapse routing pairs
          </label>
          <label
            className="flex items-center gap-2 text-[11px] text-ink-faint cursor-pointer lowercase"
            title="hide engine handle_invocation / call spans entirely"
          >
            <input
              type="checkbox"
              checked={hideEngineRouting}
              onChange={(e) => setHideEngineRouting(e.target.checked)}
              className="border-rule bg-bg text-accent focus:ring-accent/30"
            />
            hide engine routing
          </label>
        </div>
        <div className="text-[11px] text-ink-faint tabular-nums lowercase">
          {visibleSpans.length} of {data.span_count} spans
        </div>
      </div>

      {/* sticky time axis */}
      <div
        style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
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
          style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
          className="flex-1 overflow-y-auto"
          onScroll={handleScroll}
        >
          {/* Height spacer keeps the native scrollbar accurate; the inner
              `translateY` positions the slice at its real Y in the list.
              See `lib/virtualWindow.ts`. */}
          <div
            style={{ height: contentHeight, position: 'relative' }}
          >
            <div style={{ transform: `translateY(${virtualWindow.offsetY}px)` }}>
          {visibleSlice.map((span) => {
            return (
              <WaterfallRow
                key={span.span_id}
                span={span}
                isSelected={selectedSpanId === span.span_id}
                isExpanded={expandedIds.has(span.span_id)}
                isCritical={showCriticalPath && span.isCriticalPath}
                onSpanClick={onSpanClick}
                onToggleExpand={toggleExpand}
              />
            )
          })}
            </div>
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
              {data.spans.map((span, i) => {
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
                      top: `${(i / data.spans.length) * 100}%`,
                      left: `${span.start_percent}%`,
                      width: `${Math.max(2, span.width_percent)}%`,
                    }}
                  />
                )
              })}
              <div
                className="absolute left-0 right-0 bg-accent/20 border border-accent"
                style={{
                  top: thumbPosition,
                  height: thumbHeight,
                }}
              />
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
