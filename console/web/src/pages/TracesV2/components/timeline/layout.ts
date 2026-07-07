/**
 * Pure layout logic for the live trace Timeline.
 *
 * Lane model: up to `maxLanes` horizontal "threads". Lane indices are
 * abstract — index 0 is the lane closest above the center axis, 1 the
 * closest below, 2 the second above, 3 the second below, … so sparse
 * traffic hugs the axis (see `laneCenterOffset`).
 *
 * Placement is VISUAL-clearance aware, not just temporal: bars are clamped
 * to MIN_BAR_WIDTH, so two ms-long traces landing a few hundred ms apart
 * occupy overlapping pixels even though they never overlapped in time.
 * A span therefore takes the innermost lane whose previous bar ended at
 * least `clearanceMs` earlier (one min-width bar plus a breath, in time
 * units); only when no lane offers that does it fall back to the
 * most-cleared temporally-free lane (bars may touch, but never share a
 * lane while actually overlapping in time). Net effect: a burst of
 * near-simultaneous instants fans out across the threads instead of
 * piling up on the axis, while sparse traffic still hugs the axis.
 *
 * When every lane is busy, a span overflows: it renders as an icon-only
 * chip riding the busiest lane at its own start position. Chips that land
 * close together fan out into an avatar stack (`computeChipPositions`)
 * with the longest span behind and the shortest on top.
 *
 * Assignment is sticky: once a span is placed (bar lane or chip lane) it
 * never moves, even as older spans are pruned from the input list.
 * `computeAssignments` takes the previous assignment map and only decides
 * placements for unseen ids — feeding its own output back as `prev` is a
 * no-op, which keeps renders (and React StrictMode re-renders) stable.
 * (Stickiness accepts one imperfection: a bar whose end later EXTENDS —
 * live traces settle later — can grow into a lane-mate placed while it
 * looked short. Placement never revisits old decisions by design.)
 */

export type TimelineSpanKind = 'zap' | 'sparkle' | 'flame' | 'lambda'

export interface TimelineSpan {
  id: string
  /** epoch ms */
  startTime: number
  /** epoch ms; null/undefined while the span is still running */
  endTime?: number | null
  /** icon shown at the left of the bar; defaults to 'zap' */
  kind?: TimelineSpanKind
  /** bar color override; defaults to a schematic ink shade (alert on error) */
  color?: string
  status?: 'ok' | 'error' | 'pending' | 'unset'
  label?: string
  /** hover-card subtitle (worker name, trace id, …) */
  meta?: string
}

export interface SpanAssignment {
  spanId: string
  type: 'bar' | 'chip'
  lane: number
}

export interface BarLayout {
  span: TimelineSpan
  lane: number
  live: boolean
}

export interface ChipLayout {
  span: TimelineSpan
  lane: number
}

export interface ChipRender extends ChipLayout {
  /** x in track coordinates (px), after avatar-stack separation */
  x: number
  /** longest span lowest, shortest highest — shortest sits on top */
  zIndex: number
}

export interface TimelineLayout {
  bars: BarLayout[]
  chips: ChipLayout[]
}

export const BAR_HEIGHT = 16
export const LANE_PITCH = 20
export const AXIS_GAP = 4
export const MIN_BAR_WIDTH = 18
/** minimum horizontal separation between chips in a stack (px) */
export const CHIP_STACK_STEP = 10
/** keep spans slightly past the window edge so the mask fade finishes */
const PRUNE_SLACK_MS = 1_500

export function effectiveEnd(span: TimelineSpan, now: number): number {
  return span.endTime ?? now
}

/** open end = still running: occupies its lane indefinitely */
function occupancyEnd(span: TimelineSpan): number {
  return span.endTime ?? Number.POSITIVE_INFINITY
}

/**
 * Signed offset (px) of a lane's vertical center from the axis line.
 * Lanes fill outward from the center: above, below, above, below, …
 */
export function laneCenterOffset(lane: number): number {
  const row = lane >> 1
  const sign = lane % 2 === 0 ? -1 : 1
  return sign * (AXIS_GAP + BAR_HEIGHT / 2 + row * LANE_PITCH)
}

/**
 * Greedy interval lane assignment with sticky results.
 *
 * Spans are processed in start order. A span takes a temporally-free lane,
 * preferring one with real VISUAL clearance (`clearanceMs`, converted from
 * pixels by the caller): min-width bars make near-simultaneous instants
 * overlap on screen even when they never overlapped in time, so those
 * spread onto emptier threads instead of piling up. Among lanes with
 * clearance the least-recently-used wins (fan out, round-robin feel);
 * without clearance the most-cleared free lane wins (least pixel overlap).
 * If every lane is temporally busy the span becomes a chip on the lane
 * whose occupant will stay on screen the longest, i.e. it visually stacks
 * onto the longest-running bar.
 */
