import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getPaletteSources,
  registerPaletteSource,
  resetPaletteSources,
  searchPaletteSources,
} from './providers'
import { loadRecents, recentOf, recordRecent } from './recents'
import { DEFAULT_KINDS, groupEntries, parseQuery, scoreEntry } from './sources'

const noop = () => {}

describe('parseQuery', () => {
  it('reads a prefix as a mode and strips it from the text', () => {
    expect(parseQuery('#main', 'all')).toEqual({
      prefix: '#',
      kinds: new Set(['file']),
      text: 'main',
    })
    expect(parseQuery('> open', 'all')).toEqual({
      prefix: '>',
      kinds: new Set(['command', 'action']),
      text: 'open',
    })
    expect(parseQuery('/stop', 'chat').kinds).toEqual(
      new Set(['command', 'action']),
    )
    expect(parseQuery('@standup', 'all').kinds).toEqual(new Set(['chat']))
  })

  it('otherwise searches navigation, or the chip filter', () => {
    expect(parseQuery(' open file ', 'all')).toEqual({
      prefix: null,
      kinds: new Set(DEFAULT_KINDS),
      text: 'open file',
    })
    expect(parseQuery('x', 'worker').kinds).toEqual(new Set(['worker']))
  })
})

describe('scoreEntry with several words', () => {
  const entry = {
    id: 'c',
    kind: 'command' as const,
    title: 'Shell: Open file…',
    detail: 'Find a file by name',
    run: noop,
  }
  it('matches words in any order and needs every word', () => {
    expect(scoreEntry(entry, 'open file')).toBeGreaterThan(0)
    expect(scoreEntry(entry, 'file open')).toBeGreaterThan(0)
    expect(scoreEntry(entry, 'file shell')).toBeGreaterThan(0)
    expect(scoreEntry(entry, 'open nothing')).toBe(0)
  })
})

describe('palette sources', () => {
  beforeEach(() => {
    resetPaletteSources()
    vi.spyOn(console, 'warn').mockImplementation(noop)
  })
  afterEach(() => vi.restoreAllMocks())

  const files = (rows: string[]) => ({
    id: 'files',
    title: 'Files',
    kind: 'file' as const,
    prefix: '#',
    minQuery: 3,
    search: async (query: string) =>
      rows
        .filter((row) => row.includes(query))
        .map((row) => ({ id: row, title: row, run: noop })),
  })

  it('exists only while registered and is keyed by scope', () => {
    const off = registerPaletteSource('shell', files([]))
    expect(getPaletteSources().map((s) => s.key)).toEqual(['shell.files'])
    off()
    expect(getPaletteSources()).toEqual([])
  })

  it('answers with its prefix at any length, otherwise from minQuery', async () => {
    registerPaletteSource('shell', files(['src/main.rs', 'README.md']))
    const sources = getPaletteSources()
    const ask = (text: string, prefix: string | null) =>
      searchPaletteSources(sources, {
        text,
        prefix,
        kinds: prefix === '#' ? new Set(['file']) : null,
        workingDir: null,
        conversationId: null,
        signal: new AbortController().signal,
      })
    expect(await ask('ma', null)).toEqual([])
    expect((await ask('mai', null)).map((e) => e.title)).toEqual([
      'src/main.rs',
    ])
    expect((await ask('m', '#')).map((e) => e.title)).toEqual([
      'src/main.rs',
      'README.md',
    ])
  })

  it("reaches a source through its own prefix even outside the mode's kinds", async () => {
    registerPaletteSource('database', {
      id: 'tables',
      title: 'Tables',
      kind: 'item',
      prefix: '#',
      search: async () => [{ id: 't', title: 'users', run: () => {} }],
    })
    const rows = await searchPaletteSources(getPaletteSources(), {
      text: 'us',
      prefix: '#',
      kinds: new Set(['file']),
      workingDir: null,
      conversationId: null,
      signal: new AbortController().signal,
    })
    expect(rows.map((entry) => entry.title)).toEqual(['users'])
  })

  it('skips a source outside the mode and one that throws', async () => {
    registerPaletteSource('shell', files(['src/main.rs']))
    registerPaletteSource('other', {
      id: 'broken',
      title: 'Broken',
      kind: 'item',
      search: async () => {
        throw new Error('no')
      },
    })
    const rows = await searchPaletteSources(getPaletteSources(), {
      text: 'main',
      prefix: '>',
      kinds: new Set(['command', 'action']),
      workingDir: null,
      conversationId: null,
      signal: new AbortController().signal,
    })
    expect(rows).toEqual([])
    const all = await searchPaletteSources(getPaletteSources(), {
      text: 'main',
      prefix: null,
      kinds: null,
      workingDir: null,
      conversationId: null,
      signal: new AbortController().signal,
    })
    expect(all.map((e) => e.id)).toEqual(['source:shell.files:src/main.rs'])
  })

  it('drops an answer the palette has moved past', async () => {
    registerPaletteSource('shell', files(['src/main.rs']))
    const controller = new AbortController()
    const pending = searchPaletteSources(getPaletteSources(), {
      text: 'main',
      prefix: null,
      kinds: null,
      workingDir: null,
      conversationId: null,
      signal: controller.signal,
    })
    controller.abort()
    expect(await pending).toEqual([])
  })
})

describe('recents', () => {
  beforeEach(() => {
    const store = new Map<string, string>()
    vi.stubGlobal('window', {
      localStorage: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => {
          store.set(key, value)
        },
      },
    })
  })
  afterEach(() => vi.unstubAllGlobals())

  it('keeps the newest ten, most recent first, and leads the empty query', () => {
    for (let i = 0; i < 12; i += 1) recordRecent(`id-${i}`, i)
    const recents = loadRecents()
    expect(recents).toHaveLength(10)
    expect(recents[0].id).toBe('id-11')
    recordRecent('id-3', 99)
    expect(loadRecents()[0].id).toBe('id-3')

    const entries = [
      { id: 'id-3', kind: 'action' as const, title: 'three', run: noop },
      { id: 'id-11', kind: 'page' as const, title: 'eleven', run: noop },
      { id: 'other', kind: 'page' as const, title: 'other', run: noop },
    ]
    const recent = recentOf(entries, loadRecents())
    expect(recent.map((e) => e.id)).toEqual(['id-3', 'id-11'])
    const groups = groupEntries(entries, recent)
    expect(groups.map(([group, rows]) => [group, rows.length])).toEqual([
      ['recent', 2],
      ['page', 1],
    ])
  })
})
