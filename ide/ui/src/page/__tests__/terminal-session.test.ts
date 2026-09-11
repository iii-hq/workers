import { describe, expect, it, vi } from 'vitest'

vi.mock('react', () => ({
  useCallback: (callback: unknown) => callback,
  useEffect: () => undefined,
  useMemo: (factory: () => unknown) => factory(),
  useReducer: () => [],
  useRef: () => ({ current: null }),
  useState: () => [],
}))

import {
  closeTerminalConnection,
  connectTerminalSession,
  createTerminalConnectionCoordinator,
  createTerminalSessionState,
  detachTerminalSessionForUnmount,
  disposeTerminalConnection,
  legacyTerminalStorageKey,
  normalizeTerminalDimensions,
  reclaimTerminalLease,
  reduceTerminalLease,
  reduceTerminalSessionState,
  shouldDetachStaleConnection,
  useTerminalSession,
} from '../terminal-session'
import {
  loadRecoverableTerminalLeases,
  removeRecoverableTerminalLease,
} from '../terminal-leases'

describe('terminal session state', () => {
  it('exports the per-pane session controller hook', () => {
    expect(typeof useTerminalSession).toBe('function')
  })

  it('uses a reload-stable lease key for the legacy terminal', () => {
    expect(legacyTerminalStorageKey('/repo')).toBe(
      'iii::shell-ui::terminal-leases::legacy::/repo',
    )
  })

  it('clamps xterm dimensions to the backend PTY boundary', () => {
    expect(normalizeTerminalDimensions(1, 0)).toEqual({ cols: 2, rows: 2 })
    expect(normalizeTerminalDimensions(501, 900)).toEqual({
      cols: 500,
      rows: 500,
    })
  })

  it('does not detach a connection owned by a newer retry generation', () => {
    expect(shouldDetachStaleConnection(true, 1, 2)).toBe(false)
    expect(shouldDetachStaleConnection(true, 2, 2)).toBe(true)
  })

  it('fences a pending connection after the pane remounts', () => {
    const requestIds = ['request-1', 'request-2']
    const coordinator = createTerminalConnectionCoordinator(
      () => requestIds.shift() ?? 'request-3',
    )
    const oldMount = coordinator.begin()
    const newMount = coordinator.begin()

    expect(newMount.requestId).toBe(oldMount.requestId)
    expect(coordinator.isCurrent(oldMount.generation)).toBe(false)
    expect(coordinator.isCurrent(newMount.generation)).toBe(true)
    coordinator.complete(newMount.generation)
    expect(coordinator.begin().requestId).toBe('request-2')
  })

  it('transitions from connecting to ready', () => {
    const state = reduceTerminalSessionState(
      createTerminalSessionState('/repo'),
      {
        type: 'connected',
        sessionId: 'session-1',
        cwd: '/repo',
      },
    )

    expect(state).toEqual({
      status: 'ready',
      cwd: '/repo',
      error: null,
      notice: null,
      sessionId: 'session-1',
      lastSequence: 0,
    })
  })

  it('transitions from connecting to disconnected', () => {
    const state = reduceTerminalSessionState(
      createTerminalSessionState('/repo'),
      {
        type: 'disconnected',
        error: 'terminal session does not exist',
      },
    )

    expect(state.status).toBe('disconnected')
    expect(state.error).toBe('terminal session does not exist')
    expect(state.sessionId).toBeNull()
  })

  it('transitions from ready to exited', () => {
    const ready = reduceTerminalSessionState(
      createTerminalSessionState('/repo'),
      {
        type: 'connected',
        sessionId: 'session-1',
        cwd: '/repo',
      },
    )
    const exited = reduceTerminalSessionState(ready, {
      type: 'exited',
      error: null,
    })

    expect(exited.status).toBe('exited')
    expect(exited.sessionId).toBe('session-1')
  })

  it('records a replay truncation notice', () => {
    const state = reduceTerminalSessionState(
      createTerminalSessionState('/repo'),
      {
        type: 'replay-truncated',
      },
    )

    expect(state.notice).toBe(
      '\r\n[terminal output truncated; showing retained output]\r\n',
    )
  })

  it('replaces the pane lease after restart', () => {
    const next = {
      paneId: 'pane-1',
      sessionId: 'session-2',
      reconnectToken: 'token-2',
      lastSequence: 0,
    }

    expect(
      reduceTerminalLease(
        {
          paneId: 'pane-1',
          sessionId: 'session-1',
          reconnectToken: 'token-1',
          lastSequence: 9,
        },
        { type: 'restarted', lease: next },
      ),
    ).toEqual(next)
  })

  it('removes the pane lease after close', () => {
    expect(
      reduceTerminalLease(
        {
          paneId: 'pane-1',
          sessionId: 'session-1',
          reconnectToken: 'token-1',
          lastSequence: 9,
        },
        { type: 'closed' },
      ),
    ).toBeNull()
  })
})

