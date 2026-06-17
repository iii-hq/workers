import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type { StoredSpan } from '@/pages/Traces/api/traces'
import {
  extractStreamSpans,
  isAppendableTraceList,
  mergeTraceListSpans,
  startTraceListStream,
  startTraceSpansStream,
} from './traces-stream'

const NS = 1_000_000

function span(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return {
    trace_id: 't-1',
    span_id: 's-1',
    name: 'call x',
    start_time_unix_nano: 1_700_000_000_000 * NS,
    end_time_unix_nano: 1_700_000_000_010 * NS,
    status: 'OK',
    attributes: [],
    events: [],
    links: [],
    service_name: 'svc',
    ...overrides,
  }
}

/** A `stream::send` Event frame, as the engine serializes StreamWrapperMessage. */
function sendFrame(streamName: string, groupId: string, spans: StoredSpan[]) {
  return {
    type: 'stream',
    timestamp: 0,
    streamName,
    groupId,
    id: null,
    event: { type: 'event', event: { type: 'spans', data: { spans } } },
  }
}

function fakeClient() {
  const triggers: Array<{
    type: string
    function_id: string
    config?: unknown
  }> = []
  const handlers = new Map<string, (p: unknown) => void>()
  const offHandlers = new Map<string, ReturnType<typeof vi.fn>>()
  const triggerUnregister = vi.fn()

  const on = vi.fn((fn: string, handler: (p: unknown) => void) => {
    handlers.set(fn, handler)
    const off = vi.fn()
    offHandlers.set(fn, off)
    return off
  })
  const registerTrigger = vi.fn(
    (input: { type: string; function_id: string; config?: unknown }) => {
      triggers.push(input)
      return triggerUnregister
    },
  )

  const client = {
    browserId: 'console-test',
    on,
    registerTrigger,
    addConnectionStateListener: vi.fn(() => vi.fn()),
    trigger: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    registerTrigger,
    triggers,
    triggerUnregister,
    offHandlers,
    fire: (fn: string, frame: unknown) => handlers.get(fn)?.(frame),
  }
}

describe('extractStreamSpans', () => {
  it('reads the spans out of a `send` Event frame (nested event.event.data)', () => {
    const spans = [span({ span_id: 'a' }), span({ span_id: 'b' })]
    const out = extractStreamSpans(
      sendFrame('iii:devtools:trace-rows', 'all', spans),
    )
    expect(out.map((s) => s.span_id)).toEqual(['a', 'b'])
  })

  it('falls back to the flat Create/Update shape ({ event: { data } })', () => {
    const spans = [span({ span_id: 'c' })]
    const frame = {
      streamName: 's',
      groupId: 'g',
      event: { type: 'create', data: { spans } },
    }
    expect(extractStreamSpans(frame).map((s) => s.span_id)).toEqual(['c'])
  })

  it('returns [] for malformed / empty / missing-spans frames', () => {
    expect(extractStreamSpans(null)).toEqual([])
    expect(extractStreamSpans({})).toEqual([])
    expect(extractStreamSpans({ event: {} })).toEqual([])
    expect(
      extractStreamSpans({
        event: { type: 'event', event: { type: 'spans', data: {} } },
      }),
    ).toEqual([])
    expect(
      extractStreamSpans({
        event: {
          type: 'event',
          event: { type: 'spans', data: { spans: 'nope' } },
        },
      }),
    ).toEqual([])
  })
})

describe('mergeTraceListSpans', () => {
  it('seeds from an empty list', () => {
    const out = mergeTraceListSpans([], [span({ trace_id: 'a' })], 500)
    expect(out).toHaveLength(1)
    expect(out[0].trace_id).toBe('a')
  })

  it('dedupes by trace_id (one row per trace)', () => {
    const out = mergeTraceListSpans(
      [span({ trace_id: 'a', span_id: 'old', start_time_unix_nano: 1 * NS })],
      [span({ trace_id: 'a', span_id: 'new', start_time_unix_nano: 2 * NS })],
      500,
    )
    expect(out).toHaveLength(1)
    expect(out[0].span_id).toBe('new')
  })

  it('prefers a root span over a non-root for the same trace', () => {
    const out = mergeTraceListSpans(
      [span({ trace_id: 'a', span_id: 'child', parent_span_id: 'p' })],
      [span({ trace_id: 'a', span_id: 'root' })],
      500,
    )
    expect(out[0].span_id).toBe('root')
  })

  it('sorts newest-first and caps the list', () => {
    const incoming = [
      span({ trace_id: 'a', start_time_unix_nano: 1 }),
      span({ trace_id: 'b', start_time_unix_nano: 3 }),
      span({ trace_id: 'c', start_time_unix_nano: 2 }),
    ]
    const out = mergeTraceListSpans([], incoming, 2)
    expect(out.map((s) => s.trace_id)).toEqual(['b', 'c'])
  })

  it('does not mutate its inputs', () => {
    const existing = [span({ trace_id: 'a' })]
    const incoming = [span({ trace_id: 'b' })]
    mergeTraceListSpans(existing, incoming, 500)
    expect(existing).toHaveLength(1)
    expect(incoming).toHaveLength(1)
  })
})

