import { describe, expect, it } from 'vitest'
import type { TraceListItem } from '../hooks/useTraceData'
import {
  isTraceLive,
  TRACE_ACTIVITY_ECHO_MS,
  TRACE_ACTIVITY_IDLE_MS,
  traceListToTimelineSpans,
  waterfallToTimelineSpans,
} from './timelineSpans'
import type { VisualizationSpan, WaterfallData } from './traceTransform'

/** A closed root row: the trace started at T and its root ended 40ms later. */
function row(overrides: Partial<TraceListItem> = {}): TraceListItem {
  return {
    traceId: 't-1',
    rootOperation: 'publish chat.message',
    topic: 'chat.message',
    status: 'ok',
    startTime: 100_000,
    endTime: 100_040,
    duration: 40,
    spanCount: 1,
    workers: ['engine'],
    ...overrides,
  }
}

function liveness(entries: Record<string, number>, now: number) {
  return { activity: new Map(Object.entries(entries)), now }
}

describe('traceListToTimelineSpans', () => {
  it('maps a closed row to a settled bar (no liveness given)', () => {
    const [span] = traceListToTimelineSpans([row()])
    expect(span).toMatchObject({
      id: 't-1',
      startTime: 100_000,
      endTime: 100_040,
      status: 'ok',
    })
  })

  it('keeps a pending row live regardless of activity', () => {
    const [span] = traceListToTimelineSpans([
      row({ status: 'pending', endTime: undefined }),
    ])
    expect(span.endTime).toBeNull()
    expect(span.status).toBe('pending')
  })

  it('ignores the row’s own close echoing back through the trigger', () => {
    // Activity within the echo dead-band of the root's end is just the
    // root's own arrival — the trace must NOT go live because of it.
    const echo = 100_040 + TRACE_ACTIVITY_ECHO_MS - 1
    const [span] = traceListToTimelineSpans(
      [row()],
      liveness({ 't-1': echo }, echo + 100),
    )
    expect(span.endTime).toBe(100_040)
    expect(span.status).toBe('ok')
  })

  it('marks a trace live while span-close activity is fresh', () => {
    const activityAt = 100_040 + 5_000 // children still closing 5s past root
    const now = activityAt + TRACE_ACTIVITY_IDLE_MS - 500
    const [span] = traceListToTimelineSpans(
      [row()],
      liveness({ 't-1': activityAt }, now),
    )
    expect(span.endTime).toBeNull()
    expect(span.status).toBe('pending')
  })

  it('settles a quiet trace at its last activity time', () => {
    const activityAt = 100_040 + 5_000
    const now = activityAt + TRACE_ACTIVITY_IDLE_MS + 500
    const [span] = traceListToTimelineSpans(
      [row()],
      liveness({ 't-1': activityAt }, now),
    )
    expect(span.endTime).toBe(activityAt)
    expect(span.status).toBe('ok')
  })

  it('keeps error status on a live bar', () => {
    const activityAt = 100_040 + 5_000
    const [span] = traceListToTimelineSpans(
      [row({ status: 'error' })],
      liveness({ 't-1': activityAt }, activityAt),
    )
    expect(span.endTime).toBeNull()
    expect(span.status).toBe('error')
  })

  it('ignores activity for traces that have no row', () => {
    const spans = traceListToTimelineSpans(
      [row()],
      liveness({ 'unknown-trace': 999_999_999 }, 999_999_999),
    )
    expect(spans).toHaveLength(1)
    expect(spans[0].id).toBe('t-1')
  })
})

describe('isTraceLive', () => {
  it('is live while the root itself is still pending', () => {
    const pending = row({ status: 'pending', endTime: undefined })
    expect(isTraceLive(pending, liveness({}, 100_000))).toBe(true)
  })

  it('is not live with no liveness activity at all', () => {
    expect(isTraceLive(row(), liveness({}, 100_040))).toBe(false)
  })

  it('is not live for the root close echoing back through the trigger', () => {
    const echo = 100_040 + TRACE_ACTIVITY_ECHO_MS - 1
    expect(isTraceLive(row(), liveness({ 't-1': echo }, echo + 100))).toBe(
      false,
    )
  })

  it('is live while span-close activity beyond the root is fresh', () => {
    const activityAt = 100_040 + 5_000
    const now = activityAt + TRACE_ACTIVITY_IDLE_MS - 500
    expect(isTraceLive(row(), liveness({ 't-1': activityAt }, now))).toBe(true)
  })

  it('is not live once a quiet trace ages past the idle window', () => {
    const activityAt = 100_040 + 5_000
    const now = activityAt + TRACE_ACTIVITY_IDLE_MS + 500
    expect(isTraceLive(row(), liveness({ 't-1': activityAt }, now))).toBe(false)
  })

  it('agrees with traceListToTimelineSpans on the same inputs', () => {
    const activityAt = 100_040 + 5_000
    const now = activityAt + TRACE_ACTIVITY_IDLE_MS - 500
    const live = liveness({ 't-1': activityAt }, now)
    const [span] = traceListToTimelineSpans([row()], live)
    expect(isTraceLive(row(), live)).toBe(span.endTime == null)
  })
})

function vis(overrides: Partial<VisualizationSpan> = {}): VisualizationSpan {
  return {
    name: 'op',
    span_id: 's-1',
    trace_id: 't-1',
    duration_ms: 10,
    status: 'ok',
    depth: 0,
    start_percent: 0,
    width_percent: 100,
    attributes: {},
    events: [],
    links: [],
    pending: false,
    ...overrides,
  }
}

function waterfall(spans: VisualizationSpan[]): WaterfallData {
  return { spans, total_duration_ms: 100, span_count: spans.length }
}

describe('waterfallToTimelineSpans', () => {
  it('strips verb prefixes (`execute `, `call `) from bar labels', () => {
    const { spans } = waterfallToTimelineSpans(
      waterfall([
        vis({ span_id: 'a', name: 'execute worker::list' }),
        vis({ span_id: 'b', name: 'call slow_fn' }),
        vis({ span_id: 'c', name: 'HTTP GET' }),
      ]),
    )
    expect(spans.map((s) => s.label)).toEqual([
      'worker::list',
      'slow_fn',
      'HTTP GET',
    ])
  })
})
