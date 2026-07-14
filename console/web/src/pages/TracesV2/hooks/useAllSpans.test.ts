import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import { isContextFreeInternalSpan } from './useAllSpans'

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
