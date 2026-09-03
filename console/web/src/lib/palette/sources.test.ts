import { describe, expect, it } from 'vitest'
import {
  BARE_SEARCH_KINDS,
  DEFAULT_KINDS,
  filterEntries,
  groupEntries,
  KIND_ORDER,
  type PaletteEntry,
  type PaletteKind,
  parseQuery,
  scoreEntry,
} from './sources'

function entry(
  title: string,
  kind: PaletteKind = 'action',
  rest: Partial<PaletteEntry> = {},
): PaletteEntry {
  return { id: `${kind}:${title}`, kind, title, run: () => {}, ...rest }
}

function parsedKinds(
  query: string,
  filter: PaletteKind | 'all',
): PaletteKind[] {
  return [...(parseQuery(query, filter).kinds ?? [])]
}

describe('parseQuery', () => {
  it('keeps the empty All view focused on navigation', () => {
    expect(parsedKinds('', 'all')).toEqual(DEFAULT_KINDS)
  })

  it('includes commands and actions in a typed All search', () => {
    const parsed = parseQuery('split', 'all')
    expect(parsed.text).toBe('split')
    expect(parsedKinds('split', 'all')).toEqual(BARE_SEARCH_KINDS)
    expect(parsed.kinds?.has('file')).toBe(false)
    expect(parsed.kinds?.has('function')).toBe(false)
  })

  it('keeps explicit prefixes and filter chips narrowly scoped', () => {
    expect(parsedKinds('> split', 'all')).toEqual(['command', 'action'])
    expect(parsedKinds('split', 'action')).toEqual(['action'])
  })
})

describe('scoreEntry', () => {
  it('keeps everything when there is nothing to match against', () => {
    expect(scoreEntry(entry('shell'), '')).toBe(1)
  })

  it('ranks exact over prefix over word boundary over anywhere', () => {
    const exact = scoreEntry(entry('shell'), 'shell')
    const prefix = scoreEntry(entry('shell-agent'), 'shell')
    const boundary = scoreEntry(entry('worker::shell'), 'shell')
    const anywhere = scoreEntry(entry('reshell'), 'shell')
    expect(exact).toBeGreaterThan(prefix)
    expect(prefix).toBeGreaterThan(boundary)
    expect(boundary).toBeGreaterThan(anywhere)
    expect(anywhere).toBeGreaterThan(0)
  })

  it('falls back to a subsequence for ids and paths, below every literal match', () => {
    const fn = entry('engine::workers::list', 'function')
    const subsequence = scoreEntry(fn, 'ewl')
    expect(subsequence).toBeGreaterThan(0)
    expect(subsequence).toBeLessThan(scoreEntry(fn, 'workers'))
  })

  it('never fuzzes a prose title', () => {
    expect(scoreEntry(entry('Open settings'), 'ping')).toBe(0)
    expect(scoreEntry(entry('Open settings'), 'settings')).toBeGreaterThan(0)
  })

  it('rejects an entry it cannot match at all', () => {
    expect(scoreEntry(entry('shell'), 'zzz')).toBe(0)
  })

  it('matches detail and keywords, not just the title', () => {
    expect(
      scoreEntry(
        entry('Open settings', 'action', { detail: 'preferences' }),
        'pref',
      ),
    ).toBeGreaterThan(0)
    expect(
      scoreEntry(
        entry('Open settings', 'action', { keywords: ['config'] }),
        'config',
      ),
    ).toBeGreaterThan(0)
  })

  it('ignores case on both sides', () => {
    expect(scoreEntry(entry('Shell'), 'SHELL')).toBe(100)
  })
})

describe('filterEntries', () => {
  const entries = [
    entry('shell', 'worker'),
    entry('shell::exec', 'function'),
    entry('New chat', 'action'),
  ]

  it('scopes to one kind without scoring', () => {
    expect(
      filterEntries(entries, '', 'worker').map((row) => row.title),
    ).toEqual(['shell'])
  })

  it('sorts a query best-first', () => {
    expect(
      filterEntries(entries, 'shell', 'all').map((row) => row.title),
    ).toEqual(['shell', 'shell::exec'])
  })

  it('treats a whitespace-only query as no query', () => {
    expect(filterEntries(entries, '   ', 'all')).toHaveLength(entries.length)
  })

  it('drops everything that does not match', () => {
    expect(filterEntries(entries, 'zzz', 'all')).toEqual([])
  })
})

describe('groupEntries', () => {
  it('emits groups in a fixed order, whatever the input order', () => {
    const grouped = groupEntries([
      entry('shell::exec', 'function'),
      entry('New chat', 'action'),
      entry('shell', 'worker'),
    ])
    expect(grouped.map(([kind]) => kind)).toEqual(
      KIND_ORDER.filter((kind) =>
        ['action', 'worker', 'function'].includes(kind),
      ),
    )
  })

  it('omits kinds with no rows', () => {
    const grouped = groupEntries([entry('shell', 'worker')])
    expect(grouped).toHaveLength(1)
    expect(grouped[0][0]).toBe('worker')
  })

  it('keeps every entry exactly once', () => {
    const input = [entry('a', 'page'), entry('b', 'page'), entry('c', 'chat')]
    expect(groupEntries(input).flatMap(([, rows]) => rows)).toHaveLength(
      input.length,
    )
  })
})
