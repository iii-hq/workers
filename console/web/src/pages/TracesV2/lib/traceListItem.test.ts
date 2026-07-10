import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import { mapSpanToListItem } from './traceListItem'

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
