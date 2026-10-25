/**
 * Static trace-scoped timeline — the detail-view sibling of the live
 * Timeline strip (it replaced the flame graph in the view switcher).
 *
 * Where the live strip packs spans into thread lanes, this view is
 * HIERARCHICAL: every parent sits on a line above the spans it started,
 * and nothing ever collapses into chips — all spans are always visible.
 * Sequential siblings (non-overlapping in time) SHARE one line, exactly
 * like a flame graph; only genuinely concurrent subtrees stack onto
 * extra lines below (see `buildLayout`). Bars keep honest time
 * coordinates over a FIXED window — exactly [trace start, trace end],
 * proportional ruler on top — but each child bar is additionally inset a
 * small padding from its parent's left edge, and elbow connectors tie
 * parents to their children, so the "who started what" chain stays
 * readable even when a child starts on the same instant as its parent.
 *
 * The lines stack as tall as they need; when they outgrow the viewport
 * it pans by mouse drag (wheel scrolling works too — the grab cursor
 * signals the affordance). A press only becomes a pan after a few px of
 * movement, so bar clicks stay clicks.
 *
 * The canvas also grows HORIZONTALLY when the bars need more room than
 * the time axis offers: min-width inflation, sibling gaps and hierarchy
 * indents accumulate (hundreds of sequential ms-scale spans on one line),
 * and clamping them into the viewport used to crush the tail of dense
 * traces into an unreadable pile at the right edge. Instead the content
 * takes the width the packed bars actually need and the viewport scrolls
 * on both axes; the ruler is sticky (pinned while panning down, gliding
 * with the bars while panning right) and the time grid scrolls with the
 * content it measures. Bars past the 100% gridline are the structural
 * spill — time-wise they live at the trace tail, the hover card carries
 * the honest numbers.
 *
 * Hovering a bar shows the shared SpanHoverCard (with % of trace);
 * clicking selects the underlying `VisualizationSpan` (opens the span
 * panel). Selection = 2px ink ring, hover = 1px half-ink ring (the recipe
 * in `spanVisuals.ts`, shared with the waterfall). A bar is a solid block
 * of its worker's hue (alert on error) — a `*-900` dark badge with a
 * `*-800` inset edge, the same colour the masthead strip and the legend
 * use for that worker — with the label on it in the hue's own `*-50` ink. A
 * kind glyph in the same ink sits before the label only on the bars that
 * are NOT plain function invocations (an inbound root, an outbound client
 * call, a queue hop): the ordinary `lambda` bar is the common case, and a
 * glyph on every one of them said nothing. Lines sit on a 32px rhythm
 * (24px bar, 8px gap) so a dense trace stays a diagram instead of a wall.
 *
 * A toolbar row sits above the canvas: the caller's view switcher on the
 * left (`toolbarStart`), the span filter and the visible-span count on the
 * right — the same row the waterfall draws, so switching views never moves
 * the controls. Under it, the worker legend (`WorkerLegend`): one chip per
 * worker, swatch and name, each toggling that worker's spans.
 */

