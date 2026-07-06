/**
 * Live trace timeline — a right-anchored 60s window driven by a virtual
 * clock that only advances while there is something to show.
 *
 * Motion model: every span bar and tick line is laid out ONCE in absolute
 * time-pixel coordinates inside a "track" element (origin = `timeBase`).
 * One requestAnimationFrame loop slides the tracks — the lanes track and
 * the ruler track, in lockstep — and grows the bars of still-running spans
 * by mutating styles through refs — React never re-renders per frame. A
 * coarse 4Hz state tick handles structure only: pruning expired spans,
 * adding tick lines, re-sorting chip stacks.
 *
 * Vertically the component is a flex column: a slim in-flow time ruler on
 * top (the tick timestamps), then the masked lanes viewport. The ruler is
 * a normal row, not an overlay — timestamps slide horizontally with the
 * track but never sit in front of the bars.
 *
 * The virtual clock (`viewNow`):
 * - while any span is running, it tracks the wall clock (bars grow, the
 *   window scrolls);
 * - once everything completes, it parks just past the last span's end —
 *   the timeline FREEZES with the last bar at the right edge;
 * - when a new span arrives after a gap, it eases up to the wall clock
 *   (the gap whooshes in, then the new span materializes at the edge);
 * - gaps larger than one window (fixtures from a fixed past, late seeds)
 *   jump instantly instead, re-basing the track's coordinate origin so
 *   pixel offsets stay small.
 *
 * Bars are right-edge anchored (right = end-of-span, width extends
 * leftward, clamped to MIN_BAR_WIDTH) so a newborn span materializes as an
 * icon square hugging the "now" edge and grows into a bar while it runs.
 * Wide-enough bars reveal the span name with a LEADING ellipsis.
 *
 * Interactivity: bars and chips are buttons. Hovering shows a SpanHoverCard
 * with the span's details (rendered OUTSIDE the masked viewport — a mask
 * applies to the whole subtree, fixed descendants included); clicking fires
 * `onSpanClick`. The selected span carries the region's single accent moment
 * (2px accent ring); hover stays on a 1px ink ring.
 */

