import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import {
  isTraceLive,
  storedSpansToTimelineSpans,
  TRACE_ACTIVITY_ECHO_MS,
  TRACE_ACTIVITY_IDLE_MS,
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
})

/** Realistic epoch base — `toMs` only treats values past ~2100 as ns. */
const T0_MS = 1_700_000_100_000

function storedSpan(
  overrides: Partial<StoredSpan> & { span_id: string },
): StoredSpan {
  return {
    trace_id: 't-1',
    parent_span_id: 'parent-1',
    name: 'execute session::update-message',
    kind: 'internal',
    start_time_unix_nano: (T0_MS + 10) * 1_000_000,
    end_time_unix_nano: (T0_MS + 50) * 1_000_000,
    status: 'OK',
    attributes: [],
    events: [],
    links: [],
    service_name: 'harness',
    ...overrides,
  }
}

describe('storedSpansToTimelineSpans', () => {
  it('maps one bar per span with waterfall-consistent label and keys', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 's-1',
        attributes: [['function_id', 'session::update-message']],
      }),
    ])
    expect(bar).toMatchObject({
      id: 's-1',
      traceId: 't-1',
      label: 'session::update-message', // `execute ` verb stripped
      groupKey: 'session::update-message', // owning function id
      workerKey: 'harness',
      status: 'ok',
      kind: 'lambda',
    })
    // ns→ms via float loses sub-ms precision at epoch scale — tolerate it.
    expect(bar.startTime).toBeCloseTo(T0_MS + 10, 0)
    expect(bar.endTime).toBeCloseTo(T0_MS + 50, 0)
  })

  it('renders a pending span as a live bar', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({ span_id: 's-1', end_time_unix_nano: 0 }),
    ])
    expect(bar.endTime).toBeNull()
    expect(bar.status).toBe('pending')
  })

  it('skips engine routing wrappers but keeps worker spans', () => {
    const bars = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'wrap-1',
        name: 'handle_invocation session::update-message',
        service_name: 'iii',
        attributes: [['function_id', 'session::update-message']],
      }),
      storedSpan({
        span_id: 'call-1',
        name: 'call session::update-message',
        service_name: 'iii',
        attributes: [['function_id', 'session::update-message']],
      }),
      storedSpan({ span_id: 'exec-1' }),
    ])
    expect(bars.map((b) => b.id)).toEqual(['exec-1'])
  })

  it('honors producer display names and tag kinds', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'fnq-1',
        name: 'fn_queue default',
        kind: 'consumer',
        service_name: 'iii',
        attributes: [
          ['function_id', 'harness::turn'],
          ['iii.tag.kind', 'queue.process'],
          ['iii.tag.display_name', 'harness::turn (default)'],
        ],
      }),
    ])
    expect(bar.label).toBe('harness::turn (default)')
    expect(bar.kind).toBe('flame')
    expect(bar.groupKey).toBe('harness::turn')
  })

  it('groups unattributed spans by their span name', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({ span_id: 's-1', name: 'HTTP GET', kind: 'client' }),
    ])
    expect(bar.groupKey).toBe('HTTP GET')
    expect(bar.kind).toBe('sparkle')
  })

  it('reports error status', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({ span_id: 's-1', status: 'ERROR' }),
    ])
    expect(bar.status).toBe('error')
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