describe('isAppendableTraceList', () => {
  it('appends the default unfiltered view (sort defaults are present, not counted)', () => {
    // The filter builder always emits the default sort, so an "empty" view
    // still carries sort_by/sort_order — these must NOT block append.
    expect(
      isAppendableTraceList({ sort_by: 'start_time', sort_order: 'desc' }, ''),
    ).toBe(true)
    expect(isAppendableTraceList({}, '')).toBe(true)
  })

  it('refetches (no append) when a search is active', () => {
    expect(isAppendableTraceList({}, 'harness::send')).toBe(false)
  })

  it('refetches when any content filter is set', () => {
    expect(isAppendableTraceList({ service_name: 'svc' }, '')).toBe(false)
    expect(isAppendableTraceList({ name: 'GET /x' }, '')).toBe(false)
    expect(isAppendableTraceList({ status: 'error' }, '')).toBe(false)
    expect(isAppendableTraceList({ min_duration_ms: 5 }, '')).toBe(false)
    expect(isAppendableTraceList({ start_time: 1 }, '')).toBe(false)
    expect(isAppendableTraceList({ search_all_spans: true }, '')).toBe(false)
    expect(isAppendableTraceList({ attributes: [['k', 'v']] }, '')).toBe(false)
  })

  it('refetches when sorted by anything other than newest-first', () => {
    expect(
      isAppendableTraceList({ sort_by: 'duration', sort_order: 'desc' }, ''),
    ).toBe(false)
    expect(
      isAppendableTraceList({ sort_by: 'start_time', sort_order: 'asc' }, ''),
    ).toBe(false)
  })
})

describe('startTraceListStream', () => {
  it('registers the rows handler and a stream trigger on the global group', () => {
    const { client, on, triggers } = fakeClient()
    startTraceListStream(client, () => {})

    expect(on).toHaveBeenCalledWith(
      'iii::console::trace_rows',
      expect.any(Function),
    )
    expect(triggers).toEqual([
      {
        type: 'stream',
        function_id: 'iii::console::trace_rows::console-test',
        config: { stream_name: 'iii:devtools:trace-rows', group_id: 'all' },
      },
    ])
  })

  it('delivers extracted spans to onSpans (and ignores empty frames)', () => {
    const { client, fire } = fakeClient()
    const onSpans = vi.fn()
    startTraceListStream(client, onSpans)

    fire(
      'iii::console::trace_rows',
      sendFrame('iii:devtools:trace-rows', 'all', []),
    )
    expect(onSpans).not.toHaveBeenCalled()

    const spans = [span({ trace_id: 'z' })]
    fire(
      'iii::console::trace_rows',
      sendFrame('iii:devtools:trace-rows', 'all', spans),
    )
    expect(onSpans).toHaveBeenCalledTimes(1)
    expect(onSpans.mock.calls[0][0].map((s: StoredSpan) => s.trace_id)).toEqual(
      ['z'],
    )
  })

  it('unregisters the handler and trigger on cleanup', () => {
    const { client, offHandlers, triggerUnregister } = fakeClient()
    const stop = startTraceListStream(client, () => {})
    stop()
    expect(offHandlers.get('iii::console::trace_rows')).toHaveBeenCalledTimes(1)
    expect(triggerUnregister).toHaveBeenCalledTimes(1)
  })
})

describe('startTraceSpansStream', () => {
  it('registers the spans handler and a trigger scoped to the trace group', () => {
    const { client, on, triggers } = fakeClient()
    startTraceSpansStream(client, 'trace-42', () => {})

    expect(on).toHaveBeenCalledWith(
      'iii::console::trace_spans',
      expect.any(Function),
    )
    expect(triggers).toEqual([
      {
        type: 'stream',
        function_id: 'iii::console::trace_spans::console-test',
        config: {
          stream_name: 'iii:devtools:trace-spans',
          group_id: 'trace-42',
        },
      },
    ])
  })

  it('filters delivered spans to the subscribed trace (defense-in-depth)', () => {
    const { client, fire } = fakeClient()
    const onSpans = vi.fn()
    startTraceSpansStream(client, 'trace-42', onSpans)

    const spans = [
      span({ trace_id: 'trace-42', span_id: 'keep' }),
      span({ trace_id: 'other', span_id: 'drop' }),
    ]
    fire(
      'iii::console::trace_spans',
      sendFrame('iii:devtools:trace-spans', 'trace-42', spans),
    )

    expect(onSpans).toHaveBeenCalledTimes(1)
    expect(onSpans.mock.calls[0][0].map((s: StoredSpan) => s.span_id)).toEqual([
      'keep',
    ])
  })

  it('unregisters the handler and trigger on cleanup', () => {
    const { client, offHandlers, triggerUnregister } = fakeClient()
    const stop = startTraceSpansStream(client, 'trace-42', () => {})
    stop()
    expect(offHandlers.get('iii::console::trace_spans')).toHaveBeenCalledTimes(
      1,
    )
    expect(triggerUnregister).toHaveBeenCalledTimes(1)
  })
})
