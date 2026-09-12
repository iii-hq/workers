import { afterEach, describe, expect, it, vi } from 'vitest'
import type { StatusChangedEvent } from '@/lib/sessions/types'
import {
  __resetCompletionBellForTests,
  ringForCompletionEvent,
  shouldRingCompletionBell,
} from './completion-bell'

const CLAIMS_KEY = 'iii-completion-bell-claims'

type MutableRequest = {
  result?: unknown
  error: DOMException | null
  onsuccess: ((event: Event) => void) | null
  onerror: ((event: Event) => void) | null
  onupgradeneeded?: ((event: IDBVersionChangeEvent) => void) | null
  onblocked?: ((event: Event) => void) | null
}

function memoryStorage(): Storage {
  const entries = new Map<string, string>()
  return {
    get length() {
      return entries.size
    },
    clear: () => entries.clear(),
    getItem: (key) => entries.get(key) ?? null,
    key: (index) => [...entries.keys()][index] ?? null,
    removeItem: (key) => entries.delete(key),
    setItem: (key, value) => entries.set(key, value),
  }
}

class FakeBroadcastChannel {
  static instances = new Set<FakeBroadcastChannel>()
  onmessage: ((event: MessageEvent) => void) | null = null

  constructor(readonly name: string) {
    FakeBroadcastChannel.instances.add(this)
  }

  postMessage(data: unknown): void {
    for (const channel of FakeBroadcastChannel.instances) {
      if (channel !== this && channel.name === this.name) {
        channel.onmessage?.({ data } as MessageEvent)
      }
    }
  }

  close(): void {
    FakeBroadcastChannel.instances.delete(this)
  }
}

function fakeIndexedDb(): IDBFactory {
  const records = new Map<string, unknown>()
  const jobs: Array<() => void> = []
  let busy = false
  let storeCreated = false

  const pump = () => {
    if (busy) return
    const job = jobs.shift()
    if (!job) return
    busy = true
    queueMicrotask(() => {
      job()
      busy = false
      pump()
    })
  }

  const database = {
    objectStoreNames: { contains: () => storeCreated },
    createObjectStore: () => {
      storeCreated = true
      return {} as IDBObjectStore
    },
    close: () => undefined,
    transaction: () => {
      const transaction = {
        error: null,
        oncomplete: null as ((event: Event) => void) | null,
        onabort: null as ((event: Event) => void) | null,
        objectStore: () => {
          const store = {
            get: (key: IDBValidKey) => {
              const request: MutableRequest = {
                error: null,
                onsuccess: null,
                onerror: null,
              }
              jobs.push(() => {
                request.result = records.get(String(key))
                request.onsuccess?.(new Event('success'))
                transaction.oncomplete?.(new Event('complete'))
              })
              pump()
              return request as unknown as IDBRequest
            },
            put: (value: { eventKey: string }) => {
              records.set(value.eventKey, value)
              return {} as IDBRequest
            },
            delete: (key: IDBValidKey) => {
              records.delete(String(key))
              return {} as IDBRequest
            },
          }
          return store as unknown as IDBObjectStore
        },
      }
      return transaction as unknown as IDBTransaction
    },
  }

  return {
    open: () => {
      const request: MutableRequest = {
        result: database,
        error: null,
        onsuccess: null,
        onerror: null,
        onupgradeneeded: null,
        onblocked: null,
      }
      queueMicrotask(() => {
        if (!storeCreated) {
          request.onupgradeneeded?.({} as IDBVersionChangeEvent)
        }
        request.onsuccess?.(new Event('success'))
      })
      return request as unknown as IDBOpenDBRequest
    },
  } as unknown as IDBFactory
}

function fakeAudioContext(
  resumeOutcomes: Array<'success' | 'failure'>,
  onToneStart: () => void = () => undefined,
): {
  constructor: new () => AudioContext
  stats: { resumes: number; tones: number }
} {
  const stats = { resumes: 0, tones: 0 }
  class Context {
    state: AudioContextState = 'suspended'
    currentTime = 0
    destination = {} as AudioDestinationNode

    async resume(): Promise<void> {
      const outcome = resumeOutcomes[stats.resumes] ?? 'success'
      stats.resumes += 1
      if (outcome === 'failure') throw new Error('audio blocked')
      this.state = 'running'
    }

    createOscillator(): OscillatorNode {
      return {
        frequency: { setValueAtTime: () => undefined },
        connect: () => undefined,
        start: () => {
          stats.tones += 1
          onToneStart()
        },
        stop: () => undefined,
      } as unknown as OscillatorNode
    }

    createGain(): GainNode {
      return {
        gain: {
          setValueAtTime: () => undefined,
          exponentialRampToValueAtTime: () => undefined,
        },
        connect: () => undefined,
      } as unknown as GainNode
    }
  }

  return { constructor: Context as unknown as new () => AudioContext, stats }
}

function ringableConversation() {
  return { status: 'working' as const, hierarchyResolved: true }
}

