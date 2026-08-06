import { describe, expect, it } from 'vitest'
import {
  defaultTabs,
  parseActiveTabId,
  parseWorkspaceTabs,
  resolveActiveTab,
  screenForView,
  screenLabel,
  tabColumns,
  tabLabel,
  tabSizes,
  type WorkspaceTab,
  withActiveTabId,
  withColumnAdded,
  withColumnRemoved,
  withScreenDetached,
  withWorkspaceTabs,
} from './workspace-tabs'

const NO_EXT = new Map<string, string>()

describe('parseWorkspaceTabs', () => {
  it('returns [] for an empty / malformed config', () => {
    expect(parseWorkspaceTabs({})).toEqual([])
    expect(parseWorkspaceTabs({ workspace: 'nope' })).toEqual([])
    expect(parseWorkspaceTabs({ workspace: { tabs: 'nope' } })).toEqual([])
  })

  it('keeps valid tabs (incl. empty and null columns) and drops malformed ones', () => {
    const tabs = parseWorkspaceTabs({
      workspace: {
        tabs: [
          { id: 't1', screens: ['traces'] },
          { id: 't2', screens: ['chat', 'workers'] },
          { id: 't3', screens: ['ext:my-page'] },
          { id: 't4', columns: 2, screens: [] },
          { id: 't5', columns: 2, screens: [null, 'traces'] },
          { id: 't6', columns: 3, screens: ['traces', 'chat', 'workers'] },
          { id: 't7', columns: 2, screens: ['chat', 'traces'], sizes: [1, 3] },
          { id: 'bad-screen', screens: ['nonsense'] },
          { id: 'bad-four', screens: ['traces', 'chat', 'workers', 'memory'] },
          { id: 'bad-columns', columns: 4, screens: ['traces'] },
          { id: 'bad-sizes', screens: ['traces'], sizes: [0, -1] },
          { screens: ['traces'] },
        ],
      },
    })
    expect(tabs.map((t) => t.id)).toEqual([
      't1',
      't2',
      't3',
      't4',
      't5',
      't6',
      't7',
    ])
  })

  it("migrates legacy 'configuration' screens to empty columns", () => {
    const tabs = parseWorkspaceTabs({
      workspace: {
        tabs: [{ id: 'cfg', columns: 2, screens: ['configuration', 'traces'] }],
      },
    })
    expect(tabs).toHaveLength(1)
    expect(tabs[0].screens).toEqual([null, 'traces'])
  })
})

describe('tabColumns', () => {
  it('prefers the explicit count, falls back to the screen count', () => {
    expect(tabColumns({ id: 't', columns: 2, screens: [] })).toBe(2)
    expect(
      tabColumns({ id: 't', columns: 1, screens: ['chat', 'traces'] }),
    ).toBe(1)
    expect(tabColumns({ id: 't', screens: ['chat', 'traces'] })).toBe(2)
    expect(tabColumns({ id: 't', screens: [] })).toBe(1)
  })
})

describe('active pointer round-trip', () => {
  it('writes and reads the pointer without clobbering siblings', () => {
    const value = withActiveTabId(
      { traces: { views: [] }, workspace: { tabs: defaultTabs() } },
      'tab-x',
    )
    expect(parseActiveTabId(value)).toBe('tab-x')
    expect(value.traces).toEqual({ views: [] })
    expect(parseWorkspaceTabs(value)).toEqual(defaultTabs())
  })

  it('withWorkspaceTabs preserves the active pointer', () => {
    const value = withWorkspaceTabs(withActiveTabId({}, 'tab-y'), defaultTabs())
    expect(parseActiveTabId(value)).toBe('tab-y')
  })
})

describe('screen mapping + labels', () => {
  it('maps views to screens; ext-without-id and configuration have none', () => {
    expect(screenForView('workers', null)).toBe('workers')
    expect(screenForView('ext', 'my-page')).toBe('ext:my-page')
    // The ext transient (view and page id land in separate commits) must
    // not resolve to a fallback screen — that used to spawn duplicate tabs.
    expect(screenForView('ext', null)).toBeNull()
    // Settings are an overlay page, not a tab screen.
    expect(screenForView('configuration', null)).toBeNull()
  })

  it('labels ext screens through the registry, falling back to the id', () => {
    const titles = new Map([['my-page', 'my page']])
    expect(screenLabel('ext:my-page', titles)).toBe('my page')
    expect(screenLabel('ext:ghost', titles)).toBe('ghost')
    expect(screenLabel('traces', titles)).toBe('traces')
  })

  it('tab labels join attached screens; empty tabs read "new tab"', () => {
    expect(tabLabel({ id: 't', screens: ['chat', 'traces'] }, NO_EXT)).toBe(
      'chat + traces',
    )
    expect(
      tabLabel({ id: 't', columns: 2, screens: [null, 'traces'] }, NO_EXT),
    ).toBe('traces')
    expect(tabLabel({ id: 't', columns: 2, screens: [] }, NO_EXT)).toBe(
      'new tab',
    )
    expect(
      tabLabel({ id: 't', name: 'my board', screens: ['traces'] }, NO_EXT),
    ).toBe('my board')
  })
})