import {
  Fragment,
  memo,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { cn } from '@/lib/utils'
import type { SpanFilterControls } from '../../lib/spanFilters'
import { internalFamilyOf } from '../../lib/spanLabel'
import { waterfallToTimelineSpans } from '../../lib/timelineSpans'
import { borderOnFill, inkOnFill } from '../../lib/traceColors'
import type { VisualizationSpan, WaterfallData } from '../../lib/traceTransform'
import { formatDuration } from '../../lib/traceUtils'
import {
  BAR_HEIGHT,
  MIN_BAR_WIDTH,
  type TimelineSpan,
  type TimelineSpanKind,
} from './layout'
import { SpanFilterMenu } from './SpanFilterMenu'
import { SpanHoverCard } from './SpanHoverCard'
import {
  applyHiddenSpanFilters,
  deriveSpanGroups,
  type SpanGroupKey,
  workerGroupKey,
} from './spanVisibility'
import {
  BarLabel,
  KIND_ICONS,
  resolveColor,
  ringFor,
  showsKindGlyph,
} from './spanVisuals'
import { WorkerLegend } from './WorkerLegend'

export interface TraceTimelineProps {
  data: WaterfallData
  onSpanClick?: (span: VisualizationSpan) => void
  selectedSpanId?: string
  className?: string
  /**
   * Grouping for the filter menu's spans section: the menu lists groups
   * most-populated first, and hiding a group hides ONLY its spans — their
   * children re-attach to the hidden span's parent. WHAT a group is stays
   * the caller's business — see `lib/traceTimelineFilters.ts` for the
   * page's grouping.
   */
  spanGroupKey?: SpanGroupKey
  /**
   * Selection + mutations behind the floating filter menu (funnel,
   * top-right of the canvas). Owned by the caller so the same selection
   * can be shared with the waterfall view and persisted (see
   * `hooks/useSpanFilterSelection.ts`). The menu renders only when BOTH
   * this and `spanGroupKey` are provided.
   */
  spanFilter?: SpanFilterControls
  /**
   * Size the canvas to the packed lines instead of filling the parent —
   * for inline/accordion placement where the surrounding page scrolls.
   * Still capped (60dvh) so a huge trace keeps its internal pan and the
   * sticky ruler rather than becoming a screens-tall block.
   */
  fitContent?: boolean
  /** Rendered at the start of the toolbar row (the detail's view switcher). */
  toolbarStart?: ReactNode
}

const PADDING_X = 16
const RULER_FRACTIONS = [0, 0.25, 0.5, 0.75, 1] as const
/** A trace with no pending span counts as SETTLED only after this long: a
 *  running turn has nothing pending for a few milliseconds between two
 *  engine calls, and shrinking the stage on that gap is the very jump the
 *  grow-only rule below exists to prevent. */
const SETTLE_MS = 3_000

/** height of the sticky time ruler row inside the scrollport (h-5) */
const RULER_H = 20
/** vertical rhythm of the stacked lines: a 24px bar and an 8px gap */
const ROW_PITCH = 32
const CONTENT_PAD_Y = 10
/** minimum left inset of a child bar from its parent's — the hierarchy padding */
const DEPTH_INDENT = 14
/** breathing room between sequential bars sharing a line */
const SIBLING_GAP = 4
/** the connector rail drops from under the parent bar's coloured edge */
const RAIL_INSET = 1
/** movement (px) before a press turns into a pan instead of a click */
const DRAG_THRESHOLD_PX = 4
/** mono 12px semibold glyph advance — for estimating whether a label fits its bar */
const LABEL_CHAR_PX = 7.25
/** the bar's padding on both sides of the label */
const BAR_LABEL_OVERHEAD_PX = 16
/** a kind glyph (16px) and the gap before the label, when a bar shows one */
const BAR_GLYPH_PX = 22
const MIN_SPILL_LABEL_PX = 40
/** below this a truncated in-bar label is pure noise — leave the bar blank */
const MIN_INBAR_LABEL_CHARS = 4

interface PlacedSpan {
  span: TimelineSpan
  /** vertical line index (0 = top) */
  line: number
  /** index into the placed array; emission is pre-order so parent < child */
  parentIndex: number | null
}

interface TraceLayout {
  placed: PlacedSpan[]
  lineCount: number
  /** each span's packed offset relative to its parent — fed back into the
   *  next `buildLayout` so a streaming re-pack keeps bars on their lines */
  offsets: ReadonlyMap<string, number>
}

/** A subtree's footprint: the lines it spans and its full time extent. */
interface SubtreeInfo {
  /** lines this subtree occupies, its own line included */
  height: number
  minStart: number
  maxEnd: number
  /** line offset relative to the parent's line, assigned while packing */
  offset: number
}

interface BarRect {
  left: number
  width: number
}

/** Lines and pixels kept rendered beyond the viewport, so a pan never
 *  shows a bar popping in at the edge. */
const OVERSCAN_LINES = 4
const OVERSCAN_PX = 240

interface DragState {
  pointerId: number
  startX: number
  startY: number
  startScrollLeft: number
  startScrollTop: number
  active: boolean
}

function rowTop(line: number): number {
  return CONTENT_PAD_Y + line * ROW_PITCH
}

interface BarProps {
  id: string
  label: string
  sourceName: string | undefined
  ariaLabel: string
  top: number
  left: number
  width: number
  color: string
  /** the ink that reads on `color` — the label and the glyph wear it */
  ink: string
  /** the hue's one-step-lighter edge, drawn as an inset hairline */
  border: string
  kind: TimelineSpanKind
  selected: boolean
  hovered: boolean
  inBarLabel: boolean
  /** still running: the right edge fades out and an amber sweep crosses it */
  running: boolean
  /** play the entrance animation (first appearance only) */
  enter: boolean
  clickable: boolean
  onEnter: (id: string) => void
  onLeave: (id: string) => void
  onClick: (id: string) => void
}

/**
 * One bar. Memoised on its own props, so a hover or a selection re-renders
 * the two bars it touches and nothing else, and a streaming re-pack only
 * re-renders the bars whose geometry moved.
 */
const TimelineBar = memo(function TimelineBar({
  id,
  label,
  sourceName,
  ariaLabel,
  top,
  left,
  width,
  color,
  ink,
  border,
  kind,
  selected,
  hovered,
  inBarLabel,
  running,
  enter,
  clickable,
  onEnter,
  onLeave,
  onClick,
}: BarProps) {
  const Icon = showsKindGlyph(kind) ? KIND_ICONS[kind] : null
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      data-trace-span-id={id}
      data-trace-span-name={sourceName}
      onMouseEnter={() => onEnter(id)}
      onMouseLeave={() => onLeave(id)}
      onClick={clickable ? () => onClick(id) : undefined}
      aria-pressed={selected}
      className={cn(
        'timeline-glide absolute flex items-center gap-1.5 overflow-hidden rounded-md px-2 transition-[box-shadow] duration-[var(--motion-duration-fast)] ease-[var(--motion-ease-standard)]',
        'focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring',
        enter && 'timeline-enter',
        running && 'trace-running',
        clickable ? 'cursor-pointer' : 'cursor-default',
      )}
      style={{
        top,
        height: BAR_HEIGHT,
        left,
        width,
        backgroundColor: color,
        color: ink,
        // The hue's own edge first, then the state ring on top of it.
        boxShadow: [`inset 0 0 0 1px ${border}`, ringFor(selected, hovered)]
          .filter(Boolean)
          .join(', '),
        zIndex: selected || hovered ? 5 : undefined,
      }}
    >
      {/* The kind glyph, only on the bars with a kind worth telling apart
          (root, client call, queue hop); it and the label inherit the
          bar's ink. */}
      {Icon ? (
        <Icon
          aria-hidden
          data-span-kind={kind}
          className="size-4 shrink-0"
          strokeWidth={2.25}
        />
      ) : null}
      {inBarLabel && <BarLabel text={label} />}
    </button>
  )
})