afterEach(() => {
  __resetCompletionBellForTests()
  FakeBroadcastChannel.instances.clear()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

function statusEvent(
  status: StatusChangedEvent['status'],
  overrides: Partial<StatusChangedEvent> = {},
): StatusChangedEvent {
  return {
    session_id: 'session-1',
    previous_status: 'working',
    status,
    timestamp: 1_000,
    ...overrides,
  }
}

describe('shouldRingCompletionBell', () => {
  it.each(['done', 'error'] as const)(
    'rings for a top-level working → %s transition',
    (status) => {
      expect(
        shouldRingCompletionBell(statusEvent(status), {
          status: 'working',
          hierarchyResolved: true,
        }),
      ).toBe(true)
    },
  )

  it('does not ring for state hydration or duplicate terminal events', () => {
    expect(
      shouldRingCompletionBell(
        statusEvent('done', { previous_status: 'done' }),
        { status: 'done' },
      ),
    ).toBe(false)
  })

  it('does not ring when a child finishes before hierarchy metadata resolves', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        status: 'working',
        hierarchyResolved: false,
      }),
    ).toBe(false)
  })

  it('does not ring for known child sessions', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        parentId: 'parent',
        hierarchyResolved: true,
        status: 'working',
      }),
    ).toBe(false)
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        depth: 1,
        hierarchyResolved: true,
        status: 'working',
      }),
    ).toBe(false)
  })

  it('does not ring for a delayed event older than the current status', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        status: 'working',
        hierarchyResolved: true,
        serverStatusUpdatedAt: 2_000,
      }),
    ).toBe(false)
  })

  it('does not ring for a user-stopped turn', () => {
    expect(
      shouldRingCompletionBell(
        statusEvent('done', { status_reason: 'stopped' }),
        { status: 'working', hierarchyResolved: true },
      ),
    ).toBe(false)
  })

  it('does not ring after the local conversation is already terminal', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), { status: 'done' }),
    ).toBe(false)
  })

  it('does not ring for a session that is not known locally', () => {
    expect(shouldRingCompletionBell(statusEvent('done'), undefined)).toBe(false)
  })
})

describe('ringForCompletionEvent cross-tab claims', () => {
  it('holds the Web Lock through playback and claims only after success', async () => {
    const storage = memoryStorage()
    let lockHeld = false
    const audio = fakeAudioContext(['failure', 'success'], () => {
      expect(lockHeld).toBe(true)
    })
    vi.stubGlobal('window', { AudioContext: audio.constructor })
    vi.stubGlobal('localStorage', storage)
    vi.stubGlobal('navigator', {
      locks: {
        request: async (_name: string, callback: () => Promise<boolean>) => {
          expect(lockHeld).toBe(false)
          lockHeld = true
          try {
            return await callback()
          } finally {
            lockHeld = false
          }
        },
      },
    })
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)

    const event = statusEvent('done')
    await expect(
      ringForCompletionEvent(event, ringableConversation()),
    ).resolves.toBe(false)
    expect(storage.getItem(CLAIMS_KEY)).toBeNull()

    await expect(
      ringForCompletionEvent(event, ringableConversation()),
    ).resolves.toBe(true)
    expect(storage.getItem(CLAIMS_KEY)).toContain(event.session_id)
    expect(audio.stats).toEqual({ resumes: 2, tones: 2 })
  })

  it('uses one atomic IndexedDB reservation when Web Locks are unavailable', async () => {
    const audio = fakeAudioContext(['success'])
    vi.stubGlobal('window', { AudioContext: audio.constructor })
    vi.stubGlobal('localStorage', memoryStorage())
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('indexedDB', fakeIndexedDb())
    vi.stubGlobal(
      'BroadcastChannel',
      FakeBroadcastChannel as unknown as typeof BroadcastChannel,
    )

    const event = statusEvent('done')
    const results = await Promise.all([
      ringForCompletionEvent(event, ringableConversation()),
      ringForCompletionEvent(event, ringableConversation()),
    ])

    expect(results.sort()).toEqual([false, true])
    expect(audio.stats).toEqual({ resumes: 1, tones: 2 })
  })

  it('releases a failed IndexedDB reservation for another playable tab', async () => {
    const audio = fakeAudioContext(['failure', 'success'])
    vi.stubGlobal('window', { AudioContext: audio.constructor })
    vi.stubGlobal('localStorage', memoryStorage())
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('indexedDB', fakeIndexedDb())
    vi.stubGlobal(
      'BroadcastChannel',
      FakeBroadcastChannel as unknown as typeof BroadcastChannel,
    )
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)

    const event = statusEvent('error')
    const results = await Promise.all([
      ringForCompletionEvent(event, ringableConversation()),
      ringForCompletionEvent(event, ringableConversation()),
    ])

    expect(results).toEqual([false, true])
    expect(audio.stats).toEqual({ resumes: 2, tones: 2 })
  })
})
