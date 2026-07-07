import { describe, expect, it } from 'vitest'
import {
  EMPTY_SPAN_FILTERS,
  parseSpanFilters,
  withSpanFilters,
} from './spanFilters'

describe('parseSpanFilters', () => {
  it('reads hidden groups and workers from traces.spanFilters', () => {
    const out = parseSpanFilters({
      traces: {
        spanFilters: {
          hiddenGroups: ['session::update-message'],
          hiddenWorkers: ['postgres'],
        },
      },
    })
    expect([...out.hiddenGroups]).toEqual(['session::update-message'])
    expect([...out.hiddenWorkers]).toEqual(['postgres'])
  })

  it('returns the empty selection for missing or malformed shapes', () => {
    expect(parseSpanFilters({})).toBe(EMPTY_SPAN_FILTERS)
    expect(parseSpanFilters({ traces: 'nope' })).toBe(EMPTY_SPAN_FILTERS)
    expect(parseSpanFilters({ traces: {} })).toBe(EMPTY_SPAN_FILTERS)
    expect(parseSpanFilters({ traces: { spanFilters: [] } })).toEqual(
      EMPTY_SPAN_FILTERS,
    )
  })

  it('drops non-string entries instead of erroring', () => {
    const out = parseSpanFilters({
      traces: { spanFilters: { hiddenGroups: ['ok', 7, null] } },
    })
    expect([...out.hiddenGroups]).toEqual(['ok'])
    expect(out.hiddenWorkers.size).toBe(0)
  })
})

describe('withSpanFilters', () => {
  it('writes sorted arrays and preserves sibling traces keys', () => {
    const out = withSpanFilters(
      { traces: { views: [{ id: 'v1' }] }, other: true },
      {
        hiddenGroups: new Set(['b', 'a']),
        hiddenWorkers: new Set(['worker-2', 'worker-1']),
      },
    )
    expect(out).toEqual({
      other: true,
      traces: {
        views: [{ id: 'v1' }],
        spanFilters: {
          hiddenGroups: ['a', 'b'],
          hiddenWorkers: ['worker-1', 'worker-2'],
        },
      },
    })
  })

  it('round-trips through parse', () => {
    const selection = {
      hiddenGroups: new Set(['g']),
      hiddenWorkers: new Set(['w']),
    }
    expect(parseSpanFilters(withSpanFilters({}, selection))).toEqual(selection)
  })
})
