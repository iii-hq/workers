import { describe, expect, it } from 'vitest'
import fixtures from './workspace-open.fixtures.json'
import {
  CHAT_SCREEN,
  defaultTabs,
  MAX_COLUMNS,
  parseActiveTabId,
  parseWorkspaceTabs,
  resolveActiveTab,
  screenForView,
  screenLabel,
  shouldFlushPendingWrite,
  tabColumns,
  tabLabel,
  tabPaneIds,
  tabSizes,
  type WorkspaceTab,
  withActiveTabId,
  withColumnAdded,
  withColumnRemoved,
  withPaneRemoved,
  withScreenDetached,
  withScreenOpenedBeside,
  withWorkspaceScreenOpened,
  withWorkspaceTabs,
  workspaceLayoutSource,
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
          {
            id: 'bad-too-many-screens',
            screens: Array.from({ length: MAX_COLUMNS + 1 }, () => 'traces'),
          },
          {
            id: 'bad-columns',
            columns: MAX_COLUMNS + 1,
            screens: ['traces'],
          },
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

  it('rewrites migrated per-worker screens to their ext form', () => {
    const tabs = parseWorkspaceTabs({
      workspace: {
        tabs: [
          { id: 'mig', columns: 2, screens: ['memory', 'worktrees'] },
          { id: 'mig2', columns: 2, screens: ['browser', 'github'] },
        ],
      },
    })
    expect(tabs).toHaveLength(2)
    expect(tabs[0].screens).toEqual(['ext:memory', 'ext:worktree'])
    expect(tabs[1].screens).toEqual(['ext:browser', 'ext:github'])
  })

  it('round-trips valid pane ids and strips only malformed identity metadata', () => {
    const tabs = parseWorkspaceTabs({
      workspace: {
        tabs: [
          {
            id: 'stable',
            columns: 2,
            screens: ['chat', 'traces'],
            paneIds: ['pane-chat', 'pane-traces'],
          },
          {
            id: 'legacy',
            columns: 2,
            screens: ['chat', 'traces'],
            paneIds: ['pane-chat', ''],
          },
          {
            id: 'wrong-shape',
            columns: 1,
            screens: ['chat'],
            paneIds: 'legacy-value',
          },
          {
            id: 'duplicate',
            columns: 2,
            screens: ['chat', 'traces'],
            paneIds: ['same', 'same'],
          },
          {
            id: 'misaligned',
            columns: 2,
            screens: ['chat', 'traces'],
            paneIds: ['only-one'],
          },
        ],
      },
    })

    expect(tabs).toHaveLength(5)
    expect(tabs[0].paneIds).toEqual(['pane-chat', 'pane-traces'])
    for (const tab of tabs.slice(1)) expect(tab).not.toHaveProperty('paneIds')
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

describe('tabPaneIds', () => {
  it('uses stored identities and supplies deterministic ids for legacy tabs', () => {
    expect(
      tabPaneIds({
        id: 'stable',
        columns: 2,
        screens: ['chat', 'traces'],
        paneIds: ['pane-chat', 'pane-traces'],
      }),
    ).toEqual(['pane-chat', 'pane-traces'])
    expect(
      tabPaneIds({
        id: 'legacy',
        columns: 2,
        screens: ['chat', 'traces'],
      }),
    ).toEqual(['legacy:pane:0', 'legacy:pane:1'])
  })

  it('falls back when stored identities are duplicated or misaligned', () => {
    expect(
      tabPaneIds({
        id: 'legacy',
        columns: 2,
        screens: ['chat', 'traces'],
        paneIds: ['duplicate', 'duplicate'],
      }),
    ).toEqual(['legacy:pane:0', 'legacy:pane:1'])
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

  it('preserves pane identities across insertion and removal', () => {
    const stable: WorkspaceTab = {
      id: 'stable',
      columns: 2,
      screens: ['chat', null],
      paneIds: ['pane-chat', 'pane-empty'],
    }
    const inserted = withColumnAdded(stable, 'left')
    expect(inserted.paneIds?.slice(1)).toEqual(['pane-chat', 'pane-empty'])
    expect(new Set(inserted.paneIds).size).toBe(3)

    const removed = withColumnRemoved(inserted, 1)
    expect(removed.paneIds).toEqual([inserted.paneIds?.[0], 'pane-empty'])

    const removedAfterMove = withPaneRemoved(inserted, 'pane-empty')
    expect(removedAfterMove.paneIds).not.toContain('pane-empty')
    expect(withPaneRemoved(inserted, 'missing-pane')).toBe(inserted)
  })

  it('captures deterministic legacy identities on the first structural edit', () => {
    const legacy: WorkspaceTab = {
      id: 'legacy',
      columns: 2,
      screens: ['chat', 'traces'],
    }
    const inserted = withColumnAdded(legacy, 'left')

    expect(inserted.paneIds?.slice(1)).toEqual([
      'legacy:pane:0',
      'legacy:pane:1',
    ])
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

  it('grows beyond three panels, caps at the safety ceiling, and never removes the last column', () => {
    let many = base
    for (let index = 1; index < MAX_COLUMNS; index += 1) {
      many = withColumnAdded(many, 'right')
    }
    expect(tabColumns(many)).toBe(MAX_COLUMNS)
    expect(many.screens).toHaveLength(MAX_COLUMNS)
    expect(withColumnAdded(many, 'right')).toBe(many)
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

  it('round-trips through the validator with more than three columns', () => {
    const four = withColumnAdded(
      withColumnAdded(withColumnAdded(base, 'right'), 'left'),
      'right',
    )
    const parsed = parseWorkspaceTabs({ workspace: { tabs: [four] } })
    expect(parsed).toHaveLength(1)
    expect(tabColumns(parsed[0])).toBe(4)
  })
})

describe('withScreenOpenedBeside', () => {
  it('uses the empty column beside chat before growing the split', () => {
    const tab: WorkspaceTab = {
      id: 'a',
      columns: 2,
      screens: [CHAT_SCREEN, null],
    }
    expect(withScreenOpenedBeside(tab, 'ext:shell')?.screens).toEqual([
      CHAT_SCREEN,
      'ext:shell',
    ])
  })

  it('inserts beside chat without replacing another panel', () => {
    const tab: WorkspaceTab = {
      id: 'a',
      columns: 2,
      screens: [CHAT_SCREEN, 'traces'],
      paneIds: ['pane-chat', 'pane-traces'],
    }
    const next = withScreenOpenedBeside(tab, 'ext:shell')
    expect(next?.screens).toEqual([CHAT_SCREEN, 'ext:shell', 'traces'])
    expect(next?.paneIds?.[0]).toBe('pane-chat')
    expect(next?.paneIds?.[2]).toBe('pane-traces')
    expect(next?.sizes?.reduce((sum, value) => sum + value, 0)).toBeCloseTo(1)
  })

  it('reuses an existing screen and refuses to overwrite a full tab', () => {
    const existing: WorkspaceTab = {
      id: 'a',
      columns: 2,
      screens: [CHAT_SCREEN, 'ext:shell'],
    }
    expect(withScreenOpenedBeside(existing, 'ext:shell')).toBe(existing)

    const fullScreens = [
      CHAT_SCREEN,
      ...Array.from(
        { length: MAX_COLUMNS - 1 },
        (_, index) => `ext:full-${index}`,
      ),
    ]
    const full: WorkspaceTab = {
      id: 'b',
      columns: MAX_COLUMNS,
      screens: fullScreens,
    }
    expect(withScreenOpenedBeside(full, 'ext:shell')).toBeNull()
  })
})

describe('withWorkspaceScreenOpened', () => {
  it('activates an already-open panel without duplicating it', () => {
    const tabs: WorkspaceTab[] = [
      { id: 'chat', screens: [CHAT_SCREEN, 'traces'] },
      { id: 'shell', screens: ['ext:shell'] },
    ]
    expect(
      withWorkspaceScreenOpened(tabs, 'chat', 'ext:shell', () => 'new'),
    ).toEqual({
      tabs,
      activeTabId: 'shell',
    })
  })

  it('stays put when the workspace you are on already shows that screen', () => {
    const tabs: WorkspaceTab[] = [
      { id: 'first-chat', screens: [CHAT_SCREEN] },
      { id: 'chat-and-shell', screens: [CHAT_SCREEN, 'ext:shell'] },
    ]
    // Opening chat from the second tab must not send you to the first one
    // just because it was created earlier.
    expect(
      withWorkspaceScreenOpened(
        tabs,
        'chat-and-shell',
        CHAT_SCREEN,
        () => 'new',
      ),
    ).toEqual({ tabs, activeTabId: 'chat-and-shell' })
  })

  it('places beside chat, then creates a safe split when the active tab is full', () => {
    const open = withWorkspaceScreenOpened(
      [{ id: 'chat', columns: 2, screens: [CHAT_SCREEN, 'traces'] }],
      'chat',
      'ext:shell',
      () => 'new',
    )
    expect(open.tabs[0].screens).toEqual([CHAT_SCREEN, 'ext:shell', 'traces'])

    const fullScreens = [
      CHAT_SCREEN,
      ...Array.from(
        { length: MAX_COLUMNS - 1 },
        (_, index) => `ext:full-${index}`,
      ),
    ]
    const full = withWorkspaceScreenOpened(
      [
        {
          id: 'full',
          columns: MAX_COLUMNS,
          screens: fullScreens,
        },
      ],
      'full',
      'ext:shell',
      () => 'new',
    )
    expect(full.activeTabId).toBe('new')
    expect(full.tabs[1]).toEqual({
      id: 'new',
      columns: 2,
      screens: [CHAT_SCREEN, 'ext:shell'],
    })
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

describe('console::workspace fixtures (shared with the Rust worker)', () => {
  for (const f of fixtures.open) {
    it(`open: ${f.name}`, () => {
      const result = withWorkspaceScreenOpened(
        f.tabs as WorkspaceTab[],
        f.activeTabId,
        f.screen,
        () => 'tab-new',
        () => 'pane-new',
      )
      expect({ tabs: result.tabs, activeTabId: result.activeTabId }).toEqual(
        f.expect,
      )
    })
  }

  for (const f of fixtures.close) {
    it(`close: ${f.name}`, () => {
      const tabIds: string[] = []
      const tabs = (f.tabs as WorkspaceTab[]).map((tab) => {
        const column = tab.screens.indexOf(f.screen)
        if (column < 0) return tab
        tabIds.push(tab.id)
        return tabColumns(tab) > 1
          ? withColumnRemoved(tab, column)
          : withScreenDetached(tab, column)
      })
      expect({ tabIds, tabs }).toEqual(f.expect)
    })
  }
})

describe('workspaceLayoutSource', () => {
  it('is pending until the first answer, then server or local', () => {
    expect(workspaceLayoutSource(false, false)).toBe('pending')
    expect(workspaceLayoutSource(true, false)).toBe('local')
    expect(workspaceLayoutSource(true, true)).toBe('server')
  })

  it('flips from local to server when a later poll succeeds', () => {
    const timeline = [
      [true, false],
      [true, false],
      [true, true],
    ] as const
    expect(timeline.map(([f, a]) => workspaceLayoutSource(f, a))).toEqual([
      'local',
      'local',
      'server',
    ])
  })
})

describe('shouldFlushPendingWrite', () => {
  it('flushes only once hydrated and something is queued', () => {
    expect(shouldFlushPendingWrite('pending', true)).toBe(false)
    expect(shouldFlushPendingWrite('pending', false)).toBe(false)
    expect(shouldFlushPendingWrite('server', false)).toBe(false)
    expect(shouldFlushPendingWrite('server', true)).toBe(true)
    expect(shouldFlushPendingWrite('local', true)).toBe(true)
  })
})