interface Rail {
  id: string
  left: number
  top: number
  height: number
}
interface Stub {
  id: string
  left: number
  top: number
  width: number
}

/** The elbow connectors in view. Memoised: a hover elsewhere on the canvas
 *  must not re-reconcile a thousand hairlines. */
const ConnectorLayer = memo(function ConnectorLayer({
  rails,
  stubs,
}: {
  rails: readonly Rail[]
  stubs: readonly Stub[]
}) {
  return (
    <>
      {rails.map((rail) => (
        <div
          key={rail.id}
          aria-hidden
          className="timeline-glide absolute w-px bg-ink/15"
          style={{ left: rail.left, top: rail.top, height: rail.height }}
        />
      ))}
      {stubs.map((stub) => (
        <div
          key={stub.id}
          aria-hidden
          className="timeline-glide absolute h-px bg-ink/15"
          style={{ left: stub.left, top: stub.top, width: stub.width }}
        />
      ))}
    </>
  )
})

/**
 * The hover card, positioned at the pointer. It owns the pointer position
 * itself (a native listener on the viewport), so the per-pixel mouse
 * movement never reaches the timeline's React state — only entering or
 * leaving a bar does.
 */
function HoverCardLayer({
  span,
  total,
  viewportRef,
  pointerRef,
}: {
  span: TimelineSpan
  total: number
  viewportRef: React.RefObject<HTMLDivElement | null>
  pointerRef: React.RefObject<{ x: number; y: number }>
}) {
  const [pos, setPos] = useState(() => pointerRef.current)
  useEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const onMove = (e: PointerEvent) => setPos({ x: e.clientX, y: e.clientY })
    el.addEventListener('pointermove', onMove, { passive: true })
    return () => el.removeEventListener('pointermove', onMove)
  }, [viewportRef])
  return (
    <SpanHoverCard
      span={span}
      now={total}
      x={pos.x}
      y={pos.y}
      relativeStart
      tracePercent={(((span.endTime ?? total) - span.startTime) / total) * 100}
    />
  )
}

/**
 * Greedy first-fit packing of sibling subtrees, flame-graph style.
 *
 * Each subtree is a solid rectangle (time extent × line span). Siblings
 * are placed in start order at the smallest line offset ≥ `firstOffset`
 * where they collide with no earlier sibling — so sequential subtrees
 * land on the SAME line and only time-overlapping (concurrent) subtrees
 * stack further down. Rectangles never interleave, which keeps every
 * subtree a contiguous visual block.
 *
 * Sticky re-packs: when `prev` holds a span's offset from the previous
 * layout and the subtree still fits there, it stays — a streaming trace
 * re-packs on every span, and without the preference a growing sibling
 * reshuffles lines that were already settled. Only spans whose remembered
 * spot now collides fall back to first-fit.
 *
 * Mutates each child's `info.offset`; returns the deepest occupied
 * offset+height (i.e. lines consumed), or `firstOffset` when empty.
 */
function packSubtrees(
  children: readonly VisualizationSpan[],
  info: ReadonlyMap<string, SubtreeInfo>,
  firstOffset: number,
  prev?: ReadonlyMap<string, number>,
): number {
  const placed: SubtreeInfo[] = []
  let deepest = firstOffset
  const collides = (rect: SubtreeInfo, offset: number): boolean => {
    for (const other of placed) {
      const timeOverlap =
        rect.minStart < other.maxEnd && other.minStart < rect.maxEnd
      const lineOverlap =
        offset < other.offset + other.height &&
        other.offset < offset + rect.height
      if (timeOverlap && lineOverlap) return true
    }
    return false
  }
  for (const child of children) {
    const rect = info.get(child.span_id)
    if (!rect) continue
    const kept = prev?.get(child.span_id)
    let offset: number
    if (kept != null && kept >= firstOffset && !collides(rect, kept)) {
      // ponytail: holes a departed subtree leaves behind persist until the
      // remembered spot collides; re-compact from scratch if sparse layouts
      // ever matter more than stability.
      offset = kept
    } else {
      offset = firstOffset
      while (collides(rect, offset)) {
        for (const other of placed) {
          const timeOverlap =
            rect.minStart < other.maxEnd && other.minStart < rect.maxEnd
          const lineOverlap =
            offset < other.offset + other.height &&
            other.offset < offset + rect.height
          if (timeOverlap && lineOverlap) offset = other.offset + other.height
        }
      }
    }
    rect.offset = offset
    placed.push(rect)
    deepest = Math.max(deepest, offset + rect.height)
  }
  return deepest
}

/**
 * Resolve the trace tree into positioned lines.
 *
 * Two passes, both iterative so a multi-thousand-deep parent chain can't
 * blow the call stack:
 * - post-order: compute each subtree's footprint (extent + height) and
 *   pack every node's children with `packSubtrees`;
 * - pre-order: turn the relative offsets into absolute lines, emitting
 *   parents before children (renderers rely on `parentIndex < index`).
 *
 * Spans whose parent chain never reaches a root (malformed cycles) are
 * appended on their own lines at the bottom, so every span of the trace
 * is always visible.
 */
