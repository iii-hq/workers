import { readFileSync } from 'node:fs'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createSeenSpans,
  feedSpansFrom,
  MAX_TRACE_IDS,
  type SpanEvent,
  type SpanFeedBus,
  startSpanFeed,
  traceIdsOf,
} from '../../../ui/src/catalog/span-feed'

const T0 = 1_800_000_000_000

interface SpanInit {
  trace?: string
  startMs?: number
  endMs?: number | null
  parent?: string | null
  name?: string
  attributes?: [string, string][]
  status?: string
}

function span(id: string, fn: string, init: SpanInit = {}) {
  const startMs = init.startMs ?? T0 + 10
  const endMs = init.endMs === undefined ? startMs + 5 : init.endMs
  return {
    span_id: id,
    trace_id: init.trace ?? 't1',
    parent_span_id: init.parent === undefined ? 'root' : init.parent,
    name: init.name ?? `execute ${fn}`,
    service_name: 'demo',
    status: init.status ?? 'ok',
    start_time_unix_nano: startMs * 1e6,
    end_time_unix_nano: endMs === null ? 0 : endMs * 1e6,
    attributes: init.attributes ?? [],
  }
}

type Responder = (fn: string, payload: Record<string, unknown>) => unknown

function fakeBus(respond: Responder) {
  const handlers = new Map<string, (payload: unknown) => void>()
  const triggers: { type: string; function_id: string; config: unknown }[] = []
  const calls: { fn: string; payload: Record<string, unknown> }[] = []
  let unregistered = 0
  let connection: ((state: unknown) => void) | undefined
  const bus = {
    browserId: 'console-tab1',
    on(id: string, handler: (payload: unknown) => void) {
      handlers.set(id, handler)
      return () => handlers.delete(id)
    },
    registerTrigger(input: {
      type: string
      function_id: string
      config: unknown
    }) {
      triggers.push(input)
      return () => {
        unregistered += 1
      }
    },
    async trigger(fn: string, payload: Record<string, unknown> = {}) {
      calls.push({ fn, payload })
      return respond(fn, payload)
    },
    addConnectionStateListener(handler: (state: unknown) => void) {
      connection = handler
      return () => {
        connection = undefined
      }
    },
  }
  return {
    bus: bus as unknown as SpanFeedBus,
    handlers,
    triggers,
    calls,
    tick: (traceIds: unknown) =>
      handlers.get('iii::console-catalog::spans-1')?.({ trace_ids: traceIds }),
    connection: (state: unknown) => connection?.(state),
    get unregistered() {
      return unregistered
    },
  }
}

describe('feedSpansFrom', () => {
  it('keeps execute spans, drops caller and context-free internal spans', () => {
    const out = feedSpansFrom([
      span('a', 'orders::create'),
      span('b', 'orders::create', { name: 'call orders::create' }),
      span('c', 'engine::traces::list', {
        parent: null,
        attributes: [['function_id', 'engine::traces::list']],
      }),
      span('d', 'configuration::list', {
        attributes: [['iii.function.kind', 'internal']],
      }),
      span('e', 'x', { startMs: 0 }),
      null,
      { name: 'execute nope' },
    ])
    expect(out.map((s) => s.spanId)).toEqual(['a', 'd'])
    expect(out[0]?.event).toEqual({
      functionId: 'orders::create',
      worker: 'demo',
      // Nanosecond timestamps exceed 2^53: sub-microsecond drift is expected.
      durationMs: expect.closeTo(5, 2),
      ok: true,
      atMs: expect.closeTo(T0 + 10, 0),
    })
  })

  it('reads in-flight spans as pending with zero duration', () => {
    const [pending] = feedSpansFrom([
      span('a', 'f', { endMs: null, status: 'unset' }),
    ])
    expect(pending?.final).toBe(false)
    expect(pending?.event.durationMs).toBe(0)
    const [failed] = feedSpansFrom([span('b', 'f', { status: 'error' })])
    expect(failed?.event.ok).toBe(false)
  })
})

