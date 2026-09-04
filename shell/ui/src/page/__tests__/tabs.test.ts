import { describe, expect, it } from 'vitest'
import {
  activateTab,
  basename,
  closeTab,
  cycleTab,
  diffTarget,
  EMPTY_TABS,
  fileTarget,
  lastSegments,
  openPinned,
  openPreview,
  persistedTabs,
  pinTab,
  restoreTabs,
  tabIdFor,
  tabsForPath,
} from '../tabs'

const a = fileTarget('a.ts')
const b = fileTarget('b.ts')
const aStaged = diffTarget('a.ts', { type: 'staged' })
const aUnstaged = diffTarget('a.ts', { type: 'unstaged' })

describe('tab ids', () => {
  it('derive from kind, source and path', () => {
    expect(tabIdFor(a)).toBe('file:a.ts')
    expect(tabIdFor(aStaged)).toBe('diff:staged:a.ts')
    expect(tabIdFor(diffTarget('x', { type: 'turn', turnId: 't1' }))).toBe('diff:turn=t1:x')
    expect(tabIdFor(diffTarget('x', { type: 'compare', ref: 'refs/heads/main' }))).toBe('diff:compare=refs/heads/main:x')
  })
})

describe('openPreview', () => {
  it('opens the first target as the single preview tab', () => {
    const s = openPreview(EMPTY_TABS, a)
    expect(s).toEqual({ tabs: [{ id: 'file:a.ts', target: a, pinned: false }], active: 'file:a.ts' })
  })

  it('the next preview REPLACES the current one in place', () => {
    let s = openPreview(EMPTY_TABS, a)
    s = openPreview(s, b)
    expect(s.tabs.map((t) => t.id)).toEqual(['file:b.ts'])
    expect(s.active).toBe('file:b.ts')
  })

  it('never touches pinned tabs — the preview slots in beside them', () => {
    let s = openPinned(EMPTY_TABS, fileTarget('pinned.ts'))
    s = openPreview(s, a)
    s = openPreview(s, b)
    expect(s.tabs.map((t) => [t.id, t.pinned])).toEqual([
      ['file:pinned.ts', true],
      ['file:b.ts', false],
    ])
  })

  it('a file and its diffs are distinct tabs of the same path', () => {
    let s = openPinned(EMPTY_TABS, a)
    s = openPinned(s, aStaged)
    s = openPreview(s, aUnstaged)
    expect(s.tabs.map((t) => t.id)).toEqual(['file:a.ts', 'diff:staged:a.ts', 'diff:unstaged:a.ts'])
    expect(tabsForPath(s, 'a.ts')).toHaveLength(3)
    // Re-previewing an already-open diff only activates it.
    s = openPreview(s, aStaged)
    expect(s.active).toBe('diff:staged:a.ts')
    expect(s.tabs).toHaveLength(3)
  })
})

describe('openPinned / pinTab', () => {
  it('double-click promotes the existing preview tab in place', () => {
    let s = openPreview(EMPTY_TABS, a)
    s = openPinned(s, a)
    expect(s.tabs).toEqual([{ id: 'file:a.ts', target: a, pinned: true }])
  })

  it('pinTab is a no-op for already-pinned or unknown ids', () => {
    const s = openPinned(EMPTY_TABS, a)
    expect(pinTab(s, 'file:a.ts')).toBe(s)
    expect(pinTab(s, 'file:zzz')).toBe(s)
  })
})

describe('closeTab / activateTab / cycleTab', () => {
  it('activates the right neighbor, else the left, else nothing', () => {
    let s = openPinned(EMPTY_TABS, a)
    s = openPinned(s, b)
    s = openPinned(s, aStaged)
    s = activateTab(s, 'file:b.ts')
    s = closeTab(s, 'file:b.ts')
    expect(s.active).toBe('diff:staged:a.ts')
    s = closeTab(s, 'diff:staged:a.ts')
    expect(s.active).toBe('file:a.ts')
    s = closeTab(s, 'file:a.ts')
    expect(s).toEqual(EMPTY_TABS)
  })

  it('cycles with wrap-around', () => {
    let s = openPinned(EMPTY_TABS, a)
    s = openPinned(s, b)
    expect(cycleTab(s, 1).active).toBe('file:a.ts')
    expect(cycleTab(s, -1).active).toBe('file:a.ts')
    expect(cycleTab(EMPTY_TABS, 1)).toBe(EMPTY_TABS)
  })
})

describe('restoreTabs / persistedTabs', () => {
  it('rebuilds from persisted JSON, dropping junk and change tabs', () => {
    const open = [
      { kind: 'file', path: 'a.ts', pinned: true },
      { kind: 'diff', path: 'a.ts', source: { type: 'staged' }, pinned: true },
      { kind: 'diff', path: 'a.ts', source: { type: 'turn', turnId: 't1' }, pinned: false },
      { kind: 'diff', path: 'a.ts', source: { type: 'bogus' }, pinned: true },
      { path: '', pinned: true },
      42,
    ]
    const s = restoreTabs(open, 'diff:staged:a.ts')
    expect(s.tabs.map((t) => t.id)).toEqual(['file:a.ts', 'diff:staged:a.ts', 'diff:turn=t1:a.ts'])
    expect(s.active).toBe('diff:staged:a.ts')
    const withChange = openPinned(s, diffTarget('b.ts', { type: 'change', changeId: 'c1' }))
    expect(persistedTabs(withChange).map((t) => t.path)).toEqual(['a.ts', 'a.ts', 'a.ts'])
  })

  it('reads the pre-diff shape as file tabs and a path as the active id', () => {
    const s = restoreTabs([{ path: 'a.ts', pinned: true }, { path: 'b.ts', pinned: false }], 'b.ts')
    expect(s.tabs.map((t) => t.id)).toEqual(['file:a.ts', 'file:b.ts'])
    expect(s.active).toBe('file:b.ts')
  })

  it('falls back to the first tab when active is stale', () => {
    expect(restoreTabs([{ path: 'a.ts', pinned: true }], 'nope').active).toBe('file:a.ts')
    expect(restoreTabs(null, undefined)).toEqual(EMPTY_TABS)
  })
})

describe('lastSegments / basename', () => {
  it('shows the last two folder names of a root', () => {
    expect(lastSegments('/Users/x/proj/src')).toBe('proj/src')
    expect(lastSegments('/')).toBe('/')
  })

  it('basename strips the dirname', () => {
    expect(basename('a/b/c.ts')).toBe('c.ts')
    expect(basename('c.ts')).toBe('c.ts')
  })
})
