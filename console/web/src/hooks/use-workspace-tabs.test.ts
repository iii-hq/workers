import { describe, expect, it } from 'vitest'
import type { ConsoleConfigValue } from '@/lib/console-config'
import {
  parseActivation,
  parseActiveTabId,
  parseWorkspaceTabs,
  tabPaneIds,
  withTabClosed,
} from '@/lib/workspace-tabs'
import { SerializedConfigWriter } from './lib/serialized-config-writer'
import {
  type WorkspaceTransform,
  workspaceConfigTransform,
} from './use-workspace-tabs'

/** A pre-hydration write held by the hook: activate a second tab. It is
    queued as a TRANSFORM (a delta), which is what makes replay order safe —
    a snapshot replayed late would erase whatever landed in between. */
const activateSecondTab: WorkspaceTransform = (state) => ({
  tabs: state.tabs.some((tab) => tab.id === 'tab-second')
    ? state.tabs
    : [...state.tabs, { id: 'tab-second', columns: 1 as const, screens: [] }],
  activeTabId: 'tab-second',
})

/** App.tsx's deep-link path: open a screen beside chat on the active tab's
    layout. Kept simple — the ordering under test is the writer's, not the
    open-screen geometry (workspace-tabs.test.ts owns that). */
const openShellScreen: WorkspaceTransform = (state) => ({
  tabs: [
    ...state.tabs,
    { id: 'tab-shell', columns: 1 as const, screens: ['ext:shell'] },
  ],
  activeTabId: 'tab-shell',
})

function harness(initial: ConsoleConfigValue) {
  let remote: ConsoleConfigValue = { ...initial }
  let cache: ConsoleConfigValue | null = { ...initial }
  const writer = new SerializedConfigWriter({
    readRemote: async () => ({ ...remote }),
    writeRemote: async (value) => {
      remote = value
    },
    readCached: () => cache,
    publish: (value) => {
      cache = value
    },
  })
  return {
    writer,
    remote: () => remote,
    flush: () => new Promise((resolve) => setTimeout(resolve, 0)),
  }
}

describe('pending-layout replay vs a deep-linked screen', () => {
  // The regression: the pre-hydration layout write used to go through an
  // unserialized whole-configuration write, so it could race App.tsx's
  // openScreen and one of the two would vanish. Both now enter the same
  // serialized writer as transforms, so BOTH intents survive either order.
  for (const order of ['replay-first', 'open-first'] as const) {
    it(`keeps both writes when the ${order.replace('-', ' ')}`, async () => {
      const stack = harness({})
      const transforms =
        order === 'replay-first'
          ? [activateSecondTab, openShellScreen]
          : [openShellScreen, activateSecondTab]
      for (const update of transforms) {
        stack.writer.enqueue(workspaceConfigTransform(update))
      }
      await stack.flush()

      const tabs = parseWorkspaceTabs(stack.remote())
      const ids = tabs.map((tab) => tab.id)
      expect(ids).toContain('tab-second')
      expect(ids).toContain('tab-shell')
      const shellTab = tabs.find((tab) => tab.id === 'tab-shell')
      expect(shellTab && tabPaneIds(shellTab).length).toBeGreaterThan(0)
      // The later transform's activation wins — last write, not lost write.
      expect(parseActiveTabId(stack.remote())).toBe(
        order === 'replay-first' ? 'tab-shell' : 'tab-second',
      )
    })
  }

  it('composes several pre-hydration writes instead of keeping only the last', async () => {
    // The hook composes queued transforms; a queue that kept only the last
    // SNAPSHOT would replay a layout that predates the earlier writes.
    const stack = harness({})
    const composed: WorkspaceTransform = (state) =>
      openShellScreen(activateSecondTab(state))
    stack.writer.enqueue(workspaceConfigTransform(composed))
    await stack.flush()

    const ids = parseWorkspaceTabs(stack.remote()).map((tab) => tab.id)
    expect(ids).toContain('tab-second')
    expect(ids).toContain('tab-shell')
  })
})

describe('pointer provenance on the server', () => {
  const functionActivated: ConsoleConfigValue = {
    workspace: {
      tabs: [
        { id: 'tab-home', columns: 2, screens: ['chat', 'traces'] },
        { id: 'tab-shell', columns: 1, screens: ['ext:shell'] },
      ],
      activeTabId: 'tab-shell',
      activatedAt: 500,
      activatedBy: 'function',
    },
  }

  it('a write that leaves the pointer alone keeps the function activation', async () => {
    const stack = harness(functionActivated)
    const rename: WorkspaceTransform = (state) => ({
      ...state,
      tabs: state.tabs.map((tab) =>
        tab.id === 'tab-shell' ? { ...tab, name: 'IDE' } : tab,
      ),
    })
    stack.writer.enqueue(workspaceConfigTransform(rename, () => 900))
    await stack.flush()
    expect(parseActivation(stack.remote())).toEqual({
      tabId: 'tab-shell',
      at: 500,
      by: 'function',
    })
    expect(parseWorkspaceTabs(stack.remote())[1].name).toBe('IDE')
  })

  it('a write that moves the pointer stamps a browser activation', async () => {
    const stack = harness(functionActivated)
    const activateHome: WorkspaceTransform = (state) => ({
      ...state,
      activeTabId: 'tab-home',
    })
    stack.writer.enqueue(workspaceConfigTransform(activateHome, () => 900))
    await stack.flush()
    expect(parseActivation(stack.remote())).toEqual({
      tabId: 'tab-home',
      at: 900,
      by: 'browser',
    })
  })

  it('closing the active tab on the server follows the neighbour rule', async () => {
    const stack = harness({
      workspace: {
        tabs: [
          { id: 'a', screens: ['chat'] },
          { id: 'b', screens: ['traces'] },
          { id: 'c', screens: ['workers'] },
        ],
        activeTabId: 'b',
      },
    })
    stack.writer.enqueue(
      workspaceConfigTransform((state) => withTabClosed(state, 'b')),
    )
    await stack.flush()
    expect(parseWorkspaceTabs(stack.remote()).map((t) => t.id)).toEqual([
      'a',
      'c',
    ])
    expect(parseActiveTabId(stack.remote())).toBe('c')
  })
})
