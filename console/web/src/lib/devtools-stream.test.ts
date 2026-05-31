import type { QueryClient } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'
import type { IIIConnectionState, IiiClient } from '@/lib/iii-client'
import {
  makeTracesChangedHandler,
  startTracesSubscription,
} from './devtools-stream'

function fakeQueryClient() {
  const invalidateQueries = vi.fn()
  return {
    client: { invalidateQueries } as unknown as QueryClient,
    invalidateQueries,
  }
}

function fakeClient() {
  const calls: Array<{ fn: string; payload: unknown }> = []
  let signalHandler: ((p: unknown) => void) | null = null
  let connListener: ((s: IIIConnectionState) => void) | null = null
  const offSignal = vi.fn()
  const offConn = vi.fn()
  const on = vi.fn((fn: string, handler: (p: unknown) => void) => {
    if (fn === 'ui::traces::changed') signalHandler = handler
    return offSignal
  })
  const call = vi.fn(async (fn: string, payload?: Record<string, unknown>) => {
    calls.push({ fn, payload })
    return null
  })
  const addConnectionStateListener = vi.fn(
    (handler: (s: IIIConnectionState) => void) => {
      connListener = handler
      return offConn
    },
  )
  const client = {
    browserId: 'console-test',
    on,
    call,
    addConnectionStateListener,
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient
  return {
    client,
    on,
    call,
    calls,
    offSignal,
    offConn,
    fireSignal: () => signalHandler?.(undefined),
    fireConn: (s: IIIConnectionState) => connListener?.(s),
  }
}

function fakeDoc(initial: 'visible' | 'hidden' = 'visible') {
  let visibilityState = initial
  let handler: (() => void) | null = null
  const addEventListener = vi.fn((type: string, h: () => void) => {
    if (type === 'visibilitychange') handler = h
  })
  const removeEventListener = vi.fn()
  const doc = {
    get visibilityState() {
      return visibilityState
    },
    addEventListener,
    removeEventListener,
  }
  return {
    doc: doc as unknown as Document,
    addEventListener,
    removeEventListener,
    setVisibility: (s: 'visible' | 'hidden') => {
      visibilityState = s
    },
    fireVisibilityChange: () => handler?.(),
  }
}

describe('makeTracesChangedHandler', () => {
  it('invalidates both trace query keys when not paused', () => {
    const { client, invalidateQueries } = fakeQueryClient()
    const handler = makeTracesChangedHandler(client, { current: false })

    handler()

    expect(invalidateQueries).toHaveBeenCalledTimes(2)
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['traces'] })
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: ['traceGroups'],
    })
  })

  it('does nothing while paused', () => {
    const { client, invalidateQueries } = fakeQueryClient()
    const handler = makeTracesChangedHandler(client, { current: true })

    handler()

    expect(invalidateQueries).not.toHaveBeenCalled()
  })

  it('reads the pause flag live from the ref', () => {
    const { client, invalidateQueries } = fakeQueryClient()
    const ref = { current: true }
    const handler = makeTracesChangedHandler(client, ref)

    handler()
    expect(invalidateQueries).not.toHaveBeenCalled()

    ref.current = false
    handler()
    expect(invalidateQueries).toHaveBeenCalledTimes(2)
  })
})

describe('startTracesSubscription', () => {
  it('registers the signal handler and subscribes to all sessions on start', () => {
    const { client, on, calls } = fakeClient()
    startTracesSubscription(client, () => {})

    expect(on).toHaveBeenCalledWith('ui::traces::changed', expect.any(Function))
    expect(calls).toEqual([
      {
        fn: 'ui::subscribe',
        payload: { browser_id: 'console-test', session_id: null },
      },
    ])
  })

  it('routes the pushed signal to the provided callback', () => {
    const { client, fireSignal } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal)

    fireSignal()

    expect(onSignal).toHaveBeenCalledTimes(1)
  })

  it('re-subscribes when the socket reconnects', () => {
    const { client, calls, fireConn } = fakeClient()
    startTracesSubscription(client, () => {})

    fireConn('connected')

    const subscribes = calls.filter((c) => c.fn === 'ui::subscribe')
    expect(subscribes.length).toBeGreaterThanOrEqual(2)
    for (const s of subscribes) {
      expect(s.payload).toEqual({
        browser_id: 'console-test',
        session_id: null,
      })
    }
  })

  it('re-syncs (refetches) on reconnect so a blank initial fetch recovers', () => {
    const { client, fireConn } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal)

    fireConn('connected')

    expect(onSignal).toHaveBeenCalledTimes(1)
  })

  it('does not re-subscribe or re-sync on non-connected transitions', () => {
    const { client, calls, fireConn } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal)

    fireConn('reconnecting')
    fireConn('disconnected')

    expect(calls.filter((c) => c.fn === 'ui::subscribe')).toHaveLength(1)
    expect(onSignal).not.toHaveBeenCalled()
  })

  it('cleans up the handler, the listener, and unsubscribes on stop', () => {
    const { client, calls, offSignal, offConn } = fakeClient()
    const stop = startTracesSubscription(client, () => {})

    stop()

    expect(offSignal).toHaveBeenCalledTimes(1)
    expect(offConn).toHaveBeenCalledTimes(1)
    expect(calls).toContainEqual({
      fn: 'ui::unsubscribe',
      payload: { browser_id: 'console-test', session_id: null },
    })
  })

  it('re-syncs when the tab becomes visible again (recovers signals dropped while hidden)', () => {
    const { client } = fakeClient()
    const doc = fakeDoc('visible')
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal, doc.doc)

    doc.fireVisibilityChange()

    expect(onSignal).toHaveBeenCalledTimes(1)
  })

  it('does not re-sync on a visibilitychange that leaves the tab hidden', () => {
    const { client } = fakeClient()
    const doc = fakeDoc('hidden')
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal, doc.doc)

    doc.fireVisibilityChange()

    expect(onSignal).not.toHaveBeenCalled()
  })

  it('removes the visibilitychange listener on stop', () => {
    const { client } = fakeClient()
    const doc = fakeDoc('visible')
    const stop = startTracesSubscription(client, () => {}, doc.doc)

    stop()

    expect(doc.removeEventListener).toHaveBeenCalledWith(
      'visibilitychange',
      expect.any(Function),
    )
  })
})