describe('reclaimTerminalLease', () => {
  it('attaches before closing with rotated access and removes the lease', async () => {
    const calls: Array<{ id: string; payload: Record<string, unknown> }> = []
    let subscribed = false
    const host = {
      iii: {
        trigger: async (id: string, payload: Record<string, unknown>) => {
          expect(subscribed).toBe(true)
          calls.push({ id, payload })
          if (id === 'shell::pty::attach') {
            return {
              access_key: 'rotated-access',
              reconnect_token: 'rotated-reconnect',
              frames: [],
              truncated: false,
              next_sequence: 10,
              cwd: '/repo',
              status: 'attached',
            }
          }
          return { closed: true }
        },
      },
    } as never
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: () => {
        subscribed = true
        return () => {
          subscribed = false
        }
      },
      drain: () => [],
      dispose: () => undefined,
    }
    const updates: string[] = []
    let removed = false

    await reclaimTerminalLease(host, router, {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'reconnect',
      lastSequence: 9,
      update: (lease) => updates.push(lease.reconnectToken),
      remove: () => {
        removed = true
      },
    })

    expect(calls.map((call) => call.id)).toEqual([
      'shell::pty::attach',
      'shell::pty::close',
    ])
    expect(calls[1]?.payload.access_key).toBe('rotated-access')
    expect(updates).toEqual(['rotated-reconnect'])
    expect(removed).toBe(true)
    expect(subscribed).toBe(false)
  })

  it('removes a stale lease after the backend confirms the session is missing', async () => {
    const host = {
      iii: {
        trigger: async () => {
          throw new Error('terminal session does not exist')
        },
      },
    } as never
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: () => () => undefined,
      drain: () => [],
      dispose: () => undefined,
    }
    let removed = false

    await reclaimTerminalLease(host, router, {
      paneId: 'pane-1',
      sessionId: 'missing',
      reconnectToken: 'reconnect',
      lastSequence: 9,
      update: () => undefined,
      remove: () => {
        removed = true
      },
    })

    expect(removed).toBe(true)
  })
})

describe('connectTerminalSession', () => {
  it('subscribes before attach and deduplicates queued live replay overlap', async () => {
    let emit:
      | ((event: {
          session_id: string
          sequence: number
          data: string | null
          eof: boolean
          exit_code: number | null
          signal: string | null
          error: string | null
        }) => void)
      | null = null
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: (_sessionId: string, listener: typeof emit) => {
        emit = listener
        return () => {
          emit = null
        }
      },
      drain: () => [],
      dispose: () => undefined,
    }
    const host = {
      iii: {
        trigger: async () => {
          expect(emit).not.toBeNull()
          emit?.({
            session_id: 'session-1',
            sequence: 5,
            data: 'Zml2ZQ==',
            eof: false,
            exit_code: null,
            signal: null,
            error: null,
          })
          return {
            access_key: 'access-2',
            reconnect_token: 'reconnect-2',
            frames: [
              { sequence: 4, data: 'Zm91cg==' },
              { sequence: 5, data: 'Zml2ZQ==' },
            ],
            truncated: false,
            next_sequence: 6,
            cwd: '/repo',
            status: 'attached',
          }
        },
      },
    } as never

    const connection = await connectTerminalSession({
      host,
      router,
      root: '/repo',
      requestId: 'request-1',
      lease: {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'reconnect-1',
        lastSequence: 3,
      },
      cols: 80,
      rows: 24,
    })
    const activated = connection.activate(() => undefined)

    expect(activated.frames).toEqual([
      { sequence: 4, data: 'Zm91cg==' },
      { sequence: 5, data: 'Zml2ZQ==' },
    ])
    connection.unsubscribe()
  })

  it('replays retained output when xterm was disposed before reattach', async () => {
    let attachPayload: Record<string, unknown> | null = null
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: () => () => undefined,
      drain: () => [],
      dispose: () => undefined,
    }
    const host = {
      iii: {
        trigger: async (_id: string, payload: Record<string, unknown>) => {
          attachPayload = payload
          return {
            access_key: 'access-2',
            reconnect_token: 'reconnect-2',
            frames: [
              { sequence: 1, data: 'b25l' },
              { sequence: 2, data: 'dHdv' },
            ],
            truncated: false,
            next_sequence: 3,
            cwd: '/repo',
            status: 'attached',
          }
        },
      },
    } as never

    const connection = await connectTerminalSession({
      host,
      router,
      root: '/repo',
      requestId: 'request-1',
      lease: {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'reconnect-1',
        lastSequence: 2,
      },
      cols: 80,
      rows: 24,
    })

    expect(attachPayload).toMatchObject({ after_sequence: 0 })
    expect(connection.activate(() => undefined).frames).toEqual([
      { sequence: 1, data: 'b25l' },
      { sequence: 2, data: 'dHdv' },
    ])
    connection.unsubscribe()
  })

  it('bounds output queued during attach and requests replay after eviction', async () => {
    let emit:
      | ((event: {
          session_id: string
          sequence: number
          data: string | null
          eof: boolean
          exit_code: number | null
          signal: string | null
          error: string | null
        }) => void)
      | null = null
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: (_sessionId: string, listener: typeof emit) => {
        emit = listener
        return () => undefined
      },
      drain: () => [],
      dispose: () => undefined,
    }
    const oneMiB = btoa('x'.repeat(1024 * 1024))
    const host = {
      iii: {
        trigger: async () => {
          for (let sequence = 1; sequence <= 3; sequence += 1) {
            emit?.({
              session_id: 'session-1',
              sequence,
              data: oneMiB,
              eof: false,
              exit_code: null,
              signal: null,
              error: null,
            })
          }
          return {
            access_key: 'access-2',
            reconnect_token: 'reconnect-2',
            frames: [],
            truncated: false,
            next_sequence: 1,
            cwd: '/repo',
            status: 'attached',
          }
        },
      },
    } as never

    const connection = await connectTerminalSession({
      host,
      router,
      root: '/repo',
      requestId: 'request-1',
      lease: {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'reconnect-1',
        lastSequence: 0,
      },
      cols: 80,
      rows: 24,
    })
    const activated = connection.activate(() => undefined)

    expect(activated.requiresReplay).toBe(true)
    expect(activated.frames).toEqual([])
  })
})

