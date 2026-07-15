/**
 * The TracesV2 masthead: the live Timeline wearing the page-header chrome.
 * Replaces the old `$ traces` + h1 header block — the strip IS the title
 * row now. One 3px line per SPAN across all traces, arranged
 * HIERARCHICALLY like the detail view's `TraceTimeline` and sliding
 * through a 60s window; hovering a bar shows the span's details, clicking
 * it opens its owning trace.
 *
 * Bars come straight from the all-spans feed (`useAllSpans`: one seed read
 * + the engine's `iii:devtools:all-spans` stream): a pending span renders
 * as a LIVE bar growing along the now-edge and settles when its close frame
 * arrives, so liveness is span-accurate — no trace-level correction needed.
 * Engine routing wrappers are skipped (see `storedSpansToTimelineSpans`).
 *
 * The visualization: every parent sits on a line above the spans it
 * triggered (elbow connectors tie them together, child bars are inset
 * from their parent's left edge), sequential subtrees share a line
 * flame-graph style and only genuinely concurrent subtrees stack further
 * down. Lines carry no icons or in-bar labels — the SpanHoverCard does the
 * talking. There is no lane cap: lines stack as tall as they need and the
 * viewport scrolls vertically when they outgrow the strip. No selection
 * ring either — the strip doesn't mark the open trace (hover keeps a 1px
 * ink ring so the pointer can find the 3px target).
 *
 * Motion: the sliding window rides a virtual clock (parks when idle,
 * whooshes on catch-up, re-bases on long jumps); one rAF loop slides the
 * ruler and lines tracks in lockstep and grows live bars along the
 * now-edge (the right edge rides `viewNow`; the hierarchy keeps the LEFT
 * edge fixed, so growth is width-only).
 *
 * Which bars are VISIBLE is the same hidden span-group / worker selection
 * the detail views use (`lib/spanFilters.ts`, one shared `spanFilter`
 * instance per page) — the funnel menu in the header row lists the strip's
 * current function families and workers, and hiding one here hides it in
 * the waterfall too. On an all-spans strip that menu is the volume control:
 * hide the bookkeeping families and the strip reads as real work.
 *
 * Chrome sits in a header row above the visualization (solid `bg-bg`,
 * bounded by 1px rules): the eyebrow + paused badge on the left, the
 * follow-turn toggle and span-filter funnel on the right. The follow
 * toggle (shown when the page wires `onToggleFollowTurns`) auto-opens the
 * trace of the active chat's live turn — user interactions only, never
 * sub-agents (see `hooks/useFollowLiveTurn.ts`).
 *
 * Hierarchy edges come from `TimelineSpan.parentId` (nearest non-routing
 * ancestor, resolved by `storedSpansToTimelineSpans`). A span whose
 * parent isn't in the pruned window renders as a root. Placement is NOT
 * sticky: the layout re-packs from scratch whenever the span set changes.
 * In practice that's calm — a subtree placed below a live one keeps
 * time-overlapping it after it settles (the child started while the
 * parent ran), so settling rarely reshuffles lines; arrivals of
 * concurrent children grow their subtree downward exactly like the
 * detail view would.
 */

import { Crosshair, Pause } from 'lucide-react'
import {
  type CSSProperties,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Badge } from '@/components/ui/Badge'
import { cn } from '@/lib/utils'
import type { StoredSpan } from '../../api/traces'
import type { SpanFilterControls } from '../../lib/spanFilters'
import { storedSpansToTimelineSpans } from '../../lib/timelineSpans'
import { formatDuration } from '../../lib/traceUtils'
import { IconToggleButton } from '../IconToggleButton'
import { computeTicks, effectiveEnd, type TimelineSpan } from './layout'
import { SpanFilterMenu } from './SpanFilterMenu'
import { SpanHoverCard } from './SpanHoverCard'
import {
  deriveSpanGroups,
  isSpanBarHidden,
  reparentThroughHidden,
} from './spanVisibility'
import { resolveColor } from './spanVisuals'