describe('resolveActiveTab', () => {
  const chatTraces: WorkspaceTab = { id: 'home', screens: ['chat', 'traces'] }
  const solo: WorkspaceTab = { id: 'solo', screens: ['workers'] }
  const named: WorkspaceTab = {
    id: 'named',
    name: 'board',
    screens: ['chat', 'traces'],
  }

  it('follows a pointer at an existing tab', () => {
    expect(resolveActiveTab([solo, chatTraces], 'solo')).toBe(solo)
  })

  it('lands on the chat+traces tab when the pointer is missing or stale', () => {
    expect(resolveActiveTab([solo, chatTraces], undefined)).toBe(chatTraces)
    expect(resolveActiveTab([solo, chatTraces], 'closed')).toBe(chatTraces)
    expect(resolveActiveTab([solo, named], undefined)).toBe(named)
  })

  it('falls back to the first tab when no chat+traces tab exists', () => {
    expect(resolveActiveTab([solo], undefined)).toBe(solo)
  })
})

describe('tabSizes', () => {
  it('normalizes stored fractions and falls back to equal widths', () => {
    const tab: WorkspaceTab = {
      id: 't',
      columns: 2,
      screens: ['chat', 'traces'],
      sizes: [1, 3],
    }
    expect(tabSizes(tab)).toEqual([0.25, 0.75])
    // Mismatched length / junk → equal split.
    expect(tabSizes({ ...tab, sizes: [1] })).toEqual([0.5, 0.5])
    expect(tabSizes({ ...tab, sizes: undefined })).toEqual([0.5, 0.5])
  })
})

describe('withColumnAdded / withColumnRemoved', () => {
  const base: WorkspaceTab = { id: 't', columns: 1, screens: ['traces'] }

  it('adds an empty column on the chosen side at 1/(n+1) width', () => {
    const right = withColumnAdded(base, 'right')
    expect(right.columns).toBe(2)
    expect(right.screens).toEqual(['traces', null])
    expect(right.sizes).toEqual([0.5, 0.5])

    const left = withColumnAdded(base, 'left')
    expect(left.screens).toEqual([null, 'traces'])
  })

  it('keeps existing ratios inside the remaining space', () => {
    const twoCol: WorkspaceTab = {
      id: 't',
      columns: 2,
      screens: ['chat', 'traces'],
      sizes: [0.75, 0.25],
    }
    const three = withColumnAdded(twoCol, 'right')
    expect(three.sizes?.map((s) => Math.round(s * 100))).toEqual([50, 17, 33])
  })

  it('caps at MAX_COLUMNS and never removes the last column', () => {
    const three = withColumnAdded(withColumnAdded(base, 'right'), 'right')
    expect(tabColumns(three)).toBe(3)
    expect(withColumnAdded(three, 'right')).toBe(three)
    expect(withColumnRemoved(base, 0)).toBe(base)
  })

  it('removing a column re-normalizes the rest', () => {
    const three = withColumnAdded(withColumnAdded(base, 'right'), 'right')
    const two = withColumnRemoved(three, 2)
    expect(two.columns).toBe(2)
    expect(two.screens).toEqual(['traces', null])
    const total = (two.sizes ?? []).reduce((a, b) => a + b, 0)
    expect(total).toBeCloseTo(1)
  })

  it('round-trips through the validator (3 columns + sizes)', () => {
    const three = withColumnAdded(withColumnAdded(base, 'right'), 'left')
    const parsed = parseWorkspaceTabs({ workspace: { tabs: [three] } })
    expect(parsed).toHaveLength(1)
    expect(tabColumns(parsed[0])).toBe(3)
  })
})

describe('withScreenDetached', () => {
  const base: WorkspaceTab = { id: 't', columns: 1, screens: ['traces'] }

  it('blanks the column but keeps it (and the column count)', () => {
    const detached = withScreenDetached(base, 0)
    expect(detached.columns).toBe(1)
    expect(detached.screens).toEqual([null])
  })

  it('only touches the addressed column', () => {
    const two: WorkspaceTab = {
      id: 't',
      columns: 2,
      screens: ['chat', 'traces'],
    }
    expect(withScreenDetached(two, 1).screens).toEqual(['chat', null])
  })

  it('is a no-op for empty columns and out-of-range indexes', () => {
    const empty: WorkspaceTab = { id: 't', columns: 1, screens: [null] }
    expect(withScreenDetached(empty, 0)).toBe(empty)
    expect(withScreenDetached(base, 1)).toBe(base)
    expect(withScreenDetached(base, -1)).toBe(base)
  })

  it('round-trips through the validator', () => {
    const detached = withScreenDetached(base, 0)
    const parsed = parseWorkspaceTabs({ workspace: { tabs: [detached] } })
    expect(parsed).toHaveLength(1)
    expect(parsed[0].screens).toEqual([null])
  })
})