describe('disposeTerminalConnection', () => {
  it('unsubscribes, drains, and detaches an initialized connection', async () => {
    const calls: Array<{ id: string; payload: Record<string, unknown> }> = []
    const drained: string[] = []
    let unsubscribed = false
    const host = {
      iii: {
        trigger: async (id: string, payload: Record<string, unknown>) => {
          calls.push({ id, payload })
        },
      },
    } as never
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: () => () => undefined,
      drain: (sessionId: string) => {
        drained.push(sessionId)
        return []
      },
      dispose: () => undefined,
    }

    await disposeTerminalConnection(host, router, {
      sessionId: 'session-1',
      accessKey: 'access-1',
      reconnectToken: 'reconnect-1',
      cwd: '/repo',
      status: 'attached',
      truncated: false,
      nextSequence: 1,
      activate: () => ({
        frames: [],
        events: [],
        requiresReplay: false,
        replayThrough: 0,
      }),
      unsubscribe: () => {
        unsubscribed = true
      },
    })

    expect(unsubscribed).toBe(true)
    expect(drained).toEqual(['session-1'])
    expect(calls).toEqual([
      {
        id: 'shell::pty::detach',
        payload: {
          session_id: 'session-1',
          access_key: 'access-1',
        },
      },
    ])
  })
})

describe('detachTerminalSessionForUnmount', () => {
  it('keeps the reconnect lease in memory when storage is unavailable', async () => {
    const calls: Array<{ id: string; payload: Record<string, unknown> }> = []
    let unsubscribed = false
    const host = {
      iii: {
        trigger: async (id: string, payload: Record<string, unknown>) => {
          calls.push({ id, payload })
        },
      },
    } as never
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: () => () => undefined,
      drain: () => [],
      dispose: () => undefined,
    }
    const storageKey = 'unavailable-storage'

    await detachTerminalSessionForUnmount(
      host,
      router,
      storageKey,
      'pane-1',
      {
        sessionId: 'session-1',
        accessKey: 'access-1',
        reconnectToken: 'reconnect-1',
        lastSequence: 7,
        unsubscribe: () => {
          unsubscribed = true
        },
      },
    )

    expect(unsubscribed).toBe(true)
    expect(calls).toEqual([
      {
        id: 'shell::pty::detach',
        payload: {
          session_id: 'session-1',
          access_key: 'access-1',
        },
      },
    ])
    expect(loadRecoverableTerminalLeases(null, storageKey)).toEqual([
      {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'reconnect-1',
        lastSequence: 7,
      },
    ])
    removeRecoverableTerminalLease(null, storageKey, 'pane-1')
  })
})

describe('closeTerminalConnection', () => {
  it('closes an initialized connection instead of detaching it', async () => {
    const calls: string[] = []
    const host = {
      iii: {
        trigger: async (id: string) => {
          calls.push(id)
        },
      },
    } as never
    const router = {
      outputFunctionId: 'iii::shell-ui::pty-output::console-test',
      subscribe: () => () => undefined,
      drain: () => [],
      dispose: () => undefined,
    }

    await closeTerminalConnection(host, router, {
      sessionId: 'session-1',
      accessKey: 'access-1',
      reconnectToken: 'reconnect-1',
      cwd: '/repo',
      status: 'attached',
      truncated: false,
      nextSequence: 1,
      activate: () => ({
        frames: [],
        events: [],
        requiresReplay: false,
        replayThrough: 0,
      }),
      unsubscribe: () => undefined,
    })

    expect(calls).toEqual(['shell::pty::close'])
  })
})
