import { describe, expect, it } from 'vitest'
import { createTerminalWorkspace } from '../terminal-layout'
import {
  loadTerminalLeases,
  removeTerminalLease,
  saveTerminalLease,
  TerminalLeaseStorageError,
} from '../terminal-leases'

const MAX_LEASE_PAYLOAD_BYTES = 64 * 1024
const textEncoder = new TextEncoder()

function memoryStorage(): Storage {
  const values = new Map<string, string>()
  return {
    get length() {
      return values.size
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key)
    },
    setItem: (key, value) => {
      values.set(key, value)
    },
  }
}

describe('terminal leases', () => {
  it('stores reconnect tokens only in browser storage', () => {
    const storage = memoryStorage()
    saveTerminalLease(storage, 'origin:tab', {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'secret',
      lastSequence: 7,
    })

    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([
      expect.objectContaining({ reconnectToken: 'secret' }),
    ])
    expect(
      JSON.stringify({
        terminalWorkspace: createTerminalWorkspace('/repo'),
      }),
    ).not.toContain('secret')
  })

  it('rejects access keys on save', () => {
    const storage = memoryStorage()
    expect(() =>
      saveTerminalLease(storage, 'origin:tab', {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'token',
        lastSequence: 0,
        accessKey: 'must-not-persist',
      }),
    ).toThrow(/access keys must not be persisted/)
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
  })

  it('rejects malformed leases on save', () => {
    const storage = memoryStorage()
    expect(() =>
      saveTerminalLease(storage, 'origin:tab', {
        paneId: '',
        sessionId: 'session-1',
        reconnectToken: 'token',
        lastSequence: 0,
      }),
    ).toThrow(/invalid terminal lease/)
    expect(() =>
      saveTerminalLease(storage, 'origin:tab', {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'token',
        lastSequence: -1,
      }),
    ).toThrow(/invalid terminal lease/)
  })

  it('rejects malformed stored payloads on load', () => {
    const storage = memoryStorage()
    storage.setItem('origin:tab', '{not-json')
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])

    storage.setItem(
      'origin:tab',
      JSON.stringify([
        {
          paneId: 'pane-1',
          sessionId: 'session-1',
          reconnectToken: '',
          lastSequence: 0,
        },
      ]),
    )
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])

    storage.setItem(
      'origin:tab',
      JSON.stringify([
        {
          paneId: 'pane-1',
          sessionId: 'session-1',
          reconnectToken: 'token',
          lastSequence: 0,
          accessKey: 'secret',
        },
      ]),
    )
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
  })

  it('rejects duplicate pane ids in stored payloads', () => {
    const storage = memoryStorage()
    storage.setItem(
      'origin:tab',
      JSON.stringify([
        {
          paneId: 'pane-1',
          sessionId: 'session-1',
          reconnectToken: 'token-a',
          lastSequence: 1,
        },
        {
          paneId: 'pane-1',
          sessionId: 'session-2',
          reconnectToken: 'token-b',
          lastSequence: 2,
        },
      ]),
    )
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
  })

  it('rejects stored payloads larger than 64 KiB by utf-8 byte length', () => {
    const storage = memoryStorage()
    const overLimit = 'x'.repeat(MAX_LEASE_PAYLOAD_BYTES + 1)
    storage.setItem('origin:tab', overLimit)
    expect(textEncoder.encode(overLimit).length).toBeGreaterThan(
      MAX_LEASE_PAYLOAD_BYTES,
    )
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
  })

  it('accepts stored payloads at the exact 64 KiB utf-8 boundary', () => {
    const storage = memoryStorage()
    const lease = {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'token',
      lastSequence: 0,
    }
    const serialized = JSON.stringify([lease])
    const payload = `${serialized}${' '.repeat(
      MAX_LEASE_PAYLOAD_BYTES - textEncoder.encode(serialized).length,
    )}`
    expect(textEncoder.encode(payload).length).toBe(MAX_LEASE_PAYLOAD_BYTES)
    storage.setItem('origin:tab', payload)
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([lease])
  })

  it('rejects multibyte payloads that fit in utf-16 but exceed 64 KiB utf-8', () => {
    const storage = memoryStorage()
    const multibyte = '\u{1F600}'.repeat(22_000)
    const payload = JSON.stringify([
      {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: multibyte,
        lastSequence: 0,
      },
    ])
    expect(payload.length).toBeLessThan(MAX_LEASE_PAYLOAD_BYTES)
    expect(textEncoder.encode(payload).length).toBeGreaterThan(
      MAX_LEASE_PAYLOAD_BYTES,
    )
    storage.setItem('origin:tab', payload)
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
  })

  it('rejects saves that exceed 64 KiB by utf-8 byte length', () => {
    const storage = memoryStorage()
    saveTerminalLease(storage, 'origin:tab', {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'existing',
      lastSequence: 0,
    })
    const multibyte = '\u{1F600}'.repeat(22_000)
    expect(() =>
      saveTerminalLease(storage, 'origin:tab', {
        paneId: 'pane-2',
        sessionId: 'session-2',
        reconnectToken: multibyte,
        lastSequence: 0,
      }),
    ).toThrow(/terminal lease payload exceeds 64 KiB/)
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([
      {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'existing',
        lastSequence: 0,
      },
    ])
  })

  it('returns an empty list when storage read fails on load', () => {
    const storage = {
      get length() {
        return 0
      },
      clear: () => {},
      getItem: () => {
        throw new DOMException('read failed', 'SecurityError')
      },
      key: () => null,
      removeItem: () => {},
      setItem: () => {},
    }
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
  })

  it('throws TerminalLeaseStorageError when storage read fails on save', () => {
    const storage = {
      get length() {
        return 0
      },
      clear: () => {},
      getItem: () => {
        throw new DOMException('read failed', 'SecurityError')
      },
      key: () => null,
      removeItem: () => {},
      setItem: () => {},
    }
    expect(() =>
      saveTerminalLease(storage, 'origin:tab', {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'token',
        lastSequence: 0,
      }),
    ).toThrow(TerminalLeaseStorageError)
  })

  it('throws TerminalLeaseStorageError when storage write fails on save', () => {
    const backing = memoryStorage()
    saveTerminalLease(backing, 'origin:tab', {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'existing',
      lastSequence: 0,
    })
    const storage = {
      ...backing,
      setItem: (_key: string, _value: string) => {
        throw new DOMException('quota exceeded', 'QuotaExceededError')
      },
    }
    expect(() =>
      saveTerminalLease(storage, 'origin:tab', {
        paneId: 'pane-2',
        sessionId: 'session-2',
        reconnectToken: 'token',
        lastSequence: 0,
      }),
    ).toThrow(TerminalLeaseStorageError)
    expect(loadTerminalLeases(backing, 'origin:tab')).toEqual([
      {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'existing',
        lastSequence: 0,
      },
    ])
  })

  it('throws TerminalLeaseStorageError when storage read fails on remove', () => {
    const storage = {
      get length() {
        return 0
      },
      clear: () => {},
      getItem: () => {
        throw new DOMException('read failed', 'SecurityError')
      },
      key: () => null,
      removeItem: () => {},
      setItem: () => {},
    }
    expect(() => removeTerminalLease(storage, 'origin:tab', 'pane-1')).toThrow(
      TerminalLeaseStorageError,
    )
  })

  it('throws TerminalLeaseStorageError when storage remove fails', () => {
    const backing = memoryStorage()
    saveTerminalLease(backing, 'origin:tab', {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'token',
      lastSequence: 0,
    })
    const storage = {
      ...backing,
      removeItem: () => {
        throw new DOMException('remove failed', 'SecurityError')
      },
    }
    expect(() => removeTerminalLease(storage, 'origin:tab', 'pane-1')).toThrow(
      TerminalLeaseStorageError,
    )
    expect(loadTerminalLeases(backing, 'origin:tab')).toEqual([
      {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'token',
        lastSequence: 0,
      },
    ])
  })

  it('upserts leases by pane id and removes them', () => {
    const storage = memoryStorage()
    saveTerminalLease(storage, 'origin:tab', {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'token-a',
      lastSequence: 1,
    })
    saveTerminalLease(storage, 'origin:tab', {
      paneId: 'pane-1',
      sessionId: 'session-1',
      reconnectToken: 'token-b',
      lastSequence: 9,
    })
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([
      {
        paneId: 'pane-1',
        sessionId: 'session-1',
        reconnectToken: 'token-b',
        lastSequence: 9,
      },
    ])

    removeTerminalLease(storage, 'origin:tab', 'pane-1')
    expect(loadTerminalLeases(storage, 'origin:tab')).toEqual([])
    expect(storage.getItem('origin:tab')).toBeNull()
  })
})
