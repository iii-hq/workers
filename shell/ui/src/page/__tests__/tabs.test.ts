import { describe, expect, it } from 'vitest'
import {
  activateTab,
  basename,
  closeTab,
  EMPTY_TABS,
  lastSegments,
  openPinned,
  openPreview,
  pinTab,
  restoreTabs,
} from '../tabs'

describe('openPreview', () => {
  it('opens the first file as the single preview tab', () => {
    const s = openPreview(EMPTY_TABS, 'a.ts')
    expect(s).toEqual({ tabs: [{ path: 'a.ts', pinned: false }], active: 'a.ts' })
  })

  it('the next preview REPLACES the current one in place', () => {
    let s = openPreview(EMPTY_TABS, 'a.ts')
    s = openPreview(s, 'b.ts')
    expect(s.tabs).toEqual([{ path: 'b.ts', pinned: false }])
    expect(s.active).toBe('b.ts')
  })

  it('never touches pinned tabs — the preview slots in beside them', () => {
    let s = openPinned(EMPTY_TABS, 'pinned.ts')
    s = openPreview(s, 'a.ts')
    s = openPreview(s, 'b.ts')
    expect(s.tabs).toEqual([
      { path: 'pinned.ts', pinned: true },
      { path: 'b.ts', pinned: false },
    ])
  })

  it('re-previewing an already-open tab only activates it', () => {
    let s = openPinned(EMPTY_TABS, 'a.ts')
    s = openPreview(s, 'b.ts')
    s = openPreview(s, 'a.ts')
    expect(s.active).toBe('a.ts')
    // b.ts stays — a.ts was already open, so no preview was spent.
    expect(s.tabs).toHaveLength(2)
    expect(s.tabs[0]).toEqual({ path: 'a.ts', pinned: true })
  })
})

describe('openPinned / pinTab', () => {
  it('double-click promotes the existing preview tab in place', () => {
    let s = openPreview(EMPTY_TABS, 'a.ts')
    s = openPinned(s, 'a.ts')
    expect(s.tabs).toEqual([{ path: 'a.ts', pinned: true }])
  })

  it('opens unseen files directly as pinned', () => {
    const s = openPinned(EMPTY_TABS, 'a.ts')
    expect(s.tabs).toEqual([{ path: 'a.ts', pinned: true }])
  })

  it('pinTab is a no-op for already-pinned or unknown paths', () => {
    const s = openPinned(EMPTY_TABS, 'a.ts')
    expect(pinTab(s, 'a.ts')).toBe(s)
    expect(pinTab(s, 'nope.ts')).toBe(s)
  })
})

describe('closeTab', () => {
  it('activates the right neighbor, else the left, else nothing', () => {
    let s = openPinned(EMPTY_TABS, 'a.ts')
    s = openPinned(s, 'b.ts')
    s = openPinned(s, 'c.ts')
    s = activateTab(s, 'b.ts')

    s = closeTab(s, 'b.ts')
    expect(s.active).toBe('c.ts')
    s = closeTab(s, 'c.ts')
    expect(s.active).toBe('a.ts')
    s = closeTab(s, 'a.ts')
    expect(s).toEqual(EMPTY_TABS)
  })

  it('closing an inactive tab keeps the active one', () => {
    let s = openPinned(EMPTY_TABS, 'a.ts')
    s = openPinned(s, 'b.ts')
    s = closeTab(s, 'a.ts')
    expect(s.active).toBe('b.ts')
  })
})

describe('restoreTabs', () => {
  it('rebuilds from persisted JSON, dropping junk entries', () => {
    const s = restoreTabs(
      [
        { path: 'a.ts', pinned: true },
        { path: 'b.ts', pinned: false },
        { path: 'a.ts', pinned: false }, // duplicate
        { pinned: true }, // no path
        'nope', // not an object
      ],
      'b.ts',
    )
    expect(s.tabs).toEqual([
      { path: 'a.ts', pinned: true },
      { path: 'b.ts', pinned: false },
    ])
    expect(s.active).toBe('b.ts')
  })

  it('falls back to the first tab when active is stale', () => {
    const s = restoreTabs([{ path: 'a.ts', pinned: true }], 'gone.ts')
    expect(s.active).toBe('a.ts')
  })

  it('handles empty/garbage payloads', () => {
    expect(restoreTabs(undefined, undefined)).toEqual(EMPTY_TABS)
    expect(restoreTabs('junk', 42)).toEqual(EMPTY_TABS)
  })
})

describe('lastSegments / basename', () => {
  it('shows the last two folder names of a root', () => {
    expect(lastSegments('/Users/x/workspaces/iii/workers')).toBe('iii/workers')
    expect(lastSegments('/workers')).toBe('workers')
    expect(lastSegments('/')).toBe('/')
  })

  it('basename strips the dirname', () => {
    expect(basename('a/b/c.ts')).toBe('c.ts')
    expect(basename('c.ts')).toBe('c.ts')
  })
})