describe('createSeenSpans', () => {
  it('delivers a span once pending and once final, never again', () => {
    const seen = createSeenSpans()
    const pending = feedSpansFrom([span('a', 'f', { endMs: null })])
    const final = feedSpansFrom([span('a', 'f')])
    expect(seen.fresh(pending)).toHaveLength(1)
    expect(seen.fresh(pending)).toHaveLength(0)
    expect(seen.fresh(final)).toHaveLength(1)
    expect(seen.fresh(final)).toHaveLength(0)
    expect(seen.fresh(pending)).toHaveLength(0)
  })

  it('is bounded, forgetting the oldest ids first', () => {
    const seen = createSeenSpans(3)
    seen.fresh(feedSpansFrom(['a', 'b', 'c', 'd'].map((id) => span(id, 'f'))))
    expect(seen.size).toBe(3)
    expect(seen.fresh(feedSpansFrom([span('a', 'f')]))).toHaveLength(1)
    expect(seen.fresh(feedSpansFrom([span('d', 'f')]))).toHaveLength(0)
  })
})

describe('traceIdsOf', () => {
  it('extracts string ids and tolerates malformed payloads', () => {
    expect(traceIdsOf({ trace_ids: ['a', 1, 'b'] })).toEqual(['a', 'b'])
    expect(traceIdsOf({ trace_ids: 'a' })).toEqual([])
    expect(traceIdsOf(null)).toEqual([])
  })
})

