import { describe, expect, it } from 'vitest'
import {
  fuzzyMatchIndices,
  highlightSegments,
  isQuickOpenKey,
  normalizeQuickOpenQuery,
  QUICK_OPEN_SHORTCUT,
  quickOpenKeyLabel,
  quickOpenRow,
  stepQuickOpenIndex,
  toQuickOpenRows,
} from '../quick-open'

const key = (overrides: Partial<Parameters<typeof isQuickOpenKey>[0]>) => ({
  key: 'p',
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...overrides,
})

describe('quick-open shortcut', () => {
  it('is Ctrl+P on a Mac and Alt+P elsewhere, never the browser print chord', () => {
    expect(QUICK_OPEN_SHORTCUT).toEqual({ mac: ['Ctrl+P'], other: ['Alt+P'] })
    expect(quickOpenKeyLabel('mac')).toBe('Ctrl+P')
    expect(quickOpenKeyLabel('other')).toBe('Alt+P')
  })

  it('matches the chord for the platform only', () => {
    expect(isQuickOpenKey(key({ ctrlKey: true }), 'mac')).toBe(true)
    expect(isQuickOpenKey(key({ ctrlKey: true, key: 'P' }), 'mac')).toBe(true)
    expect(isQuickOpenKey(key({ metaKey: true }), 'mac')).toBe(false)
    expect(isQuickOpenKey(key({ ctrlKey: true, shiftKey: true }), 'mac')).toBe(false)
    expect(isQuickOpenKey(key({ altKey: true }), 'mac')).toBe(false)
    expect(isQuickOpenKey(key({ altKey: true }), 'other')).toBe(true)
    expect(isQuickOpenKey(key({ ctrlKey: true }), 'other')).toBe(false)
    expect(isQuickOpenKey(key({ ctrlKey: true, key: 'o' }), 'mac')).toBe(false)
  })
})

describe('normalizeQuickOpenQuery', () => {
  it('drops whitespace and lowercases', () => {
    expect(normalizeQuickOpenQuery('  Files Tab ')).toBe('filestab')
    expect(normalizeQuickOpenQuery('')).toBe('')
  })
})

describe('fuzzyMatchIndices', () => {
  it('prefers the query inside the file name', () => {
    expect(fuzzyMatchIndices('index', 'shell/ui/src/page/index.tsx')).toEqual([18, 19, 20, 21, 22])
  })

  it('then the query anywhere in the path', () => {
    expect(fuzzyMatchIndices('ui/src', 'shell/ui/src/page/index.tsx')).toEqual([6, 7, 8, 9, 10, 11])
  })

  it('then the first subsequence, the way the worker scores it', () => {
    expect(fuzzyMatchIndices('ptofolder', 'path/to/folder')).toEqual([0, 2, 6, 8, 9, 10, 11, 12, 13])
  })

  it('ignores case and whitespace in the query', () => {
    expect(fuzzyMatchIndices('Files Tab', 'ui/src/page/FilesTab.tsx')).toEqual([12, 13, 14, 15, 16, 17, 18, 19])
  })

  it('is empty for a listing and null when nothing matches', () => {
    expect(fuzzyMatchIndices('', 'a/b.ts')).toEqual([])
    expect(fuzzyMatchIndices('zzz', 'a/b.ts')).toBeNull()
  })

  it('counts code points so non-BMP names stay aligned', () => {
    expect(fuzzyMatchIndices('b', '\u{1F600}/b.ts')).toEqual([2])
  })
})

describe('highlightSegments', () => {
  it('merges adjacent hits into runs relative to an offset', () => {
    const indices = fuzzyMatchIndices('ptofolder', 'path/to/folder')
    expect(highlightSegments('folder', indices, 8)).toEqual([{ text: 'folder', hit: true }])
    expect(highlightSegments('path/to', indices, 0)).toEqual([
      { text: 'p', hit: true },
      { text: 'a', hit: false },
      { text: 't', hit: true },
      { text: 'h/t', hit: false },
      { text: 'o', hit: true },
    ])
  })

  it('is one plain run without indices', () => {
    expect(highlightSegments('abc', null)).toEqual([{ text: 'abc', hit: false }])
    expect(highlightSegments('', [0])).toEqual([])
  })
})

describe('rows', () => {
  it('splits name and folder and knows where the name starts', () => {
    const row = quickOpenRow('path/to/folder', 'ptofolder')
    expect(row).toMatchObject({ name: 'folder', dir: 'path/to', nameOffset: 8 })
    expect(quickOpenRow('README.md', 're')).toMatchObject({ dir: '', nameOffset: 0 })
  })

  it('keeps files only, root-relative, deduplicated, up to the limit', () => {
    const rows = toQuickOpenRows(
      '/repo',
      [
        { path: '/repo/src', kind: 'dir' },
        { path: '/repo/src/a.ts', kind: 'file' },
        { path: '/repo/src/a.ts', kind: 'file' },
        { path: '/repo/b.ts' },
        { path: '/repo/c.ts' },
      ],
      'ts',
      2,
    )
    expect(rows.map((row) => row.rel)).toEqual(['src/a.ts', 'b.ts'])
  })
})

describe('stepQuickOpenIndex', () => {
  it('wraps and enters from nothing at either end', () => {
    expect(stepQuickOpenIndex(-1, 1, 3)).toBe(0)
    expect(stepQuickOpenIndex(-1, -1, 3)).toBe(2)
    expect(stepQuickOpenIndex(2, 1, 3)).toBe(0)
    expect(stepQuickOpenIndex(0, -1, 3)).toBe(2)
    expect(stepQuickOpenIndex(0, 1, 0)).toBe(-1)
  })
})
