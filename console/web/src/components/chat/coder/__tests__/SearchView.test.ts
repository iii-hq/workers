import { describe, expect, it } from 'vitest'
import type { ContentMatch } from '../parsers'
import {
  buildGroupRows,
  formatContextRequest,
  formatMatchCount,
  groupContentMatches,
} from '../SearchView'

function match(
  overrides: Partial<ContentMatch> & { line: number },
): ContentMatch {
  return {
    path: '/repo/src/main.rs',
    column: 1,
    text: `line ${overrides.line}`,
    ...overrides,
  }
}

describe('groupContentMatches', () => {
  it('groups by path preserving first-seen file order', () => {
    const groups = groupContentMatches([
      match({ line: 3, path: '/repo/a.rs' }),
      match({ line: 7, path: '/repo/b.rs' }),
      match({ line: 9, path: '/repo/a.rs' }),
    ])
    expect(groups.map((g) => g.path)).toEqual(['/repo/a.rs', '/repo/b.rs'])
    expect(groups[0].matches.map((m) => m.line)).toEqual([3, 9])
    expect(groups[1].matches.map((m) => m.line)).toEqual([7])
  })

  it('returns an empty list for no matches', () => {
    expect(groupContentMatches([])).toEqual([])
  })
})

describe('buildGroupRows', () => {
  it('emits a single match row when no context is present', () => {
    const rows = buildGroupRows([match({ line: 12, column: 5 })])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      kind: 'match',
      line: 12,
      column: 5,
      text: 'line 12',
    })
  })

  it('derives context line numbers by offset from the match line', () => {
    const rows = buildGroupRows([
      match({ line: 10, before: ['a', 'b'], after: ['c'] }),
    ])
    expect(rows.map((r) => [r.kind, r.line])).toEqual([
      ['context', 8],
      ['context', 9],
      ['match', 10],
      ['context', 11],
    ])
    expect(rows.map((r) => r.text)).toEqual(['a', 'b', 'line 10', 'c'])
  })

  it('inserts a gap row between non-contiguous blocks only', () => {
    const rows = buildGroupRows([
      match({ line: 5, after: ['x'] }),
      // previous block ends at 6, next starts at 18 (before[] of 2) → gap
      match({ line: 20, before: ['y', 'z'] }),
      // contiguous with 20 → no gap
      match({ line: 21 }),
    ])
    expect(rows.map((r) => r.kind)).toEqual([
      'match',
      'context',
      'gap',
      'context',
      'context',
      'match',
      'match',
    ])
  })

  it('omits the gap when context windows touch or overlap', () => {
    const rows = buildGroupRows([
      match({ line: 5, after: ['x'] }), // ends at 6
      match({ line: 7 }), // starts at 7 — contiguous
    ])
    expect(rows.some((r) => r.kind === 'gap')).toBe(false)
  })

  it('assigns unique keys across rows', () => {
    const rows = buildGroupRows([
      match({ line: 10, before: ['a'], after: ['b'] }),
      match({ line: 10, before: ['a'], after: ['b'] }),
    ])
    const keys = rows.map((r) => r.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('only carries column on match rows', () => {
    const rows = buildGroupRows([
      match({ line: 4, column: 9, before: ['ctx'] }),
    ])
    const context = rows.find((r) => r.kind === 'context')
    const hit = rows.find((r) => r.kind === 'match')
    expect(context?.column).toBeUndefined()
    expect(hit?.column).toBe(9)
  })
})

describe('formatContextRequest', () => {
  it('returns null when no context was requested (absent, null, or 0)', () => {
    expect(formatContextRequest(undefined, undefined)).toBeNull()
    expect(formatContextRequest(null, null)).toBeNull()
    expect(formatContextRequest(0, 0)).toBeNull()
  })

  it('collapses symmetric context to ±N', () => {
    expect(formatContextRequest(2, 2)).toBe('±2')
  })

  it('shows only the sides that were asked for', () => {
    expect(formatContextRequest(3, null)).toBe('-3')
    expect(formatContextRequest(null, 1)).toBe('+1')
    expect(formatContextRequest(2, 4)).toBe('-2 +4')
  })
})

describe('formatMatchCount', () => {
  it('reports content hits as lines (one ContentMatch per line on wire)', () => {
    expect(formatMatchCount(1, 0)).toBe('1 line')
    expect(formatMatchCount(12, 0)).toBe('12 lines')
  })

  it('reports path hits with their own unit', () => {
    expect(formatMatchCount(0, 1)).toBe('1 path')
    expect(formatMatchCount(0, 3)).toBe('3 paths')
  })

  it('joins both kinds with a separator', () => {
    expect(formatMatchCount(12, 3)).toBe('12 lines · 3 paths')
  })

  it('falls back to "0 matches" when nothing hit', () => {
    expect(formatMatchCount(0, 0)).toBe('0 matches')
  })
})
