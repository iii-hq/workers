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
      sideView: undefined,
      diffOptions: undefined,
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

  it('retries a load that fails for a reason other than "nothing stored"', async () => {
    const calls: string[] = []
    const waits: number[] = []
    let attempts = 0
    const host = {
      iii: {
        trigger: async (id: string, payload: { raw?: boolean }) => {
          calls.push(id)
          expect(payload.raw).toBe(true)
          attempts += 1
          if (attempts < 3) throw new Error('engine is starting')
          return { value: { tabs: { 'tab-1': { root: '/repo', open: [], active: null, expanded: [] } } } }
        },
      },
    } as never
    const wait = async (ms: number) => {
      waits.push(ms)
    }
    const restored = await loadTabUiState(host, 'tab-1', undefined, { delays: [10, 20, 30], wait })
    expect(restored?.root).toBe('/repo')
    expect(calls).toHaveLength(3)
    expect(waits).toEqual([10, 20])
  })

  it('gives up after the retries and boots fresh, but never retries "nothing stored"', async () => {
    const failing = { iii: { trigger: async () => { throw new Error('engine is starting') } } } as never
    const waits: number[] = []
    const wait = async (ms: number) => {
      waits.push(ms)
    }
    expect(await loadTabUiState(failing, 'tab-1', undefined, { delays: [1, 2], wait })).toBeNull()
    expect(waits).toEqual([1, 2])

    let asked = 0
    const unregistered = {
      iii: {
        trigger: async () => {
          asked += 1
          throw new Error("configuration 'shell-ui' not found")
        },
      },
    } as never
    expect(await loadTabUiState(unregistered, 'tab-1', undefined, { delays: [1, 2], wait: async () => {} })).toBeNull()
    expect(asked).toBe(1)
  })

  it('reads a save keyed by the workspace tab when the pane has none', async () => {
    const host = hostWithTabState({ root: '/repo', open: [], active: null, expanded: ['src'] })
    expect(await loadTabUiState(host, 'tab-1:pane:0', 'tab-1')).toMatchObject({ root: '/repo', expanded: ['src'] })
    expect(await loadTabUiState(host, 'tab-1:pane:1')).toBeNull()
    expect(await loadTabUiState(host, '', 'tab-1')).toBeNull()
  })

  it('restores the pinned folder and what was open per folder', async () => {
    const host = hostWithTabState({
      root: '/b',
      rootPinned: true,
      open: [],
      active: null,
      expanded: [],
      roots: {
        '/a': { open: [{ kind: 'file', path: 'a.ts', pinned: true }], active: 'file:a.ts', expanded: ['src'] },
        '/junk': 'nope',
      },
    })
    const restored = await loadTabUiState(host, 'tab-1')
    expect(restored?.rootPinned).toBe(true)
    expect(restored?.roots).toEqual({
      '/a': { open: [{ kind: 'file', path: 'a.ts', pinned: true }], active: 'file:a.ts', expanded: ['src'] },
    })
    const plain = await loadTabUiState(hostWithTabState({ root: '/b', open: [], active: null, expanded: [] }), 'tab-1')
    expect(plain?.rootPinned).toBeUndefined()
    expect(plain?.roots).toBeUndefined()
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

  it('keeps trying while the entry is not registered yet, and stops for good without a worker', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('window', {
      setTimeout: globalThis.setTimeout,
      clearTimeout: globalThis.clearTimeout,
    })
    const state = { root: '/repo', open: [], active: null, expanded: [] }
    const run = async (failure: string) => {
      let sets = 0
      const host = {
        iii: {
          trigger: async (id: string) => {
            if (id === 'configuration::get') return { value: { tabs: {} } }
            sets += 1
            throw new Error(failure)
          },
        },
      } as never
      const saver = createTabUiStateSaver(host, 'tab-1')
      saver.save(state)
      await vi.advanceTimersByTimeAsync(600)
      saver.save({ ...state, expanded: ['src'] })
      await vi.advanceTimersByTimeAsync(600)
      saver.dispose()
      return sets
    }
    expect(await run("configuration 'shell-ui' not found")).toBe(2)
    expect(await run('function_not_found: configuration::set')).toBe(1)
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })
})