describe('startSpanFeed', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(T0)
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  function start(respond: Responder) {
    const fake = fakeBus(respond)
    const batches: SpanEvent[][] = []
    const stop = startSpanFeed(fake.bus, (spans) => batches.push(spans), {
      handlerId: 'iii::console-catalog::spans-1',
      isPaused: () => false,
    })
    return Object.assign(fake, { batches, stop })
  }

  it('binds the engine trace trigger, not a stream', () => {
    const feed = start(() => ({ spans: [] }))
    expect(feed.triggers).toEqual([
      {
        type: 'trace',
        function_id: 'iii::console-catalog::spans-1::console-tab1',
        config: {},
      },
    ])
    feed.stop()
    expect(feed.unregistered).toBe(1)
    expect(feed.handlers.size).toBe(0)
  })

  it('re-reads ticked traces with a bounded spans query and delivers new calls once', async () => {
    let rows = [span('a', 'orders::create', { endMs: null })]
    const feed = start(() => ({ spans: rows }))
    feed.tick(['t1'])
    feed.tick(['t1', 't2'])
    await vi.advanceTimersByTimeAsync(300)
    expect(feed.calls).toHaveLength(1)
    expect(feed.calls[0]).toEqual({
      fn: 'engine::traces::spans',
      payload: {
        trace_ids: ['t1', 't2'],
        search_all_spans: true,
        include_internal: true,
        include_events: false,
        start_time: T0 - 1_000,
        sort_by: 'start_time',
        sort_order: 'desc',
        limit: 200,
      },
    })
    expect(feed.batches).toHaveLength(1)
    expect(feed.batches[0]?.[0]?.durationMs).toBe(0)

    // Same pending span again: no re-delivery. Then it closes: one more.
    feed.tick(['t1'])
    await vi.advanceTimersByTimeAsync(1_000)
    expect(feed.calls).toHaveLength(2)
    expect(feed.batches).toHaveLength(1)
    rows = [span('a', 'orders::create')]
    feed.tick(['t1'])
    await vi.advanceTimersByTimeAsync(1_000)
    expect(feed.batches).toHaveLength(2)
    expect(feed.batches[1]?.[0]?.durationMs).toBeCloseTo(5, 2)
    feed.stop()
  })

  it('starts at most one read per second under a tick storm', async () => {
    const feed = start(() => ({ spans: [] }))
    for (let i = 0; i < 20; i += 1) {
      feed.tick([`t${i}`])
      await vi.advanceTimersByTimeAsync(100)
    }
    await vi.advanceTimersByTimeAsync(2_000)
    // 2s of ticks + drain: the floor keeps it to a handful, not 20.
    expect(feed.calls.length).toBeGreaterThanOrEqual(2)
    expect(feed.calls.length).toBeLessThanOrEqual(4)
    const asked = feed.calls.flatMap((c) => c.payload.trace_ids as string[])
    expect(new Set(asked).size).toBe(20)
    feed.stop()
  })

  it('switches to a bounded recovery read when ticked ids overflow', async () => {
    const feed = start((fn) =>
      fn === 'engine::traces::list'
        ? { traces: [{ trace_id: 'r1' }, { trace_id: 'r2' }] }
        : { spans: [span('a', 'f', { trace: 'r1' })] },
    )
    feed.tick(Array.from({ length: MAX_TRACE_IDS + 5 }, (_, i) => `t${i}`))
    await vi.advanceTimersByTimeAsync(300)
    expect(feed.calls.map((c) => c.fn)).toEqual([
      'engine::traces::list',
      'engine::traces::spans',
    ])
    expect(feed.calls[0]?.payload).toMatchObject({
      include_internal: false,
      limit: MAX_TRACE_IDS,
      sort_order: 'desc',
    })
    expect(feed.calls[1]?.payload.trace_ids).toEqual(['r1', 'r2'])
    expect(feed.batches).toHaveLength(1)
    feed.stop()
  })

  it('recovers after a reconnect, not on the first connect', async () => {
    const feed = start((fn) =>
      fn === 'engine::traces::list' ? { traces: [] } : { spans: [] },
    )
    feed.connection('connecting')
    feed.connection('connected')
    await vi.advanceTimersByTimeAsync(1_000)
    expect(feed.calls).toHaveLength(0)
    feed.connection('reconnecting')
    feed.connection('connected')
    await vi.advanceTimersByTimeAsync(300)
    expect(feed.calls.map((c) => c.fn)).toEqual(['engine::traces::list'])
    feed.stop()
  })

  it('survives a failed read and a missing trigger type', async () => {
    const fake = fakeBus(() => {
      throw new Error('memory_exporter_not_enabled')
    })
    fake.bus.registerTrigger = () => {
      throw new Error('trigger type not found')
    }
    const batches: SpanEvent[][] = []
    const stop = startSpanFeed(fake.bus, (spans) => batches.push(spans), {
      handlerId: 'iii::console-catalog::spans-1',
      isPaused: () => false,
    })
    fake.tick(['t1'])
    await vi.advanceTimersByTimeAsync(300)
    expect(fake.calls).toHaveLength(1)
    expect(batches).toHaveLength(0)
    stop()
  })

  it('defers reads while paused and drops them after cleanup', async () => {
    let paused = true
    const fake = fakeBus(() => ({ spans: [] }))
    const stop = startSpanFeed(fake.bus, () => {}, {
      handlerId: 'iii::console-catalog::spans-1',
      isPaused: () => paused,
    })
    fake.tick(['t1'])
    await vi.advanceTimersByTimeAsync(1_000)
    expect(fake.calls).toHaveLength(0)
    paused = false
    fake.tick(['t2'])
    await vi.advanceTimersByTimeAsync(300)
    expect(fake.calls[0]?.payload.trace_ids).toEqual(['t1', 't2'])
    stop()
    fake.tick(['t3'])
    await vi.advanceTimersByTimeAsync(2_000)
    expect(fake.calls).toHaveLength(1)
  })
})

describe('catalog live spans source', () => {
  it('no longer binds an iii-stream feed', () => {
    for (const file of ['span-feed.ts', 'engine.ts', 'live.tsx']) {
      const source = readFileSync(
        new URL(`../../../ui/src/catalog/${file}`, import.meta.url),
        'utf8',
      )
      expect(source).not.toMatch(/type: *['"]stream|stream_name|iii:devtools/)
    }
  })
})
