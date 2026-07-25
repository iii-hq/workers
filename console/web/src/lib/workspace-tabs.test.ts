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
  type WorkspaceTab,
  withActiveTabId,
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
          { id: 'bad-screen', screens: ['nonsense'] },
          { id: 'bad-three', screens: ['traces', 'chat', 'workers'] },
          { id: 'bad-columns', columns: 3, screens: ['traces'] },
          { screens: ['traces'] },
        ],
      },
    })
    expect(tabs.map((t) => t.id)).toEqual(['t1', 't2', 't3', 't4', 't5'])
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
  it('maps views to screens (ext needs a page id)', () => {
    expect(screenForView('workers', null)).toBe('workers')
    expect(screenForView('ext', 'my-page')).toBe('ext:my-page')
    expect(screenForView('ext', null)).toBe('traces')
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