function buildLayout(
  source: readonly VisualizationSpan[],
  spans: readonly TimelineSpan[],
  prev?: ReadonlyMap<string, number>,
): TraceLayout {
  const byId = new Map(spans.map((s) => [s.id, s]))
  const childrenOf = new Map<string, VisualizationSpan[]>()
  const roots: VisualizationSpan[] = []
  for (const s of source) {
    if (s.parent_span_id && byId.has(s.parent_span_id)) {
      const siblings = childrenOf.get(s.parent_span_id)
      if (siblings) siblings.push(s)
      else childrenOf.set(s.parent_span_id, [s])
    } else {
      roots.push(s)
    }
  }

  const startOf = (v: VisualizationSpan) =>
    byId.get(v.span_id)?.startTime ?? Number.POSITIVE_INFINITY
  const byStart = (a: VisualizationSpan, b: VisualizationSpan) =>
    startOf(a) - startOf(b) || (a.span_id < b.span_id ? -1 : 1)
  roots.sort(byStart)
  for (const list of childrenOf.values()) list.sort(byStart)

  // Post-order: subtree footprints. `seen` is marked on push so a
  // malformed cycle can never re-enter the stack.
  const info = new Map<string, SubtreeInfo>()
  const seen = new Set<string>()
  const postStack: Array<{ node: VisualizationSpan; expanded: boolean }> = []
  for (let i = roots.length - 1; i >= 0; i--) {
    seen.add(roots[i].span_id)
    postStack.push({ node: roots[i], expanded: false })
  }
  while (postStack.length > 0) {
    const frame = postStack[postStack.length - 1]
    const id = frame.node.span_id
    const kids = childrenOf.get(id)
    if (!frame.expanded) {
      frame.expanded = true
      if (kids) {
        for (let i = kids.length - 1; i >= 0; i--) {
          if (seen.has(kids[i].span_id)) continue
          seen.add(kids[i].span_id)
          postStack.push({ node: kids[i], expanded: false })
        }
      }
      continue
    }
    postStack.pop()
    const span = byId.get(id)
    if (!span) continue
    let minStart = span.startTime
    let maxEnd = span.endTime ?? span.startTime
    if (kids) {
      for (const kid of kids) {
        const ki = info.get(kid.span_id)
        if (!ki) continue
        if (ki.minStart < minStart) minStart = ki.minStart
        if (ki.maxEnd > maxEnd) maxEnd = ki.maxEnd
      }
    }
    const height = kids ? Math.max(1, packSubtrees(kids, info, 1, prev)) : 1
    info.set(id, { height, minStart, maxEnd, offset: 0 })
  }

  let lineCount = packSubtrees(roots, info, 0, prev)

  // Pre-order: absolute lines, parents emitted before their children.
  const placed: PlacedSpan[] = []
  const preStack: Array<{
    node: VisualizationSpan
    line: number
    parentIndex: number | null
  }> = []
  for (let i = roots.length - 1; i >= 0; i--) {
    const ri = info.get(roots[i].span_id)
    if (!ri) continue
    preStack.push({ node: roots[i], line: ri.offset, parentIndex: null })
  }
  while (preStack.length > 0) {
    const frame = preStack.pop() as (typeof preStack)[number]
    const span = byId.get(frame.node.span_id)
    if (!span) continue
    const index = placed.length
    placed.push({ span, line: frame.line, parentIndex: frame.parentIndex })
    const kids = childrenOf.get(frame.node.span_id)
    if (!kids) continue
    for (let i = kids.length - 1; i >= 0; i--) {
      const ki = info.get(kids[i].span_id)
      if (!ki) continue
      preStack.push({
        node: kids[i],
        line: frame.line + ki.offset,
        parentIndex: index,
      })
    }
  }

  if (placed.length < source.length) {
    const emitted = new Set(placed.map((p) => p.span.id))
    for (const s of source) {
      if (emitted.has(s.span_id)) continue
      const span = byId.get(s.span_id)
      if (span) placed.push({ span, line: lineCount++, parentIndex: null })
    }
  }

  const offsets = new Map<string, number>()
  for (const [id, i] of info) offsets.set(id, i.offset)
  return { placed, lineCount, offsets }
}