export interface TimelineStripProps {
  /** every span the strip may show (`useAllSpans` seed + live stream) */
  spans: readonly StoredSpan[]
  isPaused: boolean
  /** clicking a bar opens its owning trace's detail */
  onTraceClick?: (traceId: string) => void
  /** the page's shared span-filter selection (`useSpanFilterSelection`);
   *  the funnel menu is hidden when omitted and every bar shows */
  spanFilter?: SpanFilterControls
  /** auto-open the active chat's live turn trace (persisted per browser) */
  followTurns?: boolean
  /** toggle handler; the follow button is hidden when omitted */
  onToggleFollowTurns?: () => void
  /** visible window, ms (default 60s) */
  windowMs?: number
  className?: string
}

const PRUNE_SLACK_MS = 15_000
/** cadence of the liveness re-evaluation while any bar is live */
const LIVENESS_TICK_MS = 500

/* ------------------------------------------------------------------ */
/* hierarchical layout (TraceTimeline's model on the live span feed)    */
/* ------------------------------------------------------------------ */

/** the experiment's line geometry: 3px bars on an 8px rhythm */
const BAR_HEIGHT = 3
const ROW_PITCH = 8
const CONTENT_PAD_Y = 4
/** minimum left inset of a child bar from its parent's — the hierarchy padding */
const DEPTH_INDENT = 10
/** breathing room between sequential bars sharing a line */
const SIBLING_GAP = 2
/** the connector rail drops from just inside the parent bar's left edge */
const RAIL_INSET = 2
/** no icon square to preserve — just enough pixels to hover */
const MIN_BAR_WIDTH = 6

const EDGE_FADE_PX = 48
/** how far past the last span's end the view parks when everything is done */
const FREEZE_MARGIN_MS = 1_500
/** exponential catch-up time constant — a 30s gap whooshes in well under 1s */
const CATCH_UP_TAU_MS = 100

interface PlacedSpan {
  span: TimelineSpan
  /** vertical line index (0 = top) */
  line: number
  /** index into the placed array; emission is pre-order so parent < child */
  parentIndex: number | null
}

interface HierarchyLayout {
  placed: PlacedSpan[]
  lineCount: number
}

/** A subtree's footprint: the lines it spans and its full time extent. */
interface SubtreeInfo {
  height: number
  minStart: number
  maxEnd: number
  /** line offset relative to the parent's line, assigned while packing */
  offset: number
}

function rowTop(line: number): number {
  return CONTENT_PAD_Y + line * ROW_PITCH
}

/** open end = still running: occupies its lines until it settles */
function occupancyEnd(span: TimelineSpan): number {
  return span.endTime ?? Number.POSITIVE_INFINITY
}

/**
 * Greedy first-fit packing of sibling subtrees, flame-graph style — the
 * same rule as `TraceTimeline.tsx`: each subtree is a solid rectangle
 * (time extent × line span); siblings are placed in start order at the
 * smallest offset ≥ `firstOffset` where they collide with no earlier
 * sibling, so sequential subtrees share lines and only concurrent ones
 * stack. Mutates each child's `info.offset`; returns lines consumed.
 */
