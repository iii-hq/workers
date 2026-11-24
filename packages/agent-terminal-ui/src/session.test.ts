import { beforeEach, describe, expect, it } from 'vitest'
import {
  describePayload,
  leaveSession,
  pickOrphan,
  readLease,
  type SessionSummary,
  startFolder,
  writeLease,
} from './session'

function memoryStorage(): Storage {
  const map = new Map<string, string>()
  return {
    get length() {
      return map.size
    },
    clear: () => map.clear(),
    getItem: (k) => map.get(k) ?? null,
    key: (i) => [...map.keys()][i] ?? null,
    removeItem: (k) => void map.delete(k),
    setItem: (k, v) => void map.set(k, String(v)),
  }
}

beforeEach(() => {
  Object.assign(globalThis, { localStorage: memoryStorage(), sessionStorage: memoryStorage() })
})

describe('the lease', () => {
  it('keeps the folder the session runs in, so a reload shows it', () => {
    writeLease('p::', 't1', { sessionId: 's', reconnectToken: 'r', cwd: '/work/app' })
    expect(readLease('p::', 't1')).toEqual({ sessionId: 's', reconnectToken: 'r', cwd: '/work/app' })
  })

  it('reads a lease from before folders, without one', () => {
    localStorage.setItem('p::t1', JSON.stringify({ sessionId: 's', reconnectToken: 'r' }))
    expect(readLease('p::', 't1')).toEqual({ sessionId: 's', reconnectToken: 'r' })
  })

  it('carries over a lease an older build kept in sessionStorage', () => {
    sessionStorage.setItem('p::t1', JSON.stringify({ sessionId: 's', reconnectToken: 'r' }))
    expect(readLease('p::', 't1')?.sessionId).toBe('s')
  })

  it('reads garbage as no lease', () => {
    localStorage.setItem('p::t1', '{not json')
    expect(readLease('p::', 't1')).toBeNull()
    localStorage.setItem('p::t1', JSON.stringify({ sessionId: 1 }))
    expect(readLease('p::', 't1')).toBeNull()
  })

  it('is per tab', () => {
    writeLease('p::', 't1', { sessionId: 'a', reconnectToken: 'r' })
    expect(readLease('p::', 't2')).toBeNull()
  })
})

describe('startFolder', () => {
  it("is the chat's working directory when nothing was picked", () => {
    expect(startFolder(null, '/chat/dir')).toBe('/chat/dir')
  })

  it('is the picked folder over the chat one', () => {
    expect(startFolder('/picked', '/chat/dir')).toBe('/picked')
  })

  it('is none without either: the worker then uses the compose project folder', () => {
    expect(startFolder(null, null)).toBeNull()
    expect(startFolder('  ', undefined)).toBeNull()
  })
})

describe('describePayload', () => {
  it('asks for the folder when the page has the pill', () => {
    expect(describePayload(true, '/work/app')).toEqual({ cwd: '/work/app' })
  })

  it('asks for nothing without a folder, or without the pill (pi stays as it was)', () => {
    expect(describePayload(true, null)).toEqual({})
    expect(describePayload(false, '/work/app')).toEqual({})
  })
})

describe('pickOrphan', () => {
  const session = (over: Partial<SessionSummary>): SessionSummary => ({
    session_id: 's1',
    cwd: '/work/a',
    program: '/bin/claude',
    status: 'detached',
    ui: 'claude-ui',
    ...over,
  })
  const want = { program: '/bin/claude', ui: 'claude-ui' }

  it('takes back a detached session of this page and program', () => {
    expect(pickOrphan([session({})], want)).toBe('s1')
  })

  it('leaves attached sessions, other pages and other programs', () => {
    expect(pickOrphan([session({ status: 'attached' })], want)).toBeNull()
    expect(pickOrphan([session({ ui: 'pi-ui' })], want)).toBeNull()
    expect(pickOrphan([session({ program: '/bin/pi' })], want)).toBeNull()
  })

  it('with a folder, takes back only a session in that folder', () => {
    const sessions = [session({ session_id: 'old', cwd: '/work/a' }), session({ session_id: 'new', cwd: '/work/b' })]
    expect(pickOrphan(sessions, { ...want, cwd: '/work/b' })).toBe('new')
    expect(pickOrphan(sessions, { ...want, cwd: '/work/c' })).toBeNull()
  })
})

describe('leaveSession: moving to another folder', () => {
  it('closes the running session (never just detaches it) and clears the lease', async () => {
    writeLease('p::', 't1', { sessionId: 's', reconnectToken: 'r', cwd: '/work/a' })
    const calls: [string, unknown][] = []
    const call = async <T,>(fn: string, payload: unknown) => {
      calls.push([fn, payload])
      return {} as T
    }
    await leaveSession(call, { sessionId: 's', accessKey: 'k' }, 'p::', 't1')
    expect(calls).toEqual([['shell::pty::close', { session_id: 's', access_key: 'k' }]])
    expect(readLease('p::', 't1')).toBeNull()
  })

  it('still clears the lease when there is no session, or it is already gone', async () => {
    writeLease('p::', 't1', { sessionId: 's', reconnectToken: 'r' })
    await leaveSession(
      async () => {
        throw new Error('terminal session does not exist')
      },
      { sessionId: 's', accessKey: 'k' },
      'p::',
      't1',
    )
    expect(readLease('p::', 't1')).toBeNull()
    await expect(leaveSession(async <T,>() => ({}) as T, null, 'p::', 't1')).resolves.toBeUndefined()
  })
})