export function computeAssignments(
  spans: readonly TimelineSpan[],
  prev: ReadonlyMap<string, SpanAssignment>,
  maxLanes: number,
  clearanceMs = 0,
): Map<string, SpanAssignment> {
  const next = new Map<string, SpanAssignment>()
  const sorted = [...spans].sort(
    (a, b) => a.startTime - b.startTime || (a.id < b.id ? -1 : 1),
  )

  // Rolling per-lane occupancy: when each lane's latest bar ends, and the
  // start of the bar occupying it (for "longest-running" tie-breaks).
  const laneEnd: number[] = Array(maxLanes).fill(Number.NEGATIVE_INFINITY)
  const laneStart: number[] = Array(maxLanes).fill(Number.POSITIVE_INFINITY)

  const placeBar = (span: TimelineSpan, lane: number) => {
    if (lane < 0 || lane >= maxLanes) return
    const end = occupancyEnd(span)
    if (end > laneEnd[lane]) {
      laneEnd[lane] = end
      laneStart[lane] = span.startTime
    }
  }

  for (const span of sorted) {
    const kept = prev.get(span.id)
    if (kept) {
      next.set(span.id, kept)
      if (kept.type === 'bar') placeBar(span, kept.lane)
      continue
    }

    // Pass 1: the INNERMOST lane whose last bar cleared the screen before
    // this span starts — sparse traffic keeps hugging the axis, while a
    // span landing within a min-width bar of a lane's occupant skips to
    // the next thread out (with clearanceMs = 0 this is exactly the old
    // first-free-lane rule).
    let lane = -1
    for (let i = 0; i < maxLanes; i++) {
      if (laneEnd[i] <= span.startTime - clearanceMs) {
        lane = i
        break
      }
    }
    // Pass 2: no lane has visual clearance — take the temporally-free lane
    // with the most room (least pixel overlap; bars may touch).
    if (lane < 0) {
      for (let i = 0; i < maxLanes; i++) {
        if (laneEnd[i] > span.startTime) continue
        if (lane < 0 || laneEnd[i] < laneEnd[lane]) lane = i
      }
    }

    if (lane >= 0) {
      next.set(span.id, { spanId: span.id, type: 'bar', lane })
      placeBar(span, lane)
    } else {
      let best = 0
      for (let i = 1; i < maxLanes; i++) {
        const better =
          laneEnd[i] > laneEnd[best] ||
          (laneEnd[i] === laneEnd[best] && laneStart[i] < laneStart[best])
        if (better) best = i
      }
      next.set(span.id, { spanId: span.id, type: 'chip', lane: best })
    }
  }

  return next
}

/**
 * Resolve assignments into the render structure for the current window,
 * dropping spans that scrolled out. Chips come back sorted by
 * (lane, startTime) ready for `computeChipPositions`.
 */
export function buildVisibleLayout(
  spans: readonly TimelineSpan[],
  assignments: ReadonlyMap<string, SpanAssignment>,
  now: number,
  windowMs: number,
): TimelineLayout {
  const windowStart = now - windowMs - PRUNE_SLACK_MS

  const bars: BarLayout[] = []
  const chips: ChipLayout[] = []
  for (const span of spans) {
    const assignment = assignments.get(span.id)
    if (!assignment || effectiveEnd(span, now) < windowStart) continue
    if (assignment.type === 'bar') {
      bars.push({ span, lane: assignment.lane, live: span.endTime == null })
    } else {
      chips.push({ span, lane: assignment.lane })
    }
  }
  bars.sort((a, b) => a.span.startTime - b.span.startTime)
  chips.sort(
    (a, b) =>
      a.lane - b.lane ||
      a.span.startTime - b.span.startTime ||
      (a.span.id < b.span.id ? -1 : 1),
  )

  return { bars, chips }
}

/**
 * Place chips at their own start-time x, fanned rightward so chips that
 * land close together read as an avatar stack (min CHIP_STACK_STEP apart).
 * z-order is per lane by duration: longest behind, shortest on top. The
 * rightward-only push means already-placed chips never shift when new
 * ones arrive.
 */
export function computeChipPositions(
  chips: readonly ChipLayout[],
  pxPerMs: number,
  epoch: number,
  now: number,
): ChipRender[] {
  const duration = (s: TimelineSpan) => effectiveEnd(s, now) - s.startTime

  const byLane = new Map<number, ChipLayout[]>()
  for (const chip of chips) {
    const list = byLane.get(chip.lane)
    if (list) list.push(chip)
    else byLane.set(chip.lane, [chip])
  }

  const out: ChipRender[] = []
  for (const list of byLane.values()) {
    const zOrder = [...list].sort((a, b) => duration(b.span) - duration(a.span))
    const zIndexById = new Map(zOrder.map((c, i) => [c.span.id, 10 + i]))
    let prevX = Number.NEGATIVE_INFINITY
    for (const chip of list) {
      const raw = (chip.span.startTime - epoch) * pxPerMs
      const x = Math.max(raw, prevX + CHIP_STACK_STEP)
      prevX = x
      out.push({
        ...chip,
        x,
        zIndex: zIndexById.get(chip.span.id) ?? 10,
      })
    }
  }
  return out
}

/**
 * Wall-clock tick boundaries covering the window, plus one boundary past
 * each edge so ticks slide in from off-screen instead of popping.
 */
export function computeTicks(
  now: number,
  windowMs: number,
  tickMs: number,
): number[] {
  const first = Math.floor((now - windowMs) / tickMs) * tickMs
  const last = Math.ceil((now + tickMs) / tickMs) * tickMs
  const ticks: number[] = []
  for (let t = first; t <= last; t += tickMs) ticks.push(t)
  return ticks
}
