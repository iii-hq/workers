import { describe, expect, it, vi } from 'vitest'
import { createTabUiStateSaver, loadTabUiState } from '../persist'
import { createTerminalWorkspace } from '../terminal-layout'

function hostWithTabState(state: Record<string, unknown>) {
  return {
    iii: {
      trigger: async () => ({
        value: {
          tabs: {
            'tab-1': state,
          },
        },
      }),
    },
  } as never
}

describe('loadTabUiState', () => {
  it('restores terminal drawer, jobs, and sidebar flags', async () => {
    const host = {
      iii: {
        trigger: async () => ({ value: { tabs: {} } }),
      },
    } as never
    const hostWithState = hostWithTabState({
      root: '/repo',
      open: [],
      active: null,
      expanded: ['src'],
      showHidden: true,
      sideWidth: 280,
      workspaceMode: 'terminal',
      terminalOpen: true,
      terminalDock: 'right',
      terminalActive: true,
      terminalBottomSize: 320,
      terminalRightSize: 480,
      terminalJobIds: ['job-1'],
    })
    expect(await loadTabUiState(host, 'tab-1')).toBeNull()
    expect(await loadTabUiState(hostWithState, 'tab-1')).toEqual({
      root: '/repo',
      open: [],
      active: null,
      expanded: ['src'],
      showHidden: true,
      sideWidth: 280,
      terminalOpen: true,
      terminalDock: 'right',
      terminalActive: true,
      terminalBottomSize: 320,
      terminalRightSize: 480,
      terminalJobIds: ['job-1'],
      terminalWorkspace: {
        tabs: [
          {
            id: 'tab-1',
            title: 'zsh 1',
            layout: { type: 'pane', paneId: 'pane-1' },
          },
        ],
        panes: {
          'pane-1': { id: 'pane-1', cwd: '/repo' },
        },
        activeTabId: 'tab-1',
        focusedPaneId: 'pane-1',
      },
    })
  })

  it('migrates a legacy terminal drawer into one tab and pane', async () => {
    const restored = await loadTabUiState(
      hostWithTabState({
        root: '/repo',
        open: [],
        active: null,
        expanded: [],
        terminalOpen: true,
        terminalDock: 'right',
      }),
      'tab-1',
    )

    expect(restored?.terminalWorkspace?.tabs).toHaveLength(1)
    expect(Object.values(restored?.terminalWorkspace?.panes ?? {})).toEqual([
      expect.objectContaining({ cwd: '/repo' }),
    ])
  })

  it('persists workspace layout without session credentials', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('window', {
      setTimeout: globalThis.setTimeout,
      clearTimeout: globalThis.clearTimeout,
    })
    let saved: unknown = null
    const host = {
      iii: {
        trigger: async (id: string, payload: unknown) => {
          if (id === 'configuration::get') return { value: { tabs: {} } }
          saved = payload
          return null
        },
      },
    } as never
    const saver = createTabUiStateSaver(host, 'tab-1')

    saver.save({
      root: '/repo',
      open: [],
      active: null,
      expanded: [],
      terminalOpen: true,
      terminalWorkspace: createTerminalWorkspace('/repo'),
    })
    await vi.advanceTimersByTimeAsync(600)

    const serialized = JSON.stringify(saved)
    expect(serialized).toContain('"terminalWorkspace"')
    expect(serialized).not.toContain('accessKey')
    expect(serialized).not.toContain('reconnectToken')
    saver.dispose()
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })
})
