import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  fetchTraceSpans,
  normalizeTracesResponse,
  type StoredSpan,
  TRACES_RPC_FUNCTIONS,
  type TracesResponse,
} from './traces'

const triggerMock = vi.fn()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger: triggerMock }),
}))

function span(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return {
    trace_id: 'trace-1',
    span_id: 'root',
    name: 'root operation',
    start_time_unix_nano: 1_000_000,
    end_time_unix_nano: 5_000_000,
    status: 'ok',
    attributes: [['function_id', 'root::function']],
    events: [],
    links: [],
    service_name: 'service-a',
    ...overrides,
  }
}

beforeEach(() => triggerMock.mockReset())

describe('normalizeTracesResponse', () => {
  it('keeps the compact response untouched', () => {
    const response: TracesResponse = {
      traces: [],
      total: 123,
      offset: 50,
      limit: 50,
    }

    expect(normalizeTracesResponse(response)).toBe(response)
  })

  it('aggregates legacy spans into one trace summary', () => {
    const response = normalizeTracesResponse(
      {
        spans: [
          span(),
          span({
            span_id: 'child',
            parent_span_id: 'root',
            name: 'child operation',
            start_time_unix_nano: 2_000_000,
            end_time_unix_nano: 7_000_000,
            status: 'error',
            attributes: [
              ['custom.label', 'projected'],
              ['iii.tag.outcome', 'failed'],
            ],
          }),
        ],
        total: 2,
        offset: 0,
        limit: 50,
      },
      { attribute_projection: ['custom.label'] },
    )

    expect(response).toEqual({
      traces: [
        expect.objectContaining({
          trace_id: 'trace-1',
          name: 'root operation',
          start_time_unix_nano: 1_000_000,
          end_time_unix_nano: 7_000_000,
          status: 'error',
          function_id: 'root::function',
          span_count: 2,
          error_count: 1,
          trace_tags: { 'iii.tag.outcome': 'failed' },
          attributes: { 'custom.label': 'projected' },
        }),
      ],
      total: 1,
      offset: 0,
      limit: 50,
    })
  })
})

describe('fetchTraceSpans', () => {
  it('falls back to the legacy list RPC only when spans is unavailable', async () => {
    const legacy = {
      spans: [span()],
      total: 1,
      offset: 0,
      limit: 100,
    }
    triggerMock
      .mockRejectedValueOnce(new Error('function_not_found: not registered'))
      .mockResolvedValueOnce(legacy)

    await expect(fetchTraceSpans()).resolves.toEqual(legacy)
    expect(triggerMock).toHaveBeenNthCalledWith(1, TRACES_RPC_FUNCTIONS.spans, {
      offset: 0,
      limit: 100,
    })
    expect(triggerMock).toHaveBeenNthCalledWith(2, TRACES_RPC_FUNCTIONS.list, {
      offset: 0,
      limit: 100,
    })
  })

  it('does not hide real spans RPC failures behind the legacy endpoint', async () => {
    triggerMock.mockRejectedValueOnce(new Error('archive read failed'))

    await expect(fetchTraceSpans()).rejects.toThrow('archive read failed')
    expect(triggerMock).toHaveBeenCalledTimes(1)
  })
})
