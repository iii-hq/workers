import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// One render of the hook, driven by hand: there is no DOM here. Effects are
// collected for the test to run, and the restart token's setter is kept,
// because a restart after a lost shell is what opens a new one.
const hooks = vi.hoisted(() => ({
  effects: [] as Array<() => unknown>,
  restart: null as { mock: { calls: unknown[] } } | null,
}))

vi.mock('react', () => ({
  useCallback: (callback: unknown) => callback,
  useEffect: (effect: () => unknown) => {
    hooks.effects.push(effect)
  },
  useMemo: (factory: () => unknown) => factory(),
  useReducer: (_reducer: unknown, arg: unknown, init: (arg: unknown) => unknown) => [init(arg), () => undefined],
  useRef: (current: unknown) => ({ current }),
  useState: (initial: unknown) => {
    const set = vi.fn()
    // The restart token is the hook's one numeric state.
    if (initial === 0) hooks.restart = set
    return [initial, set]
  },
  useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
}))

import { saveRecoverableTerminalLease } from '../terminal-leases'
import { createTerminalOutputRouter } from '../terminal-output-router'
import { useTerminalSession } from '../terminal-session'
import { createTerminalConnectionCoordinator } from '../terminal-session-state'

beforeEach(() => {
  vi.stubGlobal('window', {
    setInterval: () => 0,
    clearInterval: () => undefined,
    setTimeout: () => 0,
    clearTimeout: () => undefined,
  })
})

afterEach(() => {
  vi.unstubAllGlobals()
})

/** The first attach waits for the test; any later one finds the shell gone. */
function mountPane(storageKey: string, options: { closing?: boolean } = {}) {
  const calls: string[] = []
  let failFirstAttach: (error: Error) => void = () => undefined
  let attaches = 0
  const host = {
    iii: {
      browserId: 'console-test',
      on: () => () => undefined,
      addConnectionStateListener: () => () => undefined,
      trigger: async (id: string) => {
        calls.push(id)
        if (id !== 'shell::pty::attach') throw new Error(`unexpected ${id}`)
        attaches += 1
        if (attaches > 1) throw new Error('terminal session is closed')
        return new Promise((_resolve, reject) => {
          failFirstAttach = reject
        })
      },
    },
  } as never
  const coordinator = createTerminalConnectionCoordinator(() => 'request-1')
  coordinator.closing = options.closing ?? false
  saveRecoverableTerminalLease(null, storageKey, {
    paneId: 'pane-1',
    sessionId: 'session-1',
    reconnectToken: 'reconnect-1',
    lastSequence: 0,
  })
  hooks.effects = []
  hooks.restart = null
  const session = useTerminalSession({
    paneId: 'pane-1',
    root: '/repo',
    visible: true,
    router: createTerminalOutputRouter(host),
    leaseStore: null,
    storageKey,
    connectionCoordinator: coordinator,
  })
  for (const effect of hooks.effects) effect()
  return {
    calls,
    session,
    failFirstAttach: (error: Error) => failFirstAttach(error),
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('a closing terminal pane', () => {
  it('does not attach when it mounts mid-close', () => {
    const { calls } = mountPane('closing-remount', { closing: true })

    expect(calls).toEqual([])
  })

  it('opens no new shell when its attach finds the old one gone', async () => {
    const { calls, session, failFirstAttach } = mountPane('closing-attach')

    const closed = session.close()
    failFirstAttach(new Error('terminal session is closed'))

    await expect(closed).resolves.toBeNull()
    await settle()
    expect(hooks.restart?.mock.calls).toEqual([])
    expect(calls).not.toContain('shell::pty::open')
  })

  it('still replaces a lost shell when it is not closing', async () => {
    const { failFirstAttach } = mountPane('lost-shell')

    failFirstAttach(new Error('terminal session is closed'))
    await settle()

    expect(hooks.restart?.mock.calls).toHaveLength(1)
  })
})
