import { describe, expect, it } from 'vitest'
import type { StoredSpan, TraceSummary } from '../api/traces'
import {
  fingerprintTraceList,
  mapSpanToListItem,
  mapTraceSummaryToListItem,
} from './traceListItem'

function root(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return {
    trace_id: 'trace-1',
    span_id: 'root-1',
    name: 'execute harness::turn',
    start_time_unix_nano: 1_000_000,
    end_time_unix_nano: 2_000_000,
    status: 'ok',
    attributes: [],
    events: [],
    links: [],
    ...overrides,
  }
}

describe('mapSpanToListItem', () => {
  it('marks an ok root as failed when a descendant stamped the trace outcome', () => {
    const item = mapSpanToListItem(
      root({ trace_tags: { 'iii.tag.outcome': 'failed' } }),
    )

    expect(item.status).toBe('error')
  })

  it('keeps a trace healthy without an error status or failure tag', () => {
    expect(mapSpanToListItem(root()).status).toBe('ok')
  })
})

describe('mapTraceSummaryToListItem', () => {
  it('uses aggregate status, counts and projected attributes directly', () => {
    const summary: TraceSummary = {
      trace_id: 'trace-summary',
      name: 'handle checkout',
      start_time_unix_nano: 1_780_000_000_000_000_000,
      end_time_unix_nano: 1_780_000_000_003_000_000,
      status: 'error',
      service_name: 'gateway',
      function_id: 'checkout::handle',
      topic: 'payments',
      trace_tags: { 'iii.tag.outcome': 'failed' },
      attributes: { 'custom.label': 'checkout' },
      span_count: 7,
      error_count: 1,
    }

    expect(mapTraceSummaryToListItem(summary)).toEqual({
      traceId: 'trace-summary',
      rootOperation: 'handle checkout',
      functionId: 'checkout::handle',
      topic: 'payments',
      status: 'error',
      startTime: 1_780_000_000_000,
      endTime: 1_780_000_000_003,
      duration: 3,
      spanCount: 7,
      workers: ['gateway'],
      attributes: { 'custom.label': 'checkout' },
      traceTags: { 'iii.tag.outcome': 'failed' },
    })
  })

  it('invalidates the list fingerprint when aggregate fields change', () => {
    const base = mapTraceSummaryToListItem({
      trace_id: 'trace-summary',
      name: 'handle checkout',
      start_time_unix_nano: 1_780_000_000_000_000_000,
      end_time_unix_nano: 1_780_000_000_003_000_000,
      status: 'ok',
      attributes: { 'custom.label': 'checkout' },
      span_count: 2,
      error_count: 0,
    })
    const changed = {
      ...base,
      endTime: (base.endTime ?? 0) + 10,
      spanCount: 3,
      attributes: { 'custom.label': 'payment' },
    }

    expect(fingerprintTraceList([changed])).not.toBe(
      fingerprintTraceList([base]),
    )
  })
})
