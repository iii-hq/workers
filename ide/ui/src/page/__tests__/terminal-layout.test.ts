import { describe, expect, it } from 'vitest'
import {
  createTerminalWorkspace,
  normalizeTerminalWorkspace,
  reduceTerminalWorkspace,
} from '../terminal-layout'

describe('terminal workspace layout', () => {
  it('creates and activates a second terminal tab', () => {
    const initial = createTerminalWorkspace('/repo')
    const next = reduceTerminalWorkspace(initial, {
      type: 'tab-created',
      tabId: 'tab-2',
      paneId: 'pane-2',
      root: '/repo',
    })

    expect(next.tabs.map((tab) => tab.id)).toEqual([
      initial.tabs[0].id,
      'tab-2',
    ])
    expect(next.activeTabId).toBe('tab-2')
    expect(next.focusedPaneId).toBe('pane-2')
  })

  it('rejects a fifth pane in one terminal tab', () => {
    let state = createTerminalWorkspace('/repo')
    const firstPane = state.focusedPaneId as string
    for (const index of [2, 3, 4]) {
      state = reduceTerminalWorkspace(state, {
        type: 'pane-split',
        paneId: state.focusedPaneId as string,
        newPaneId: `pane-${index}`,
        splitId: `split-${index}`,
        direction: index % 2 === 0 ? 'horizontal' : 'vertical',
      })
    }
    expect(Object.keys(state.panes)).toHaveLength(4)

    const rejected = reduceTerminalWorkspace(state, {
      type: 'pane-split',
      paneId: firstPane,
      newPaneId: 'pane-5',
      splitId: 'split-5',
      direction: 'horizontal',
    })

    expect(rejected).toBe(state)
  })

  it('rejects tab-created when pane ID already exists', () => {
    const initial = createTerminalWorkspace('/repo')
    const rejected = reduceTerminalWorkspace(initial, {
      type: 'tab-created',
      tabId: 'tab-2',
      paneId: 'pane-1',
      root: '/repo',
    })

    expect(rejected).toBe(initial)
    expect(rejected.tabs).toHaveLength(1)
  })

  it('rejects pane-split when new pane ID already exists', () => {
    const initial = createTerminalWorkspace('/repo')
    const rejected = reduceTerminalWorkspace(initial, {
      type: 'pane-split',
      paneId: 'pane-1',
      newPaneId: 'pane-1',
      splitId: 'split-2',
      direction: 'horizontal',
    })

    expect(rejected).toBe(initial)
    expect(Object.keys(rejected.panes)).toEqual(['pane-1'])
  })

  it('rejects pane-split when the split ID already exists', () => {
    const split = reduceTerminalWorkspace(createTerminalWorkspace('/repo'), {
      type: 'pane-split',
      paneId: 'pane-1',
      newPaneId: 'pane-2',
      splitId: 'split-1',
      direction: 'horizontal',
    })
    const rejected = reduceTerminalWorkspace(split, {
      type: 'pane-split',
      paneId: 'pane-2',
      newPaneId: 'pane-3',
      splitId: 'split-1',
      direction: 'vertical',
    })

    expect(rejected).toBe(split)
  })

  it('drops duplicate pane references and orphaned pane state while normalizing', () => {
    const normalized = normalizeTerminalWorkspace(
      {
        tabs: [
          {
            id: 'tab-1',
            title: 'zsh 1',
            layout: { type: 'pane', paneId: 'pane-1' },
          },
          {
            id: 'tab-2',
            title: 'zsh 2',
            layout: { type: 'pane', paneId: 'pane-1' },
          },
        ],
        panes: {
          'pane-1': { id: 'pane-1', cwd: '/repo' },
          orphan: { id: 'orphan', cwd: '/repo' },
        },
        activeTabId: 'tab-2',
        focusedPaneId: 'pane-1',
      },
      '/repo',
    )

    expect(normalized.tabs.map((tab) => tab.id)).toEqual(['tab-1'])
    expect(Object.keys(normalized.panes)).toEqual(['pane-1'])
    expect(normalized.activeTabId).toBe('tab-1')
  })

  it('restores normalized workspace state after asynchronous page boot', () => {
    const initial = createTerminalWorkspace('/')
    const restored = createTerminalWorkspace('/repo')
    const next = reduceTerminalWorkspace(initial, {
      type: 'workspace-restored',
      state: restored,
    })

    expect(next).toBe(restored)
  })
})
