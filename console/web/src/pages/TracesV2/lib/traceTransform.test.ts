import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StoredSpan } from '../api/traces'
import {
  isPendingSpan,
  mergeDetailSpan,
  toWaterfallData,
} from './traceTransform'

const NOW_MS = 1_700_000_010_000

function span(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return {
    trace_id: 't-1',
    span_id: 's-1',
    name: 'call chat.respond',
    start_time_unix_nano: (NOW_MS - 5_000) * 1_000_000,
    end_time_unix_nano: (NOW_MS - 4_000) * 1_000_000,
    status: 'ok',
    attributes: [],
    events: [],
    links: [],
    ...overrides,
  }
}

function pendingSpan(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return span({
    pending: true,
    end_time_unix_nano: 0,
    status: 'unset',
    ...overrides,
  })
}

describe('isPendingSpan', () => {
  it('flags the engine pending marker and the bare zero-end sentinel', () => {
    expect(isPendingSpan(pendingSpan())).toBe(true)
    expect(isPendingSpan(span({ end_time_unix_nano: 0 }))).toBe(true)
    expect(isPendingSpan(span())).toBe(false)
  })
})

describe('toWaterfallData with pending spans', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(NOW_MS)
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('measures a pending span as elapsed-so-far and extends the window to now', () => {
    const done = span()
    const live = pendingSpan({
      span_id: 's-2',
      start_time_unix_nano: (NOW_MS - 2_000) * 1_000_000,
    })

    const wf = toWaterfallData([done, live], 't-1')
    expect(wf).not.toBeNull()
    const liveVis = wf?.spans.find((s) => s.span_id === 's-2')
    expect(liveVis?.pending).toBe(true)
    expect(liveVis?.duration_ms).toBe(2_000)
    // Window runs from the finished span's start to "now", not to end=0.
    expect(wf?.total_duration_ms).toBe(5_000)
  })

  it('produces a finite window when every span is pending', () => {
    const live = pendingSpan({
      start_time_unix_nano: (NOW_MS - 1_000) * 1_000_000,
    })
    const wf = toWaterfallData([live], 't-1')
    expect(wf?.total_duration_ms).toBe(1_000)
    expect(wf?.spans[0]?.pending).toBe(true)
  })

  it('marks finished spans pending: false', () => {
    const wf = toWaterfallData([span()], 't-1')
    expect(wf?.spans[0]?.pending).toBe(false)
    expect(wf?.spans[0]?.duration_ms).toBe(1_000)
  })
})

describe('mergeDetailSpan', () => {
  it('lets finals replace pendings, and last-write-wins otherwise', () => {
    const spans = new Map<string, StoredSpan>()
    mergeDetailSpan(spans, pendingSpan())
    expect(spans.get('s-1')?.pending).toBe(true)

    mergeDetailSpan(spans, span())
    expect(spans.get('s-1')?.pending).toBeUndefined()
    expect(spans.get('s-1')?.end_time_unix_nano).toBeGreaterThan(0)
  })

  it('never regresses a final back to a stale pending frame', () => {
    const spans = new Map<string, StoredSpan>()
    mergeDetailSpan(spans, span())
    mergeDetailSpan(spans, pendingSpan())
    expect(spans.get('s-1')?.pending).toBeUndefined()
    expect(spans.get('s-1')?.end_time_unix_nano).toBeGreaterThan(0)
  })
})