function packSubtrees(
  children: readonly TimelineSpan[],
  info: ReadonlyMap<string, SubtreeInfo>,
  firstOffset: number,
): number {
  const placed: SubtreeInfo[] = []
  let deepest = firstOffset
  for (const child of children) {
    const rect = info.get(child.id)
    if (!rect) continue
    let offset = firstOffset
    let moved = true
    while (moved) {
      moved = false
      for (const other of placed) {
        const timeOverlap =
          rect.minStart < other.maxEnd && other.minStart < rect.maxEnd
        const lineOverlap =
          offset < other.offset + other.height &&
          other.offset < offset + rect.height
        if (timeOverlap && lineOverlap) {
          offset = other.offset + other.height
          moved = true
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
 * Resolve the live span set into positioned lines — `TraceTimeline`'s
 * two-pass layout (iterative post-order footprints, then pre-order line
 * emission with parents before children) with two live-feed adaptations:
 * roots are spans whose `parentId` is absent from the CURRENT window, and
 * a still-running span's extent is open-ended so nothing packs onto its
 * lines until it settles. Spans stranded by malformed parent chains are
 * appended on their own lines at the bottom.
 */
function buildHierarchyLayout(spans: readonly TimelineSpan[]): HierarchyLayout {
  const byId = new Map(spans.map((s) => [s.id, s]))
  const childrenOf = new Map<string, TimelineSpan[]>()
  const roots: TimelineSpan[] = []
  for (const s of spans) {
    if (s.parentId && byId.has(s.parentId) && s.parentId !== s.id) {
      const siblings = childrenOf.get(s.parentId)
      if (siblings) siblings.push(s)
      else childrenOf.set(s.parentId, [s])
    } else {
      roots.push(s)
    }
  }

  const byStart = (a: TimelineSpan, b: TimelineSpan) =>
    a.startTime - b.startTime || (a.id < b.id ? -1 : 1)
  roots.sort(byStart)
  for (const list of childrenOf.values()) list.sort(byStart)

  // Post-order: subtree footprints. `seen` is marked on push so a
  // malformed cycle can never re-enter the stack.
  const info = new Map<string, SubtreeInfo>()
  const seen = new Set<string>()
  const postStack: Array<{ node: TimelineSpan; expanded: boolean }> = []
  for (let i = roots.length - 1; i >= 0; i--) {
    seen.add(roots[i].id)
    postStack.push({ node: roots[i], expanded: false })
  }
  while (postStack.length > 0) {
    const frame = postStack[postStack.length - 1]
    const kids = childrenOf.get(frame.node.id)
    if (!frame.expanded) {
      frame.expanded = true
      if (kids) {
        for (let i = kids.length - 1; i >= 0; i--) {
          if (seen.has(kids[i].id)) continue
          seen.add(kids[i].id)
          postStack.push({ node: kids[i], expanded: false })
        }
      }
      continue
    }
    postStack.pop()
    const span = frame.node
    let minStart = span.startTime
    let maxEnd = occupancyEnd(span)
    if (kids) {
      for (const kid of kids) {
        const ki = info.get(kid.id)
        if (!ki) continue
        if (ki.minStart < minStart) minStart = ki.minStart
        if (ki.maxEnd > maxEnd) maxEnd = ki.maxEnd
      }
    }
    const height = kids ? Math.max(1, packSubtrees(kids, info, 1)) : 1
    info.set(span.id, { height, minStart, maxEnd, offset: 0 })
  }

  let lineCount = packSubtrees(roots, info, 0)

  // Pre-order: absolute lines, parents emitted before their children.
  const placed: PlacedSpan[] = []
  const preStack: Array<{
    node: TimelineSpan
    line: number
    parentIndex: number | null
  }> = []
  for (let i = roots.length - 1; i >= 0; i--) {
    const ri = info.get(roots[i].id)
    if (!ri) continue
    preStack.push({ node: roots[i], line: ri.offset, parentIndex: null })
  }
  while (preStack.length > 0) {
    const frame = preStack.pop() as (typeof preStack)[number]
    const index = placed.length
    placed.push({
      span: frame.node,
      line: frame.line,
      parentIndex: frame.parentIndex,
    })
    const kids = childrenOf.get(frame.node.id)
    if (!kids) continue
    for (let i = kids.length - 1; i >= 0; i--) {
      const ki = info.get(kids[i].id)
      if (!ki) continue
      preStack.push({
        node: kids[i],
        line: frame.line + ki.offset,
        parentIndex: index,
      })
    }
  }

  if (placed.length < spans.length) {
    const emitted = new Set(placed.map((p) => p.span.id))
    for (const s of spans) {
      if (emitted.has(s.id)) continue
      placed.push({ span: s, line: lineCount++, parentIndex: null })
    }
  }
  return { placed, lineCount }
}

/* ------------------------------------------------------------------ */
/* the live hierarchical visualization                                  */
/* ------------------------------------------------------------------ */

interface TimelineProps {
  spans: readonly TimelineSpan[]
  /** visible window, ms (default 60s) */
  windowMs?: number
  /** tick line cadence, ms (default 15s) */
  tickMs?: number
  onSpanClick?: (span: TimelineSpan) => void
  className?: string
}

interface BarRect {
  left: number
  width: number
}

interface LiveBarEntry {
  el: HTMLElement
  /** track-coordinate left edge — fixed by the hierarchy; only width grows */
  left: number
}

interface HoverState {
  id: string
  x: number
  y: number
}

function formatTick(t: number): string {
  return new Date(t).toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function spanAriaLabel(span: TimelineSpan, now: number): string {
  const duration = formatDuration(effectiveEnd(span, now) - span.startTime)
  const state = span.endTime == null ? ' · running' : ''
  return `${span.label ?? span.id} · ${duration}${state}`
}

/**
 * Where the virtual clock wants to be: the wall clock while anything is
 * running, just past the latest end once everything settled, and parked in
 * place when there are no spans at all.
 */
function desiredViewNow(
  spans: readonly TimelineSpan[],
  wallNow: number,
  current: number,
): number {
  let latestEnd = Number.NEGATIVE_INFINITY
  for (const s of spans) {
    if (s.endTime == null) return wallNow
    if (s.endTime > latestEnd) latestEnd = s.endTime
  }
  if (latestEnd === Number.NEGATIVE_INFINITY) return current
  return Math.min(wallNow, latestEnd + FREEZE_MARGIN_MS)
}

function Timeline({
  spans,
  windowMs = 60_000,
  tickMs = 15_000,
  onSpanClick,
  className,
}: TimelineProps) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const trackRef = useRef<HTMLDivElement>(null)
  const rulerTrackRef = useRef<HTMLDivElement>(null)
  const liveBarsRef = useRef(new Map<string, LiveBarEntry>())
  const [width, setWidth] = useState(0)
  // Origin of the track's time-pixel coordinate space. Re-based when the
  // virtual clock makes a long jump so pixel offsets never grow huge.
  const [timeBase, setTimeBase] = useState(() => Date.now())
  const [structureNow, setStructureNow] = useState(timeBase)
  const [hover, setHover] = useState<HoverState | null>(null)

  const pxPerMs = width > 0 ? width / windowMs : 0

  // The virtual clock, advanced by the rAF loop.
  const clockRef = useRef({ viewNow: timeBase, lastWall: 0 })

  // Snapshot render-derived values for the rAF loop (which has no deps).
  const motionRef = useRef({ width: 0, pxPerMs: 0, timeBase, windowMs })
  motionRef.current = { width, pxPerMs, timeBase, windowMs }
  const spansRef = useRef<readonly TimelineSpan[]>(spans)
  spansRef.current = spans

  // Line placement is a pure function of the span set (open ends pack as
  // unbounded), so it only recomputes when the feed changes — never on the
  // 4Hz structural tick.
  const layout = useMemo(() => buildHierarchyLayout(spans), [spans])

  // Bar geometry in emission order, track coordinates. A child's left edge
  // is time-positioned but never closer than DEPTH_INDENT to its parent's;
  // bars sharing a line never overlap the previous bar (MIN_BAR_WIDTH can
  // inflate tiny spans). Live widths refresh at 4Hz here and per-frame in
  // `applyMotion`.
  const rects = useMemo(() => {
    const out: BarRect[] = []
    const lineRight = new Map<number, number>()
    for (const p of layout.placed) {
      let left = (p.span.startTime - timeBase) * pxPerMs
      if (p.parentIndex != null) {
        left = Math.max(left, out[p.parentIndex].left + DEPTH_INDENT)
      }
      const prevRight = lineRight.get(p.line)
      if (prevRight != null) left = Math.max(left, prevRight + SIBLING_GAP)
      const right = (effectiveEnd(p.span, structureNow) - timeBase) * pxPerMs
      const width = Math.max(right - left, MIN_BAR_WIDTH)
      out.push({ left, width })
      lineRight.set(p.line, left + width)
    }
    return out
  }, [layout, pxPerMs, timeBase, structureNow])

  // Elbow connectors: one rail dropping from each parent's left edge down
  // to its lowest child line, and one horizontal stub reaching the FIRST
  // child on each line — later bars on a shared line read as the
  // sequential continuation of that elbow.
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

  const ticks = useMemo(
    () => computeTicks(structureNow, windowMs, tickMs),
    [structureNow, windowMs, tickMs],
  )

  const contentHeight = layout.lineCount * ROW_PITCH + CONTENT_PAD_Y * 2

  /**
   * Advance the virtual clock, position the tracks, and grow live bars.
   * Runs per animation frame; React state is only touched on long jumps
   * (re-base) — never on the steady path. Growth is WIDTH-only: the
   * hierarchy owns the left edge, the right edge rides `viewNow`.
   */
  const applyMotion = useCallback(() => {
    const m = motionRef.current
    const track = trackRef.current
    if (!track || m.width <= 0) return

    const wallNow = Date.now()
    const clock = clockRef.current
    const dt =
      clock.lastWall === 0
        ? 16
        : Math.min(1_000, Math.max(0, wallNow - clock.lastWall))
    clock.lastWall = wallNow

    const desired = desiredViewNow(spansRef.current, wallNow, clock.viewNow)
    const gap = desired - clock.viewNow
    if (Math.abs(gap) > m.windowMs) {
      // Far target (past history seeding in, or a huge idle gap): jump and
      // re-base the coordinate origin instead of whooshing across it.
      clock.viewNow = desired
      setTimeBase(desired)
      setStructureNow(Math.round(desired))
    } else if (gap > 0) {
      // Frame-rate-independent exponential ease toward the target.
      const k = 1 - Math.exp(-dt / CATCH_UP_TAU_MS)
      const next = clock.viewNow + gap * k
      clock.viewNow = desired - next < 1 ? desired : next
    }
    // gap in [-windowMs, 0]: parked — the view never drifts backward.

    const viewNow = clock.viewNow
    const shift = `translate3d(${m.width - (viewNow - m.timeBase) * m.pxPerMs}px, 0, 0)`
    track.style.transform = shift
    const ruler = rulerTrackRef.current
    if (ruler) ruler.style.transform = shift
    const nowEdge = (viewNow - m.timeBase) * m.pxPerMs
    for (const { el, left } of liveBarsRef.current.values()) {
      el.style.width = `${Math.max(nowEdge - left, MIN_BAR_WIDTH)}px`
    }
  }, [])

  // Measure the viewport (px-per-ms scale) before first paint.
  useLayoutEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const observer = new ResizeObserver((entries) => {
      setWidth(entries[0].contentRect.width)
    })
    observer.observe(el)
    setWidth(el.clientWidth)
    return () => observer.disconnect()
  }, [])

  // Re-position after every commit so newly mounted bars never flash at x=0.
  useLayoutEffect(applyMotion)

  // The continuous loop. Falls back to a 2Hz step under reduced motion.
  useEffect(() => {
    const reduced = window.matchMedia(
      '(prefers-reduced-motion: reduce)',
    ).matches
    if (reduced) {
      const interval = setInterval(applyMotion, 500)
      return () => clearInterval(interval)
    }
    let raf = requestAnimationFrame(function loop() {
      applyMotion()
      raf = requestAnimationFrame(loop)
    })
    return () => cancelAnimationFrame(raf)
  }, [applyMotion])

  // Coarse structural tick: prune, extend ticks, refresh live widths. Reads
  // the virtual clock, so structure freezes with the track when parked
  // (identical state values bail out of re-rendering).
  useEffect(() => {
    const interval = setInterval(() => {
      setStructureNow(Math.round(clockRef.current.viewNow))
    }, 250)
    return () => clearInterval(interval)
  }, [])

  const registerLiveBar = (
    id: string,
    left: number,
    el: HTMLElement | null,
  ) => {
    if (el) liveBarsRef.current.set(id, { el, left })
    else liveBarsRef.current.delete(id)
  }

  const trackHover = (id: string) => (e: React.MouseEvent) => {
    setHover({ id, x: e.clientX, y: e.clientY })
  }
  const clearHover = (id: string) => () => {
    setHover((h) => (h?.id === id ? null : h))
  }

  // Resolve the hovered span from the live list — if it was pruned the card
  // simply disappears; if its status/end updated the card reflects it.
  const hoveredSpan = hover
    ? (spans.find((s) => s.id === hover.id) ?? null)
    : null

  const x = (t: number) => (t - timeBase) * pxPerMs

  const maskStyle: CSSProperties = {
    maskImage: `linear-gradient(to right, transparent 0px, black ${EDGE_FADE_PX}px)`,
    WebkitMaskImage: `linear-gradient(to right, transparent 0px, black ${EDGE_FADE_PX}px)`,
  }

  return (
    <div className={cn('flex h-full w-full flex-col', className)}>
      {/* time ruler: an in-flow row on top of the lines — the timestamps
          slide with the track but never overlap the bars */}
      <div className="relative h-4 shrink-0 overflow-hidden" style={maskStyle}>
        {width > 0 && (
          <div
            ref={rulerTrackRef}
            className="absolute inset-0 will-change-transform"
          >
            {ticks.map((t) => (
              <div
                key={t}
                className="absolute top-1/2 -translate-y-1/2 font-mono text-[10px] text-ink-ghost tabular-nums whitespace-nowrap"
                style={{ left: x(t) + 4 }}
              >
                {formatTick(t)}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* the lines viewport: slides horizontally with the track, scrolls
          vertically when the hierarchy outgrows the strip */}
      <div
        ref={viewportRef}
        className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden"
        style={maskStyle}
      >
        {width > 0 && (
          <div
            ref={trackRef}
            className="relative will-change-transform"
            style={{ height: contentHeight, minHeight: '100%' }}
          >
            {/* tick lines every 15s, riding the track */}
            {ticks.map((t) => (
              <div
                key={t}
                className="absolute inset-y-0 w-px bg-rule-2"
                style={{ left: x(t) }}
              />
            ))}

            {/* parent → child elbow connectors */}
            {connectors.rails.map((rail) => (
              <div
                key={rail.id}
                aria-hidden
                className="absolute w-px bg-rule"
                style={{
                  left: rail.left,
                  top: rail.top,
                  height: rail.height,
                }}
              />
            ))}
            {connectors.stubs.map((stub) => (
              <div
                key={stub.id}
                aria-hidden
                className="absolute h-px bg-rule"
                style={{
                  left: stub.left,
                  top: stub.top,
                  width: stub.width,
                }}
              />
            ))}

            {/* the 3px lines: parents above what they triggered */}
            {layout.placed.map((p, i) => {
              const span = p.span
              const rect = rects[i]
              const live = span.endTime == null
              const hovered = hover?.id === span.id
              return (
                <button
                  key={span.id}
                  type="button"
                  ref={
                    live
                      ? (el) => registerLiveBar(span.id, rect.left, el)
                      : undefined
                  }
                  aria-label={spanAriaLabel(span, structureNow)}
                  onMouseEnter={trackHover(span.id)}
                  onMouseMove={trackHover(span.id)}
                  onMouseLeave={clearHover(span.id)}
                  onClick={onSpanClick ? () => onSpanClick(span) : undefined}
                  className={cn(
                    'timeline-enter absolute rounded-[1.5px]',
                    'focus-visible:outline-1 focus-visible:outline-accent',
                    onSpanClick ? 'cursor-pointer' : 'cursor-default',
                  )}
                  style={{
                    top: rowTop(p.line),
                    height: BAR_HEIGHT,
                    left: rect.left,
                    width: rect.width,
                    backgroundColor: resolveColor(span),
                    boxShadow: hovered
                      ? '0 0 0 1px var(--color-ink)'
                      : undefined,
                    zIndex: hovered ? 5 : undefined,
                  }}
                />
              )
            })}
          </div>
        )}
      </div>

      {hoveredSpan && hover && (
        <SpanHoverCard
          span={hoveredSpan}
          now={structureNow}
          x={hover.x}
          y={hover.y}
        />
      )}
    </div>
  )
}

/* ------------------------------------------------------------------ */
/* the strip: header chrome around the hierarchical visualization       */
/* ------------------------------------------------------------------ */

export function TimelineStrip({
  spans: storedSpans,
  isPaused,
  onTraceClick,
  spanFilter,
  followTurns,
  onToggleFollowTurns,
  windowMs = 60_000,
  className,
}: TimelineStripProps) {
  // Prune-window evaluation instant. Only re-ticked while something is live —
  // an idle strip renders exactly as often as its data changes.
  const [now, setNow] = useState(() => Date.now())

  const allSpans = useMemo(() => {
    const all = storedSpansToTimelineSpans(storedSpans)
    // Keep one window behind the NEWEST effective end so layout stays cheap
    // as history accumulates. The cutoff keys off the data, not the wall
    // clock: when the engine is idle the view parks at the last span's end,
    // and a wall-clock cutoff would filter that history out from under the
    // frozen view. Live spans count as `now`.
    let newestEnd = Number.NEGATIVE_INFINITY
    for (const s of all) {
      const end = s.endTime ?? Math.max(now, s.startTime)
      if (end > newestEnd) newestEnd = end
    }
    if (newestEnd === Number.NEGATIVE_INFINITY) return all
    const cutoff = newestEnd - windowMs - PRUNE_SLACK_MS
    return all.filter((s) => (s.endTime ?? newestEnd) >= cutoff)
  }, [storedSpans, now, windowMs])

  // The funnel menu lists the strip's CURRENT function families / workers
  // (hidden entries included, so they can be un-hidden), sharing the hidden
  // sets with the detail views. Internal bars (`iii.tag.hidden` call-site
  // tag) get their own menu section instead of counting toward the normal
  // spans/workers entries.
  const groups = useMemo(
    () =>
      deriveSpanGroups(allSpans, (s: TimelineSpan) =>
        s.internalKey ? null : s.groupKey,
      ),
    [allSpans],
  )
  const workerGroups = useMemo(
    () =>
      deriveSpanGroups(allSpans, (s: TimelineSpan) =>
        s.internalKey ? null : s.workerKey,
      ),
    [allSpans],
  )
  const internalGroups = useMemo(
    () => deriveSpanGroups(allSpans, (s: TimelineSpan) => s.internalKey),
    [allSpans],
  )

  // Hiding removes only the matched bars; their children re-attach to the
  // nearest kept ancestor so the hierarchy stays connected.
  const spans = useMemo(() => {
    if (!spanFilter) return allSpans
    return reparentThroughHidden(
      allSpans,
      (s: TimelineSpan) => !isSpanBarHidden(s, spanFilter),
    )
  }, [allSpans, spanFilter])

  // While any bar is live, tick the evaluation clock so the prune window
  // keeps advancing with the now-edge; once everything settles the strip
  // stops ticking and the view parks.
  const anyLive = useMemo(() => spans.some((s) => s.endTime == null), [spans])
  useEffect(() => {
    if (!anyLive) return
    const id = setInterval(() => setNow(Date.now()), LIVENESS_TICK_MS)
    return () => clearInterval(id)
  }, [anyLive])

  return (
    <div
      className={cn(
        'flex h-36 flex-shrink-0 flex-col border-b border-rule overflow-hidden',
        className,
      )}
    >
      <div className="flex shrink-0 items-center justify-between border-b border-rule bg-bg">
        <div className="flex items-center gap-3 px-3 py-2">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
            <span className="text-accent">$</span>
            <span className="text-ink ml-2">traces</span>
          </div>
          {isPaused ? (
            <Badge variant="warn">
              <Pause className="w-3 h-3" />
              paused
            </Badge>
          ) : (
            <span className="flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
              <span
                aria-hidden
                className="inline-block size-1.5 rounded-full bg-accent pulse-dot"
              />
              live
            </span>
          )}
        </div>

        {(spanFilter || onToggleFollowTurns) && (
          <div className="flex items-center gap-1 px-2 py-1">
            {onToggleFollowTurns && (
              <IconToggleButton
                active={!!followTurns}
                onClick={onToggleFollowTurns}
                label={
                  followTurns
                    ? 'following your turns — traces auto-open as you chat'
                    : 'follow your turns: auto-open the trace when you send a message'
                }
              >
                <Crosshair className="w-3.5 h-3.5" />
              </IconToggleButton>
            )}
            {spanFilter && (
              <SpanFilterMenu
                groups={groups}
                workerGroups={workerGroups}
                internalGroups={internalGroups}
                hiddenKeys={spanFilter.hiddenGroups}
                hiddenWorkerKeys={spanFilter.hiddenWorkers}
                shownInternalKeys={spanFilter.shownInternal}
                hiddenSpanCount={allSpans.length - spans.length}
                onToggle={spanFilter.toggleGroup}
                onToggleWorker={spanFilter.toggleWorker}
                onToggleInternal={spanFilter.toggleInternal}
                onClear={() =>
                  spanFilter.clear(internalGroups.map((g) => g.key))
                }
              />
            )}
          </div>
        )}
      </div>

      <Timeline
        className="min-h-0 flex-1"
        spans={spans}
        windowMs={windowMs}
        onSpanClick={
          onTraceClick
            ? (span) => onTraceClick(span.traceId ?? span.id)
            : undefined
        }
      />
    </div>
  )
}
