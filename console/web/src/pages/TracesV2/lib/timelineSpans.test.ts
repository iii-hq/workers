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

  it('keeps built-in call spans — they are the invocation, not a wrapper', () => {
    // Regression: a turn's `configuration::list` call (an engine built-in,
    // `iii.function.kind: internal`) has NO worker `execute` span behind it.
    // Classifying it as engine routing erased the call — and its failure —
    // from the strip entirely.
    const bars = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'step-1',
        name: 'harness::turn step',
        parent_span_id: undefined,
      }),
      storedSpan({
        span_id: 'cfg-1',
        name: 'call configuration::list',
        parent_span_id: 'step-1',
        service_name: 'iii',
        status: 'ERROR',
        attributes: [
          ['function_id', 'configuration::list'],
          ['faas.invoked_name', 'configuration::list'],
          ['iii.function.kind', 'internal'],
        ],
      }),
    ])
    expect(bars.map((b) => b.id)).toEqual(['step-1', 'cfg-1'])
    const bar = bars[1]
    expect(bar.label).toBe('configuration::list')
    expect(bar.groupKey).toBe('configuration::list')
    expect(bar.status).toBe('error')
    expect(bar.parentId).toBe('step-1')
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

  it('groups attribute-less handler spans by their `execute <fn>` name', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({ span_id: 's-1', name: 'execute harness::send' }),
    ])
    expect(bar.groupKey).toBe('harness::send')
  })

  it('carries the internal family (`iii.tag.hidden`) as internalKey', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 's-1',
        name: 'call state::set',
        service_name: 'iii',
        attributes: [
          ['function_id', 'state::set'],
          ['iii.function.kind', 'internal'],
          ['iii.tag.hidden', 'harness state'],
        ],
      }),
    ])
    expect(bar.internalKey).toBe('harness state')
    // The strip's funnel puts internal bars in their own section; the
    // normal keys stay populated for the worker/group hides to match.
    expect(bar.groupKey).toBe('state::set')
  })

  it('groups a tag ROOT under its own name, echoes under their baggage', () => {
    const bars = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'exec-1',
        name: 'execute harness::turn',
        parent_span_id: undefined,
        attributes: [['iii.function.id', 'harness::turn']],
      }),
      // The scope span a producer opened inside the handler: same baggage,
      // but a NEW `iii.tag.kind` — a tag root, so it is its own group
      // (hiding `harness::turn` machinery must not take it down).
      storedSpan({
        span_id: 'step-1',
        name: 'harness::turn step',
        parent_span_id: 'exec-1',
        attributes: [
          ['iii.function.id', 'harness::turn'],
          ['iii.tag.kind', 'harness.turn'],
        ],
      }),
      // A child repeating the kind is a baggage echo, not a new segment.
      storedSpan({
        span_id: 'echo-1',
        name: 'execute session::append',
        parent_span_id: 'step-1',
        attributes: [
          ['iii.function.id', 'session::append'],
          ['iii.tag.kind', 'harness.turn'],
        ],
      }),
    ])
    expect(bars.map((b) => b.groupKey)).toEqual([
      'harness::turn',
      'harness::turn step',
      'session::append',
    ])
  })

  it('reports error status', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({ span_id: 's-1', status: 'ERROR' }),
    ])
    expect(bar.status).toBe('error')
  })

  it('keeps a direct non-routing parent as the hierarchy edge', () => {
    const bars = storedSpansToTimelineSpans([
      storedSpan({ span_id: 'root-1', parent_span_id: undefined }),
      storedSpan({ span_id: 'child-1', parent_span_id: 'root-1' }),
    ])
    expect(bars[0].parentId).toBeUndefined()
    expect(bars[1].parentId).toBe('root-1')
  })

  it('resolves parentId across skipped routing wrappers', () => {
    const routing = (span_id: string, name: string, parent: string) =>
      storedSpan({
        span_id,
        name,
        service_name: 'iii',
        parent_span_id: parent,
        attributes: [['function_id', 'session::update-message']],
      })
    const bars = storedSpansToTimelineSpans([
      storedSpan({ span_id: 'root-1', parent_span_id: undefined }),
      routing('call-1', 'call session::update-message', 'root-1'),
      routing('wrap-1', 'handle_invocation session::update-message', 'call-1'),
      storedSpan({ span_id: 'exec-1', parent_span_id: 'wrap-1' }),
    ])
    expect(bars.map((b) => b.id)).toEqual(['root-1', 'exec-1'])
    // exec-1's raw parent chain runs through both wrappers; the bar's edge
    // lands on the nearest span that actually renders.
    expect(bars[1].parentId).toBe('root-1')
  })

  it('keeps an unknown parent id so a late-arriving parent can connect', () => {
    const [bar] = storedSpansToTimelineSpans([
      storedSpan({ span_id: 'exec-1', parent_span_id: 'not-arrived-yet' }),
    ])
    expect(bar.parentId).toBe('not-arrived-yet')
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

// Regression: trace f6292958dfe97afbd87e323d4f4541b6 — ONE sub-agent ran,
// but baggage copied its `iii.tag.display_name` onto every span started in
// its scope (69 of 129 spans), and both timeline mappers rendered them all
// as "Sub-agent · …". The display name must land on the scope span only;
// echoes keep their own labels.
describe('display-name echo suppression', () => {
  const SUBAGENT = { 'iii.tag.kind': 'harness.subagent' }
  const SUBAGENT_TAGS = {
    ...SUBAGENT,
    'iii.tag.display_name': 'Sub-agent · List all installed and running',
  }

  it('labels only the sub-agent scope span in the strip mapping', () => {
    const bars = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'exec-1',
        name: 'execute harness::turn',
        parent_span_id: undefined,
      }),
      storedSpan({
        span_id: 'step-1',
        name: 'harness::turn step',
        parent_span_id: 'exec-1',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
      storedSpan({
        span_id: 'write-1',
        name: 'execute session::update-message',
        parent_span_id: 'step-1',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
      storedSpan({
        span_id: 'tool-1',
        name: 'execute worker::list',
        parent_span_id: 'step-1',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
      storedSpan({
        span_id: 'llm-1',
        name: 'execute router::chat',
        parent_span_id: 'step-1',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
      storedSpan({
        span_id: 'provider-1',
        name: 'execute provider::anthropic::stream',
        parent_span_id: 'llm-1',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
    ])
    expect(bars.map((b) => b.label)).toEqual([
      'harness::turn',
      'Sub-agent · List all installed and running',
      'session::update-message',
      'worker::list',
      'router::chat',
      'provider::anthropic::stream',
    ])
  })

  it('labels only the sub-agent scope span in the detail mapping', () => {
    const { spans } = waterfallToTimelineSpans(
      waterfall([
        vis({ span_id: 'exec', name: 'execute harness::turn' }),
        vis({
          span_id: 'step',
          name: 'harness::turn step',
          parent_span_id: 'exec',
          attributes: SUBAGENT_TAGS,
        }),
        vis({
          span_id: 'write',
          name: 'execute session::update-message',
          parent_span_id: 'step',
          attributes: SUBAGENT_TAGS,
        }),
        vis({
          span_id: 'tool',
          name: 'execute worker::list',
          parent_span_id: 'step',
          attributes: SUBAGENT_TAGS,
        }),
      ]),
    )
    expect(spans.map((s) => s.label)).toEqual([
      'harness::turn',
      'Sub-agent · List all installed and running',
      'session::update-message',
      'worker::list',
    ])
  })

  it('suppresses echoes past tag-less gap spans (older-SDK workers)', () => {
    // The real shape from trace f6292958dfe97afbd87e323d4f4541b6: the
    // context-manager builds against an SDK whose span processor drops
    // `iii.tag.*` baggage, so `execute context::assemble` carries no tags
    // while its `router::models::get` child re-materializes them. The
    // echo test must walk PAST the gap, not compare against it.
    const bars = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'step-1',
        name: 'harness::turn step',
        parent_span_id: undefined,
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
      storedSpan({
        span_id: 'assemble-1',
        name: 'execute context::assemble',
        parent_span_id: 'step-1',
        service_name: 'context-manager',
      }),
      storedSpan({
        span_id: 'models-1',
        name: 'execute router::models::get',
        parent_span_id: 'assemble-1',
        service_name: 'llm-router',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
    ])
    expect(bars.map((b) => b.label)).toEqual([
      'Sub-agent · List all installed and running',
      'context::assemble',
      'router::models::get',
    ])
    // Grouping must agree: the echo is NOT a tag root, so it stays with
    // its own function family instead of becoming its own menu entry.
    expect(bars[2].groupKey).toBe('router::models::get')
  })

  it('suppresses echoes even when the child row precedes its parent in the feed', () => {
    const { spans } = waterfallToTimelineSpans(
      waterfall([
        vis({
          span_id: 'write',
          name: 'execute session::update-message',
          parent_span_id: 'step',
          attributes: SUBAGENT_TAGS,
        }),
        vis({
          span_id: 'step',
          name: 'harness::turn step',
          attributes: SUBAGENT_TAGS,
        }),
      ]),
    )
    expect(spans.map((s) => s.label)).toEqual([
      'session::update-message',
      'Sub-agent · List all installed and running',
    ])
  })

  it('keeps the title on each scope root across a queue boundary re-stamp', () => {
    // A continuation step of the SAME sub-agent arrives via a fresh queue
    // delivery: the consumer scrubs the tags, the wrapper stamps its own
    // queue identity, the new step re-stamps the sub-agent tags. Both
    // steps are roots and both carry the title; the wrapper keeps its own.
    const bars = storedSpansToTimelineSpans([
      storedSpan({
        span_id: 'step-1',
        name: 'harness::turn step',
        parent_span_id: undefined,
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
      storedSpan({
        span_id: 'fnq-2',
        name: 'fn_queue default',
        parent_span_id: 'step-1',
        service_name: 'iii',
        attributes: Object.entries({
          function_id: 'harness::turn',
          'iii.tag.kind': 'queue.process',
          'iii.tag.display_name': 'queue(default) harness::turn',
        }),
      }),
      storedSpan({
        span_id: 'exec-2',
        name: 'execute harness::turn',
        parent_span_id: 'fnq-2',
      }),
      storedSpan({
        span_id: 'step-2',
        name: 'harness::turn step',
        parent_span_id: 'exec-2',
        attributes: Object.entries(SUBAGENT_TAGS),
      }),
    ])
    expect(bars.map((b) => b.label)).toEqual([
      'Sub-agent · List all installed and running',
      'queue(default) harness::turn',
      'harness::turn',
      'Sub-agent · List all installed and running',
    ])
  })
})
