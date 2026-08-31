import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { StoredSpan, TraceSummary } from '../api/traces'

const { fetchTraceSpansMock, fetchTracesMock } = vi.hoisted(() => ({
  fetchTraceSpansMock: vi.fn(),
  fetchTracesMock: vi.fn(),
}))

vi.mock('../api/traces', () => ({
  fetchTraceSpans: fetchTraceSpansMock,
  fetchTraces: fetchTracesMock,
}))

import {
  fetchLiveSpanSeed,
  isContextFreeInternalSpan,
  selectSeedTraceIds,
} from './useAllSpans'

function span(
  overrides: Partial<StoredSpan> & { span_id: string },
): StoredSpan {
  return {
    trace_id: 't-1',
    name: 'op',
    start_time_unix_nano: 1,
    end_time_unix_nano: 2,
    status: 'OK',
    attributes: [],
    events: [],
    links: [],
    ...overrides,
  }
}

function summary(traceId: string, spanCount: number): TraceSummary {
  return {
    trace_id: traceId,
    name: `trace ${traceId}`,
    start_time_unix_nano: 1,
    status: 'ok',
    span_count: spanCount,
    error_count: 0,
  }
}

beforeEach(() => {
  fetchTraceSpansMock.mockReset()
  fetchTracesMock.mockReset()
})

// Mirrors `is_context_free_internal_span` in the engine's observability
// worker: the seed must exclude exactly what the live all-spans feed
// excludes, or builtin bars flip in and out of the masthead across reseeds.
describe('isContextFreeInternalSpan', () => {
  it('flags parentless internal spans (engine machinery)', () => {
    expect(
      isContextFreeInternalSpan(
        span({
          span_id: 's1',
          name: 'call stream::send',
          attributes: [['iii.function.kind', 'internal']],
        }),
      ),
    ).toBe(true)
    expect(
      isContextFreeInternalSpan(
        span({
          span_id: 's2',
          name: 'call engine::functions::list',
          attributes: [['function_id', 'engine::functions::list']],
        }),
      ),
    ).toBe(true)
  })

  it('keeps parented internal spans — built-in calls inside a real trace', () => {
    expect(
      isContextFreeInternalSpan(
        span({
          span_id: 's3',
          parent_span_id: 'step-1',
          name: 'call configuration::list',
          attributes: [
            ['function_id', 'configuration::list'],
            ['iii.function.kind', 'internal'],
          ],
        }),
      ),
    ).toBe(false)
  })

  it('keeps ordinary parentless user roots', () => {
    expect(
      isContextFreeInternalSpan(
        span({ span_id: 's4', name: 'execute harness::send' }),
      ),
    ).toBe(false)
  })
})

describe('selectSeedTraceIds', () => {
  it('selects newest traces until their span counts fill the seed budget', () => {
    const traces = [summary('t-1', 220), summary('t-2', 280), summary('t-3', 5)]

    expect(selectSeedTraceIds(traces)).toEqual(['t-1', 't-2'])
  })

  it('counts an empty legacy summary as at least one span', () => {
    const traces = Array.from({ length: 501 }, (_, index) =>
      summary(`t-${index}`, 0),
    )

    expect(selectSeedTraceIds(traces)).toHaveLength(500)
  })
})

describe('fetchLiveSpanSeed', () => {
  it('loads compact summaries before fetching recent spans by trace id', async () => {
    const recentSpan = span({ span_id: 'recent' })
    fetchTracesMock.mockResolvedValue({
      traces: [summary('t-1', 300), summary('t-2', 200), summary('t-3', 100)],
      total: 3,
      offset: 0,
      limit: 500,
    })
    fetchTraceSpansMock.mockResolvedValue({
      spans: [recentSpan],
      total: 1,
      offset: 0,
      limit: 500,
    })

    await expect(fetchLiveSpanSeed(1_000_000)).resolves.toEqual([recentSpan])
    expect(fetchTracesMock).toHaveBeenCalledWith({
      include_internal: false,
      sort_by: 'start_time',
      sort_order: 'desc',
      limit: 500,
    })
    expect(fetchTraceSpansMock).toHaveBeenCalledWith({
      trace_ids: ['t-1', 't-2'],
      search_all_spans: true,
      include_internal: true,
      start_time: 880_000,
      sort_by: 'start_time',
      sort_order: 'desc',
      limit: 500,
    })
  })

  it('skips the full-span request when no trace summary exists', async () => {
    fetchTracesMock.mockResolvedValue({
      traces: [],
      total: 0,
      offset: 0,
      limit: 500,
    })

    await expect(fetchLiveSpanSeed()).resolves.toEqual([])
    expect(fetchTraceSpansMock).not.toHaveBeenCalled()
  })

  it('keeps the global seed for engines that return the legacy list contract', async () => {
    const legacySpan = span({ span_id: 'legacy' })
    fetchTracesMock.mockResolvedValue({
      traces: [summary('t-legacy', 1)],
      total: 1,
      offset: 0,
      limit: 500,
      legacyContract: true,
    })
    fetchTraceSpansMock.mockResolvedValue({
      spans: [legacySpan],
      total: 1,
      offset: 0,
      limit: 500,
    })

    await expect(fetchLiveSpanSeed()).resolves.toEqual([legacySpan])
    expect(fetchTraceSpansMock).toHaveBeenCalledWith({
      search_all_spans: true,
      include_internal: true,
      sort_by: 'start_time',
      sort_order: 'desc',
      limit: 500,
    })
  })
})
