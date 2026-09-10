import { describe, expect, it } from 'vitest'
import {
  EMPTY_SPAN_FILTER_PREFS,
  effectiveSpanFilters,
  parseSpanFilters,
  withSpanFilters,
} from './spanFilters'

describe('parseSpanFilters', () => {
  it('reads hidden groups, workers, and shown groups from traces.spanFilters', () => {
    const out = parseSpanFilters({
      traces: {
        spanFilters: {
          hiddenGroups: ['session::update-message'],
          hiddenWorkers: ['postgres'],
          shownGroups: ['harness::turn'],
        },
      },
    })
    expect([...out.hiddenGroups]).toEqual(['session::update-message'])
    expect([...out.hiddenWorkers]).toEqual(['postgres'])
    expect([...out.shownGroups]).toEqual(['harness::turn'])
  })

  it('tolerates pre-shownGroups persisted shapes', () => {
    const out = parseSpanFilters({
      traces: { spanFilters: { hiddenGroups: ['g'], hiddenWorkers: [] } },
    })
    expect([...out.hiddenGroups]).toEqual(['g'])
    expect(out.shownGroups.size).toBe(0)
  })

  it('returns the empty prefs for missing or malformed shapes', () => {
    expect(parseSpanFilters({})).toBe(EMPTY_SPAN_FILTER_PREFS)
    expect(parseSpanFilters({ traces: 'nope' })).toBe(EMPTY_SPAN_FILTER_PREFS)
    expect(parseSpanFilters({ traces: {} })).toBe(EMPTY_SPAN_FILTER_PREFS)
    expect(parseSpanFilters({ traces: { spanFilters: [] } })).toEqual(
      EMPTY_SPAN_FILTER_PREFS,
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
        shownGroups: new Set(['s2', 's1']),
        shownInternal: new Set(['session events']),
      },
    )
    expect(out).toEqual({
      other: true,
      traces: {
        views: [{ id: 'v1' }],
        spanFilters: {
          hiddenGroups: ['a', 'b'],
          hiddenWorkers: ['worker-1', 'worker-2'],
          shownGroups: ['s1', 's2'],
          shownInternal: ['session events'],
        },
      },
    })
  })

  it('round-trips through parse', () => {
    const prefs = {
      hiddenGroups: new Set(['g']),
      hiddenWorkers: new Set(['w']),
      shownGroups: new Set(['s']),
      shownInternal: new Set(['harness state']),
    }
    expect(parseSpanFilters(withSpanFilters({}, prefs))).toEqual(prefs)
  })
})

describe('effectiveSpanFilters', () => {
  it('hides producer defaults unless the user unhid them', () => {
    const out = effectiveSpanFilters(
      {
        hiddenGroups: new Set(['user-hidden']),
        hiddenWorkers: new Set(['w']),
        shownGroups: new Set(['harness::turn']),
        shownInternal: new Set(['session events']),
      },
      new Set(['harness::turn', 'session::append']),
    )
    expect([...out.hiddenGroups].sort()).toEqual([
      'session::append',
      'user-hidden',
    ])
    expect([...out.hiddenWorkers]).toEqual(['w'])
    // The internal reveals pass through untouched — internal spans are
    // hidden by default, so the selection only carries the exceptions.
    expect([...out.shownInternal]).toEqual(['session events'])
  })

  it('returns the prefs untouched when there are no defaults', () => {
    const prefs = {
      hiddenGroups: new Set(['g']),
      hiddenWorkers: new Set<string>(),
      shownGroups: new Set<string>(),
      shownInternal: new Set<string>(),
    }
    expect(effectiveSpanFilters(prefs, new Set())).toBe(prefs)
  })
})