import {
  type CSSProperties,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { cn } from '@/lib/utils'
import { formatDuration } from '../../lib/traceUtils'
import {
  BAR_HEIGHT,
  buildVisibleLayout,
  computeAssignments,
  computeChipPositions,
  computeTicks,
  effectiveEnd,
  laneCenterOffset,
  MIN_BAR_WIDTH,
  type SpanAssignment,
  type TimelineSpan,
} from './layout'
import { SpanHoverCard } from './SpanHoverCard'
import {
  BarLabel,
  CHIP_SIZE,
  iconColorFor,
  KIND_ICONS,
  resolveColor,
  ringFor,
} from './spanVisuals'

export type { TimelineSpan, TimelineSpanKind } from './layout'

export interface TimelineProps {
  spans: TimelineSpan[]
  /** visible window, ms (default 60s) */
  windowMs?: number
  /** tick line cadence, ms (default 15s) */
  tickMs?: number
  /** concurrent lanes before spans collapse into icon stacks (default 4) */
  maxLanes?: number
  /** clicking a bar/chip selects it (trace strip: opens the trace) */
  onSpanClick?: (span: TimelineSpan) => void
  /** span carrying the accent ring */
  selectedSpanId?: string
  className?: string
}

const EDGE_FADE_PX = 48
/** how far past the last span's end the view parks when everything is done */
const FREEZE_MARGIN_MS = 1_500
/** exponential catch-up time constant — a 30s gap whooshes in well under 1s */
const CATCH_UP_TAU_MS = 100
/**
 * Minimum on-screen gap a lane's previous bar should leave before a new
 * span reuses that lane: one min-width bar plus a breath. Converted to ms
 * with the current px-per-ms scale and fed to `computeAssignments`, so
 * near-simultaneous instants spread onto different threads instead of
 * overlapping on the axis.
 */
const LANE_CLEARANCE_PX = MIN_BAR_WIDTH + 6

function formatTick(t: number): string {
  return new Date(t).toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function spanLabel(span: TimelineSpan, now: number): string {
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

/** right-anchored bar geometry: right edge sits at `end`, width grows leftward */
function barRect(
  startTime: number,
  end: number,
  pxPerMs: number,
  timeBase: number,
): { left: number; width: number } {
  const width = Math.max((end - startTime) * pxPerMs, MIN_BAR_WIDTH)
  return { left: (end - timeBase) * pxPerMs - width, width }
}

function laneTop(lane: number): string {
  return `calc(50% + ${laneCenterOffset(lane) - BAR_HEIGHT / 2}px)`
}

interface LiveBarEntry {
  el: HTMLElement
  startTime: number
}

interface HoverState {
  id: string
  x: number
  y: number
}

export function Timeline({
  spans,
  windowMs = 60_000,
  tickMs = 15_000,
  maxLanes = 4,
  onSpanClick,
  selectedSpanId,
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

  // Sticky lane assignments: only unseen span ids get placed, so bars and
  // chips never jump lanes as old spans are pruned from the input. Skipped
  // until the viewport is measured — nothing renders at width 0 anyway, and
  // deferring keeps the first real placement pass clearance-aware.
  const assignmentsRef = useRef<ReadonlyMap<string, SpanAssignment>>(new Map())
  const assignments = useMemo(() => {
    if (pxPerMs <= 0) return assignmentsRef.current
    const next = computeAssignments(
      spans,
      assignmentsRef.current,
      maxLanes,
      LANE_CLEARANCE_PX / pxPerMs,
    )
    assignmentsRef.current = next
    return next
  }, [spans, maxLanes, pxPerMs])

  const layout = useMemo(
    () => buildVisibleLayout(spans, assignments, structureNow, windowMs),
    [spans, assignments, structureNow, windowMs],
  )
  const chipRenders = useMemo(
    () => computeChipPositions(layout.chips, pxPerMs, timeBase, structureNow),
    [layout.chips, pxPerMs, timeBase, structureNow],
  )
  const ticks = useMemo(
    () => computeTicks(structureNow, windowMs, tickMs),
    [structureNow, windowMs, tickMs],
  )

  /**
   * Advance the virtual clock, position the track, and grow live bars.
   * Runs per animation frame; React state is only touched on long jumps
   * (re-base) — never on the steady path.
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
    for (const { el, startTime } of liveBarsRef.current.values()) {
      const barWidth = Math.max(
        (viewNow - startTime) * m.pxPerMs,
        MIN_BAR_WIDTH,
      )
      el.style.width = `${barWidth}px`
      el.style.left = `${(viewNow - m.timeBase) * m.pxPerMs - barWidth}px`
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

  // Coarse structural tick: prune, extend ticks, resort chip stacks. Reads
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
    startTime: number,
    el: HTMLElement | null,
  ) => {
    if (el) liveBarsRef.current.set(id, { el, startTime })
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
      {/* time ruler: an in-flow row on top of the lanes — the timestamps
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

      <div
        ref={viewportRef}
        className="relative min-h-0 flex-1 overflow-hidden"
        style={maskStyle}
      >
        {/* center axis */}
        <div className="absolute inset-x-0 top-1/2 h-px bg-rule" />

        {width > 0 && (
          <div
            ref={trackRef}
            className="absolute inset-0 will-change-transform"
          >
            {/* tick lines every 15s, riding the track */}
            {ticks.map((t) => (
              <div
                key={t}
                className="absolute inset-y-0 w-px bg-rule-2"
                style={{ left: x(t) }}
              />
            ))}

            {/* span bars */}
            {layout.bars.map(({ span, lane, live }) => {
              const color = resolveColor(span)
              const Icon = KIND_ICONS[span.kind ?? 'zap']
              const selected = selectedSpanId === span.id
              const hovered = hover?.id === span.id
              const rect = live
                ? undefined
                : barRect(
                    span.startTime,
                    effectiveEnd(span, structureNow),
                    pxPerMs,
                    timeBase,
                  )
              return (
                <button
                  key={span.id}
                  type="button"
                  ref={
                    live
                      ? (el) => registerLiveBar(span.id, span.startTime, el)
                      : undefined
                  }
                  aria-label={spanLabel(span, structureNow)}
                  onMouseEnter={trackHover(span.id)}
                  onMouseMove={trackHover(span.id)}
                  onMouseLeave={clearHover(span.id)}
                  onClick={onSpanClick ? () => onSpanClick(span) : undefined}
                  className={cn(
                    'timeline-enter absolute flex items-center gap-1 overflow-hidden rounded-[4px] px-[3px]',
                    'focus-visible:outline-1 focus-visible:outline-accent',
                    !live && 'transition-[width,left] duration-200 ease-out',
                    onSpanClick ? 'cursor-pointer' : 'cursor-default',
                  )}
                  style={{
                    top: laneTop(lane),
                    height: BAR_HEIGHT,
                    backgroundColor: color,
                    boxShadow: ringFor(selected, hovered),
                    zIndex: selected || hovered ? 5 : undefined,
                    ...(rect ? { left: rect.left, width: rect.width } : {}),
                  }}
                >
                  <Icon
                    className="h-3 w-3 shrink-0"
                    strokeWidth={2.5}
                    style={{ color: iconColorFor(color) }}
                  />
                  <BarLabel
                    text={span.label ?? span.id}
                    color={iconColorFor(color)}
                  />
                </button>
              )
            })}

            {/* overflow spans: icon-only chips riding the busiest lane,
                fanned into avatar stacks — longest behind, shortest on top */}
            {chipRenders.map(({ span, lane, x: chipX, zIndex }) => (
              <TimelineChip
                key={span.id}
                chip={span}
                left={chipX}
                lane={lane}
                zIndex={zIndex}
                now={structureNow}
                selected={selectedSpanId === span.id}
                hovered={hover?.id === span.id}
                onMouseEnter={trackHover(span.id)}
                onMouseMove={trackHover(span.id)}
                onMouseLeave={clearHover(span.id)}
                onClick={onSpanClick ? () => onSpanClick(span) : undefined}
              />
            ))}
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

function TimelineChip({
  chip,
  left,
  lane,
  zIndex,
  now,
  selected,
  hovered,
  onMouseEnter,
  onMouseMove,
  onMouseLeave,
  onClick,
}: {
  chip: TimelineSpan
  left: number
  lane: number
  zIndex: number
  now: number
  selected: boolean
  hovered: boolean
  onMouseEnter: (e: React.MouseEvent) => void
  onMouseMove: (e: React.MouseEvent) => void
  onMouseLeave: () => void
  onClick?: () => void
}) {
  const color = resolveColor(chip)
  const Icon = KIND_ICONS[chip.kind ?? 'zap']
  return (
    <button
      type="button"
      aria-label={spanLabel(chip, now)}
      onMouseEnter={onMouseEnter}
      onMouseMove={onMouseMove}
      onMouseLeave={onMouseLeave}
      onClick={onClick}
      className={cn(
        'timeline-enter absolute flex items-center justify-center rounded-full transition-[left] duration-300 ease-out',
        'focus-visible:outline-1 focus-visible:outline-accent',
        onClick ? 'cursor-pointer' : 'cursor-default',
      )}
      style={{
        left,
        top: `calc(50% + ${laneCenterOffset(lane) - CHIP_SIZE / 2}px)`,
        width: CHIP_SIZE,
        height: CHIP_SIZE,
        backgroundColor: color,
        zIndex: selected || hovered ? 40 : zIndex,
        boxShadow: ringFor(selected, hovered, '0 0 0 1.5px var(--color-bg)'),
      }}
    >
      <Icon
        className="h-2.5 w-2.5"
        strokeWidth={2.5}
        style={{ color: iconColorFor(color) }}
      />
    </button>
  )
}
