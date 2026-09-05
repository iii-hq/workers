import { describe, expect, it, vi } from 'vitest'
import { createTabUiStateSaver, loadTabUiState, UI_STATE_GET_FN, UI_STATE_SET_FN } from '../persist'
import { createTerminalWorkspace } from '../terminal-layout'

/** A worker holding `states` per key, answering `get` the way
    `shell::ui-state::get` does (the pane's key, else its legacy key). */
function hostWithStates(states: Record<string, unknown>) {
  const calls: Array<{ id: string; payload: Record<string, unknown> }> = []
  const host = {
    iii: {
      trigger: async (id: string, payload: Record<string, unknown>) => {
        calls.push({ id, payload })
        if (id !== UI_STATE_GET_FN) throw new Error(`unexpected ${id}`)
        const key = payload.key as string
        const legacy = payload.legacy_key as string | undefined
        const state = states[key] ?? (legacy !== undefined ? states[legacy] : undefined) ?? null
        return { key, state }
      },
    },
  } as never
  return { host, calls }
}

describe('loadTabUiState', () => {
  it('asks the worker for the pane and restores terminal drawer, jobs, and sidebar flags', async () => {
    const empty = hostWithStates({})
    expect(await loadTabUiState(empty.host, 'tab-1')).toBeNull()
    expect(empty.calls).toEqual([{ id: UI_STATE_GET_FN, payload: { key: 'tab-1', legacy_key: undefined } }])

    const { host } = hostWithStates({
      'tab-1': {
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
      },
    })
    expect(await loadTabUiState(host, 'tab-1')).toEqual({
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

  it('retries a load that fails, whatever the failure', async () => {
    const calls: string[] = []
    const waits: number[] = []
    let attempts = 0
    const host = {
      iii: {
        trigger: async (id: string) => {
          calls.push(id)
          attempts += 1
          if (attempts === 1) throw new Error('function_not_found: shell::ui-state::get')
          if (attempts === 2) throw new Error('engine is starting')
          return { key: 'tab-1', state: { root: '/repo', open: [], active: null, expanded: [] } }
        },
      },
    } as never
    const wait = async (ms: number) => {
      waits.push(ms)
    }
    const restored = await loadTabUiState(host, 'tab-1', undefined, { delays: [10, 20, 30], wait })
    expect(restored?.root).toBe('/repo')
    expect(calls).toEqual([UI_STATE_GET_FN, UI_STATE_GET_FN, UI_STATE_GET_FN])
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

    const nothing = hostWithStates({})
    expect(await loadTabUiState(nothing.host, 'tab-1', undefined, { delays: [1, 2], wait: async () => {} })).toBeNull()
    expect(nothing.calls).toHaveLength(1)
  })

  it('reads a save keyed by the workspace tab when the pane has none', async () => {
    const { host, calls } = hostWithStates({ 'tab-1': { root: '/repo', open: [], active: null, expanded: ['src'] } })
    expect(await loadTabUiState(host, 'tab-1:pane:0', 'tab-1')).toMatchObject({ root: '/repo', expanded: ['src'] })
    expect(calls[0]?.payload).toEqual({ key: 'tab-1:pane:0', legacy_key: 'tab-1' })
    expect(await loadTabUiState(host, 'tab-1:pane:1')).toBeNull()
    expect(await loadTabUiState(host, '', 'tab-1')).toBeNull()
  })

  it('restores the pinned folder and what was open per folder', async () => {
    const { host } = hostWithStates({
      'tab-1': {
        root: '/b',
        rootPinned: true,
        open: [],
        active: null,
        expanded: [],
        roots: {
          '/a': { open: [{ kind: 'file', path: 'a.ts', pinned: true }], active: 'file:a.ts', expanded: ['src'] },
          '/junk': 'nope',
        },
      },
    })
    const restored = await loadTabUiState(host, 'tab-1')
    expect(restored?.rootPinned).toBe(true)
    expect(restored?.roots).toEqual({
      '/a': { open: [{ kind: 'file', path: 'a.ts', pinned: true }], active: 'file:a.ts', expanded: ['src'] },
    })
    const plain = await loadTabUiState(
      hostWithStates({ 'tab-1': { root: '/b', open: [], active: null, expanded: [] } }).host,
      'tab-1',
    )
    expect(plain?.rootPinned).toBeUndefined()
    expect(plain?.roots).toBeUndefined()
  })

  it('migrates a legacy terminal drawer into one tab and pane', async () => {
    const restored = await loadTabUiState(
      hostWithStates({
        'tab-1': {
          root: '/repo',
          open: [],
          active: null,
          expanded: [],
          terminalOpen: true,
          terminalDock: 'right',
        },
      }).host,
      'tab-1',
    )

    expect(restored?.terminalWorkspace?.tabs).toHaveLength(1)
    expect(Object.values(restored?.terminalWorkspace?.panes ?? {})).toEqual([
      expect.objectContaining({ cwd: '/repo' }),
    ])
  })
})

describe('createTabUiStateSaver', () => {
  function withFakeTimers<T>(run: () => Promise<T>): Promise<T> {
    vi.useFakeTimers()
    vi.stubGlobal('window', {
      setTimeout: globalThis.setTimeout,
      clearTimeout: globalThis.clearTimeout,
    })
    return run().finally(() => {
      vi.unstubAllGlobals()
      vi.useRealTimers()
    })
  }

  it('writes only this pane, without reading first, and without session credentials', () =>
    withFakeTimers(async () => {
      const calls: Array<{ id: string; payload: unknown }> = []
      const host = {
        iii: {
          trigger: async (id: string, payload: unknown) => {
            calls.push({ id, payload })
            return { key: 'tab-1', bytes: 1 }
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

      expect(calls).toHaveLength(1)
      expect(calls[0]?.id).toBe(UI_STATE_SET_FN)
      const payload = calls[0]?.payload as { key: string; state: Record<string, unknown> }
      expect(payload.key).toBe('tab-1')
      expect(payload.state.root).toBe('/repo')
      const serialized = JSON.stringify(payload)
      expect(serialized).toContain('"terminalWorkspace"')
      expect(serialized).not.toContain('accessKey')
      expect(serialized).not.toContain('reconnectToken')
      saver.dispose()
    }))

  it('debounces to the trailing state and flushes on dispose', () =>
    withFakeTimers(async () => {
      const states: Array<Record<string, unknown>> = []
      const host = {
        iii: {
          trigger: async (_id: string, payload: { state: Record<string, unknown> }) => {
            states.push(payload.state)
            return {}
          },
        },
      } as never
      const saver = createTabUiStateSaver(host, 'tab-1')
      const base = { root: '/repo', open: [], active: null, expanded: [] as string[] }
      saver.save(base)
      saver.save({ ...base, expanded: ['a'] })
      saver.save({ ...base, expanded: ['a', 'b'] })
      await vi.advanceTimersByTimeAsync(600)
      expect(states.map((s) => s.expanded)).toEqual([['a', 'b']])

      saver.save({ ...base, expanded: ['c'] })
      saver.dispose()
      await vi.advanceTimersByTimeAsync(0)
      expect(states.map((s) => s.expanded)).toEqual([['a', 'b'], ['c']])
    }))

  it('keeps trying on the next change after a failed write, whatever the failure', () =>
    withFakeTimers(async () => {
      const run = async (failure: string) => {
        let sets = 0
        const host = {
          iii: {
            trigger: async () => {
              sets += 1
              throw new Error(failure)
            },
          },
        } as never
        const saver = createTabUiStateSaver(host, 'tab-1')
        const state = { root: '/repo', open: [], active: null, expanded: [] }
        saver.save(state)
        await vi.advanceTimersByTimeAsync(600)
        saver.save({ ...state, expanded: ['src'] })
        await vi.advanceTimersByTimeAsync(600)
        saver.dispose()
        return sets
      }
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      expect(await run('engine is starting')).toBe(2)
      expect(await run('function_not_found: shell::ui-state::set')).toBe(2)
      expect(warn).toHaveBeenCalledTimes(4)
      warn.mockRestore()
    }))

  it('never writes for an empty key', () =>
    withFakeTimers(async () => {
      let sets = 0
      const host = { iii: { trigger: async () => { sets += 1; return {} } } } as never
      const saver = createTabUiStateSaver(host, '')
      saver.save({ root: '/repo', open: [], active: null, expanded: [] })
      await vi.advanceTimersByTimeAsync(600)
      saver.dispose()
      expect(sets).toBe(0)
    }))
})