export function TraceTimeline({
  data,
  onSpanClick,
  selectedSpanId,
  className,
  spanGroupKey,
  spanFilter,
  fitContent,
  toolbarStart,
}: TraceTimelineProps) {
  const stageRef = useRef<HTMLDivElement>(null)
  const viewportRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<DragState | null>(null)
  const suppressClickRef = useRef(false)
  const [stage, setStage] = useState({ width: 0, height: 0 })
  // Only the hovered bar's identity is React state; the pointer position
  // lives in a ref and the hover card tracks it on its own.
  const [hoverId, setHoverId] = useState<string | null>(null)
  const pointerRef = useRef({ x: 0, y: 0 })
  const [dragging, setDragging] = useState(false)
  // The viewport's scroll offset, for culling: updated at most once per
  // frame, and only once it moved far enough to change what's visible.
  const [scroll, setScroll] = useState({ left: 0, top: 0 })
  const scrollFrameRef = useRef(0)
  const onScroll = useCallback(() => {
    if (scrollFrameRef.current) return
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = 0
      const el = viewportRef.current
      if (!el) return
      setScroll((prev) =>
        Math.abs(prev.left - el.scrollLeft) < OVERSCAN_PX / 2 &&
        Math.abs(prev.top - el.scrollTop) < (OVERSCAN_LINES * ROW_PITCH) / 2
          ? prev
          : { left: el.scrollLeft, top: el.scrollTop },
      )
    })
  }, [])
  useEffect(() => () => cancelAnimationFrame(scrollFrameRef.current), [])

  // The hidden-group/worker selection lives with the CALLER (`spanFilter`)
  // so the timeline and waterfall share one selection and it persists in
  // the console configuration. This component only derives menu entries
  // and applies the selection.
  const filterEnabled = !!spanGroupKey && !!spanFilter

  // Menu entries against the FULL data (an already-hidden group must keep
  // its row so it can be turned back on), busiest first. The group key
  // gets the whole trace by id so parent-dependent grouping (tag roots)
  // agrees with what `applyHiddenSpanFilters` hides.
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

  // The legend lists the trace's workers whether or not the filter is
  // wired (read-only then), busiest first like the funnel's section.
  const legendWorkers = useMemo(
    () =>
      filterEnabled
        ? workerGroups
        : deriveSpanGroups(data.spans, (s) =>
            internalFamilyOf(s.attributes) ? null : workerGroupKey(s),
          ),
    [data.spans, filterEnabled, workerGroups],
  )

  const visibleData = useMemo(
    () =>
      filterEnabled
        ? applyHiddenSpanFilters(data, spanGroupKey, spanFilter)
        : data,
    [data, spanGroupKey, spanFilter, filterEnabled],
  )

  useLayoutEffect(() => {
    const el = stageRef.current
    if (!el) return
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0].contentRect
      setStage({ width: rect.width, height: rect.height })
    })
    observer.observe(el)
    setStage({ width: el.clientWidth, height: el.clientHeight })
    return () => observer.disconnect()
  }, [])

  const detail = useMemo(
    () => waterfallToTimelineSpans(visibleData),
    [visibleData],
  )
  const total = Math.max(detail.totalDurationMs, 1)

  // A cache, never a dependency: each layout prefers the lines the previous
  // one assigned, so a streaming re-pack moves as few bars as possible.
  const stickyOffsets = useRef<ReadonlyMap<string, number>>(new Map())
  const layout = useMemo(
    () => buildLayout(visibleData.spans, detail.spans, stickyOffsets.current),
    [visibleData.spans, detail.spans],
  )
  stickyOffsets.current = layout.offsets

  const innerWidth = Math.max(stage.width - PADDING_X * 2, 0)
  const pxPerMs = innerWidth > 0 ? innerWidth / total : 0
  const x = (t: number) => PADDING_X + t * pxPerMs

  // Bar geometry in emission order. A child's left edge is
  // time-positioned but never closer than DEPTH_INDENT to its parent's —
  // the small hierarchy padding that keeps the cascade readable when
  // starts (nearly) coincide. Bars sharing a line additionally never
  // overlap the previous bar (MIN_BAR_WIDTH can inflate tiny spans).
  // Bars are NEVER clamped to the viewport: a dense line simply runs past
  // the time axis and the canvas widens to `maxRight` (the horizontal
  // scroll), instead of piling the tail into the right edge.
  const { rects, maxRight } = useMemo(() => {
    const out: BarRect[] = []
    const lineRight = new Map<number, number>()
    let maxRight = 0
    for (const p of layout.placed) {
      const end = p.span.endTime ?? total
      let left = PADDING_X + p.span.startTime * pxPerMs
      if (p.parentIndex != null) {
        left = Math.max(left, out[p.parentIndex].left + DEPTH_INDENT)
      }
      const prevRight = lineRight.get(p.line)
      if (prevRight != null) left = Math.max(left, prevRight + SIBLING_GAP)
      const width = Math.max(PADDING_X + end * pxPerMs - left, MIN_BAR_WIDTH)
      out.push({ left, width })
      lineRight.set(p.line, left + width)
      if (left + width > maxRight) maxRight = left + width
    }
    return { rects: out, maxRight }
  }, [layout, pxPerMs, total])

  // Elbow connectors: one rail dropping from each parent's left edge down
  // to its lowest child line, and one horizontal stub reaching the FIRST
  // child on each line — later bars on a shared line read as the
  // sequential continuation of that elbow, so they get no stub of their own.
  const connectors = useMemo(() => {
    const lowestChildLine = new Map<number, number>()
    const stubs: Array<{
      id: string
      left: number
      top: number
      width: number
    }> = []
    const stubbedLines = new Set<string>()
    layout.placed.forEach((p, i) => {
      if (p.parentIndex == null) return
      const current = lowestChildLine.get(p.parentIndex)
      if (current == null || p.line > current) {
        lowestChildLine.set(p.parentIndex, p.line)
      }
      const lineKey = `${p.parentIndex}:${p.line}`
      if (stubbedLines.has(lineKey)) return
      stubbedLines.add(lineKey)
      const from = rects[p.parentIndex].left + RAIL_INSET
      stubs.push({
        id: p.span.id,
        left: from,
        top: rowTop(p.line) + BAR_HEIGHT / 2,
        width: Math.max(rects[i].left - from, 0),
      })
    })
    const rails = [...lowestChildLine].map(([parent, lowest]) => {
      const top = rowTop(layout.placed[parent].line) + BAR_HEIGHT
      return {
        id: layout.placed[parent].span.id,
        left: rects[parent].left + RAIL_INSET,
        top,
        height: rowTop(lowest) + BAR_HEIGHT / 2 - top,
      }
    })
    return { rails, stubs }
  }, [layout, rects])

  // The next bar on the same line (emission order is left-to-right within
  // a line) — bounds how far a spilled label may run.
  const nextOnLine = useMemo(() => {
    const next: Array<number | null> = Array(layout.placed.length).fill(null)
    const lastByLine = new Map<number, number>()
    layout.placed.forEach((p, i) => {
      const prev = lastByLine.get(p.line)
      if (prev != null) next[prev] = i
      lastByLine.set(p.line, i)
    })
    return next
  }, [layout])

  const contentHeight = layout.lineCount * ROW_PITCH + CONTENT_PAD_Y * 2
  /** the packed bars' real extent — wider than the stage on dense traces */
  const contentWidth = Math.max(stage.width, maxRight + PADDING_X)
  const scrollableY = contentHeight + RULER_H > stage.height + 1
  const scrollableX = contentWidth > stage.width + 1
  const scrollable = scrollableY || scrollableX
  // fitContent: ruler + packed lines, plus room for a classic (non-overlay)
  // horizontal scrollbar so it can't clip the bottom line
  const fitHeight = contentHeight + RULER_H + (scrollableX ? 14 : 0)
  // On a LIVE trace the stage only grows. Every silent reload and every
  // one-second live rebuild re-packs the lines, and a pending bar growing
  // into a neighbour, a closed one settling, or the horizontal scrollbar
  // coming and going moved the stage by a line (22px) or a scrollbar
  // (14px) every second or two — with the detail open under its row
  // (follow mode) everything below jumped along (MOT-4621). Once the trace
  // has been quiet for `SETTLE_MS` the exact height applies again, so a
  // finished trace never keeps stale room.
  const live = detail.spans.some((s) => s.status === 'pending')
  const [settled, setSettled] = useState(!live)
  useEffect(() => {
    if (live) {
      setSettled(false)
      return
    }
    const timer = setTimeout(() => setSettled(true), SETTLE_MS)
    return () => clearTimeout(timer)
  }, [live])
  const stickyFitRef = useRef(0)
  const stageHeight = useMemo(() => {
    if (settled) {
      stickyFitRef.current = 0
      return fitHeight
    }
    const grown = Math.max(fitHeight, stickyFitRef.current)
    stickyFitRef.current = grown
    return grown
  }, [settled, fitHeight])

  const hoverEnter = useCallback((id: string) => {
    if (dragRef.current?.active) return
    setHoverId(id)
  }, [])
  const hoverLeave = useCallback((id: string) => {
    setHoverId((h) => (h === id ? null : h))
  }, [])
  const onSpanClickRef = useRef(onSpanClick)
  onSpanClickRef.current = onSpanClick
  const detailRef = useRef(detail)
  detailRef.current = detail
  const clickSpan = useCallback((id: string) => {
    const source = detailRef.current.byId.get(id)
    if (source && onSpanClickRef.current) onSpanClickRef.current(source)
  }, [])

  // Which bars exist in the DOM: those whose line and x-extent intersect
  // the viewport (plus overscan), and always the selected and hovered ones.
  // The DOM stays bounded by the stage, not by the trace.
  const visible = useMemo(() => {
    const out: number[] = []
    if (stage.width <= 0) return out
    const topLine =
      Math.floor((scroll.top - CONTENT_PAD_Y) / ROW_PITCH) - OVERSCAN_LINES
    const bottomLine =
      Math.ceil((scroll.top + stage.height - CONTENT_PAD_Y) / ROW_PITCH) +
      OVERSCAN_LINES
    const xMin = scroll.left - OVERSCAN_PX
    const xMax = scroll.left + stage.width + OVERSCAN_PX
    for (let i = 0; i < layout.placed.length; i++) {
      const p = layout.placed[i]
      const r = rects[i]
      if (
        p.line >= topLine &&
        p.line <= bottomLine &&
        r.left <= xMax &&
        r.left + r.width >= xMin
      ) {
        out.push(i)
      }
    }
    return out
  }, [layout, rects, scroll, stage])
  const visibleRails = useMemo(() => {
    const xMin = scroll.left - OVERSCAN_PX
    const xMax = scroll.left + stage.width + OVERSCAN_PX
    const yMin = scroll.top - OVERSCAN_LINES * ROW_PITCH
    const yMax = scroll.top + stage.height + OVERSCAN_LINES * ROW_PITCH
    return {
      rails: connectors.rails.filter(
        (r) =>
          r.left >= xMin &&
          r.left <= xMax &&
          r.top <= yMax &&
          r.top + r.height >= yMin,
      ),
      stubs: connectors.stubs.filter(
        (st) =>
          st.top >= yMin &&
          st.top <= yMax &&
          st.left <= xMax &&
          st.left + st.width >= xMin,
      ),
    }
  }, [connectors, scroll, stage])
  // A bar plays its entrance once, the first time it is rendered — not
  // again every time a pan scrolls it back into the viewport.
  const enteredRef = useRef(new Set<string>())

  // Everything a bar needs that follows from the layout alone — label fit,
  // spill geometry, colour, aria text — computed once per re-pack, so a
  // hover re-render only maps the visible indices onto ready-made props.
  const barModels = useMemo(() => {
    return layout.placed.map((p, i) => {
      const span = p.span
      const rect = rects[i]
      const color = resolveColor(span)
      const kind = span.kind ?? 'zap'
      const overhead =
        BAR_LABEL_OVERHEAD_PX + (showsKindGlyph(kind) ? BAR_GLYPH_PX : 0)
      const label = span.label ?? span.id
      const fitsInBar = rect.width >= label.length * LABEL_CHAR_PX + overhead
      const spillLeft = rect.left + rect.width + 6
      const nextIdx = nextOnLine[i]
      const spillBound =
        nextIdx != null ? rects[nextIdx].left - 4 : contentWidth - PADDING_X
      const spillWidth = spillBound - spillLeft
      const spillLabel = !fitsInBar && spillWidth >= MIN_SPILL_LABEL_PX
      // Tight bar and no room to spill: keep the truncated in-bar label
      // only while it can show a few glyphs — else the bar stays blank
      // (the hover card has the name).
      const inBarLabel =
        !spillLabel &&
        (fitsInBar ||
          rect.width >= overhead + MIN_INBAR_LABEL_CHARS * LABEL_CHAR_PX)
      return {
        id: span.id,
        kind,
        running: span.status === 'pending',
        ink: inkOnFill(color),
        border: borderOnFill(color),
        label,
        sourceName: detail.byId.get(span.id)?.name,
        ariaLabel: `${label} · ${formatDuration((span.endTime ?? total) - span.startTime)}`,
        top: rowTop(p.line),
        left: rect.left,
        width: rect.width,
        color,
        inBarLabel,
        spillLabel,
        spillLeft,
        spillWidth,
      }
    })
  }, [layout, rects, nextOnLine, contentWidth, detail.byId, total])

  // Drag-to-pan: a press only becomes a pan after DRAG_THRESHOLD_PX of
  // movement on either axis (so bar clicks stay clicks); from then on
  // pointer capture routes the gesture to the viewport and the trailing
  // click is swallowed in the capture phase.
  const onPointerDown = (e: React.PointerEvent) => {
    suppressClickRef.current = false
    if (e.button !== 0 || !viewportRef.current || !scrollable) return
    dragRef.current = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      startScrollLeft: viewportRef.current.scrollLeft,
      startScrollTop: viewportRef.current.scrollTop,
      active: false,
    }
  }
  const onPointerMove = (e: React.PointerEvent) => {
    pointerRef.current = { x: e.clientX, y: e.clientY }
    const drag = dragRef.current
    const viewport = viewportRef.current
    if (!drag || !viewport || e.pointerId !== drag.pointerId) return
    const dx = e.clientX - drag.startX
    const dy = e.clientY - drag.startY
    if (!drag.active) {
      if (Math.max(Math.abs(dx), Math.abs(dy)) < DRAG_THRESHOLD_PX) return
      drag.active = true
      suppressClickRef.current = true
      viewport.setPointerCapture(drag.pointerId)
      setDragging(true)
      setHoverId(null)
    }
    viewport.scrollLeft = drag.startScrollLeft - dx
    viewport.scrollTop = drag.startScrollTop - dy
  }
  const onPointerEnd = (e: React.PointerEvent) => {
    const drag = dragRef.current
    if (!drag || e.pointerId !== drag.pointerId) return
    dragRef.current = null
    if (drag.active) {
      setDragging(false)
      const viewport = viewportRef.current
      if (viewport?.hasPointerCapture(e.pointerId)) {
        viewport.releasePointerCapture(e.pointerId)
      }
    }
  }
  const onClickCapture = (e: React.MouseEvent) => {
    if (!suppressClickRef.current) return
    suppressClickRef.current = false
    e.preventDefault()
    e.stopPropagation()
  }

  const spanById = useMemo(
    () => new Map(detail.spans.map((s) => [s.id, s])),
    [detail.spans],
  )
  const hoveredSpan = hoverId ? (spanById.get(hoverId) ?? null) : null

  const hiddenCount = data.spans.length - visibleData.spans.length

  return (
    <div
      className={cn(
        'flex w-full flex-col font-inter',
        !fitContent && 'h-full min-h-[120px]',
        className,
      )}
    >
      {/* toolbar: the view switcher, then the span filter and the count */}
      {(toolbarStart || spanFilter) && (
        <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
          {toolbarStart}
          <span className="ml-auto flex items-center gap-2">
            {spanFilter && (
              <SpanFilterMenu
                groups={spanGroups}
                workerGroups={workerGroups}
                internalGroups={internalGroups}
                hiddenKeys={spanFilter.hiddenGroups}
                hiddenWorkerKeys={spanFilter.hiddenWorkers}
                shownInternalKeys={spanFilter.shownInternal}
                hiddenSpanCount={hiddenCount}
                onToggle={spanFilter.toggleGroup}
                onToggleWorker={spanFilter.toggleWorker}
                onToggleInternal={spanFilter.toggleInternal}
                onClear={() =>
                  spanFilter.clear(internalGroups.map((g) => g.key))
                }
              />
            )}
            <span className="text-[11px] text-muted-foreground tabular-nums">
              {hiddenCount > 0
                ? `${visibleData.spans.length} of ${data.spans.length} spans`
                : `${data.spans.length} spans`}
            </span>
          </span>
        </div>
      )}

      {/* the legend: which hue is which worker, and a toggle per worker */}
      {legendWorkers.length > 0 && (
        <WorkerLegend
          workers={legendWorkers}
          hidden={spanFilter?.hiddenWorkers}
          onToggle={spanFilter?.toggleWorker}
          className="shrink-0 border-b border-border"
        />
      )}

      {/* fitContent: the stage takes exactly the ruler + packed lines
          (capped) — the explicit height is what the absolute viewport and
          the ResizeObserver measure, so panning only engages past the cap */}
      <div
        ref={stageRef}
        className={cn(
          'relative min-h-0',
          fitContent ? 'min-h-[120px] max-h-[60dvh]' : 'flex-1',
        )}
        style={fitContent ? { height: stageHeight } : undefined}
      >
        <div
          ref={viewportRef}
          key={data.spans[0]?.trace_id ?? 'trace'}
          className={cn(
            'absolute inset-0 select-none overflow-auto',
            dragging ? 'cursor-grabbing' : scrollable && 'cursor-grab',
          )}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerEnd}
          onPointerCancel={onPointerEnd}
          onClickCapture={onClickCapture}
          onScroll={onScroll}
        >
          {/* proportional time ruler — sticky: pinned while panning down,
              gliding with the bars while panning right */}
          <div
            className="sticky top-0 z-[6] bg-card shadow-[inset_0_-1px_0_var(--color-border)]"
            style={{ height: RULER_H, width: contentWidth }}
          >
            {stage.width > 0 &&
              RULER_FRACTIONS.map((f) => (
                <div
                  key={f}
                  /* diagram micro: ruler tick labels */
                  className={cn(
                    'absolute bottom-0.5 font-mono text-[10px] text-muted-foreground tabular-nums whitespace-nowrap',
                    f === 1 ? '-translate-x-full pr-1' : 'pl-1',
                  )}
                  style={{ left: x(total * f) }}
                >
                  {formatDuration(detail.totalDurationMs * f)}
                </div>
              ))}
          </div>

          <div
            className={cn('relative', dragging && 'pointer-events-none')}
            style={{ height: contentHeight, width: contentWidth }}
          >
            {/* time grid — scrolls with the bars it measures */}
            {stage.width > 0 &&
              RULER_FRACTIONS.map((f) => (
                <div
                  key={f}
                  aria-hidden
                  className="absolute inset-y-0 w-px bg-border"
                  style={{ left: x(total * f) }}
                />
              ))}

            {/* the now-line: while the trace is live, an amber hairline
                rides its growing end and glides with the one-second
                re-pack; it fades once the trace settles */}
            {stage.width > 0 && (
              <div
                aria-hidden
                data-trace-playhead
                className={cn(
                  'timeline-glide trace-playhead pointer-events-none absolute inset-y-0 z-[4] w-px transition-opacity duration-[var(--motion-duration-panel)]',
                  !live && 'opacity-0',
                )}
                style={{ left: x(total) }}
              />
            )}

            {stage.width > 0 && (
              <>
                {/* parent → child elbow connectors */}
                <ConnectorLayer
                  rails={visibleRails.rails}
                  stubs={visibleRails.stubs}
                />

                {/* bars: parents above what they started, sequential
                    siblings sharing a line — only the ones in view */}
                {visible.map((i) => {
                  const m = barModels[i]
                  const enter = !enteredRef.current.has(m.id)
                  if (enter) enteredRef.current.add(m.id)
                  return (
                    <Fragment key={m.id}>
                      <TimelineBar
                        id={m.id}
                        label={m.label}
                        sourceName={m.sourceName}
                        ariaLabel={m.ariaLabel}
                        top={m.top}
                        left={m.left}
                        width={m.width}
                        color={m.color}
                        ink={m.ink}
                        border={m.border}
                        kind={m.kind}
                        selected={selectedSpanId === m.id}
                        hovered={hoverId === m.id}
                        inBarLabel={m.inBarLabel}
                        running={m.running}
                        enter={enter}
                        clickable={!!onSpanClick}
                        onEnter={hoverEnter}
                        onLeave={hoverLeave}
                        onClick={clickSpan}
                      />
                      {m.spillLabel && (
                        <span
                          aria-hidden
                          className="timeline-glide pointer-events-none absolute overflow-hidden font-mono text-xs leading-none whitespace-nowrap text-ellipsis text-foreground/80"
                          style={{
                            left: m.spillLeft,
                            top: m.top + BAR_HEIGHT / 2 - 6,
                            maxWidth: m.spillWidth,
                          }}
                        >
                          {m.label}
                        </span>
                      )}
                    </Fragment>
                  )
                })}
              </>
            )}
          </div>
        </div>
      </div>

      {hoveredSpan && (
        <HoverCardLayer
          span={hoveredSpan}
          total={total}
          viewportRef={viewportRef}
          pointerRef={pointerRef}
        />
      )}
    </div>
  )
}
