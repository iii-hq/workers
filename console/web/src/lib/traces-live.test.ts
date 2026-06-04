import type { QueryClient } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { IIIConnectionState, IiiClient } from '@/lib/iii-client'
import {
  makeTracesChangedHandler,
  startTracesSubscription,
} from './traces-live'

function fakeQueryClient() {
  const invalidateQueries = vi.fn()
  return {
    client: { invalidateQueries } as unknown as QueryClient,
    invalidateQueries,
  }
}

function fakeClient() {
  const triggers: Array<{
    type: string
    function_id: string
    config?: unknown
  }> = []
  let spanHandler: ((p: unknown) => void) | null = null
  let connListener: ((s: IIIConnectionState) => void) | null = null
  const offSignal = vi.fn()
  const offConn = vi.fn()
  const triggerUnregister = vi.fn()

  const on = vi.fn((fn: string, handler: (p: unknown) => void) => {
    if (fn === 'iii::console::traces_changed') spanHandler = handler
    return offSignal
  })
  const registerTrigger = vi.fn(
    (input: { type: string; function_id: string; config?: unknown }) => {
      triggers.push(input)
      return triggerUnregister
    },
  )
  const addConnectionStateListener = vi.fn(
    (handler: (s: IIIConnectionState) => void) => {
      connListener = handler
      return offConn
    },
  )

  const client = {
    browserId: 'console-test',
    on,
    registerTrigger,
    addConnectionStateListener,
    call: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    registerTrigger,
    triggers,
    offSignal,
    offConn,
    triggerUnregister,
    fireSpan: () => spanHandler?.(undefined),
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

  it('runs the onExtra callback when not paused (e.g. reload open trace detail)', () => {
    const { client } = fakeQueryClient()
    const onExtra = vi.fn()
    const handler = makeTracesChangedHandler(
      client,
      { current: false },
      onExtra,
    )

    handler()

    expect(onExtra).toHaveBeenCalledTimes(1)
  })

  it('skips onExtra while paused', () => {
    const { client } = fakeQueryClient()
    const onExtra = vi.fn()
    const handler = makeTracesChangedHandler(client, { current: true }, onExtra)

    handler()

    expect(onExtra).not.toHaveBeenCalled()
  })
})

describe('startTracesSubscription', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('registers the span handler and binds a trace trigger on start', () => {
    const { client, on, triggers } = fakeClient()
    startTracesSubscription(client, () => {})

    expect(on).toHaveBeenCalledWith(
      'iii::console::traces_changed',
      expect.any(Function),
    )
    expect(triggers).toEqual([
      {
        type: 'trace',
        function_id: 'iii::console::traces_changed::console-test',
        config: {},
      },
    ])
  })

  it('coalesces a burst of per-span ticks into a single refetch', () => {
    const { client, fireSpan } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal, { coalesceMs: 400 })

    fireSpan()
    fireSpan()
    fireSpan()
    expect(onSignal).not.toHaveBeenCalled() // still debouncing

    vi.advanceTimersByTime(400)
    expect(onSignal).toHaveBeenCalledTimes(1)
  })

  it('fans out again on a span that arrives after the window flushed', () => {
    const { client, fireSpan } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal, { coalesceMs: 400 })

    fireSpan()
    vi.advanceTimersByTime(400)
    fireSpan()
    vi.advanceTimersByTime(400)

    expect(onSignal).toHaveBeenCalledTimes(2)
  })

  it('re-syncs (refetches) on reconnect without re-registering the trigger', () => {
    const { client, registerTrigger, fireConn } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal)

    fireConn('connected')

    // SDK replays the registered trigger itself — we must not double-register.
    expect(registerTrigger).toHaveBeenCalledTimes(1)
    expect(onSignal).toHaveBeenCalledTimes(1)
  })

  it('does not re-sync on non-connected transitions', () => {
    const { client, fireConn } = fakeClient()
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal)

    fireConn('reconnecting')
    fireConn('disconnected')

    expect(onSignal).not.toHaveBeenCalled()
  })

  it('cleans up the handler, listener, trigger, and pending timer on stop', () => {
    const { client, offSignal, offConn, triggerUnregister, fireSpan } =
      fakeClient()
    const onSignal = vi.fn()
    const stop = startTracesSubscription(client, onSignal, { coalesceMs: 400 })

    fireSpan() // arm the debounce timer
    stop()
    vi.advanceTimersByTime(400)

    expect(offSignal).toHaveBeenCalledTimes(1)
    expect(offConn).toHaveBeenCalledTimes(1)
    expect(triggerUnregister).toHaveBeenCalledTimes(1)
    expect(onSignal).not.toHaveBeenCalled() // timer was cleared
  })

  it('re-syncs when the tab becomes visible again', () => {
    const { client } = fakeClient()
    const doc = fakeDoc('visible')
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal, { doc: doc.doc })

    doc.fireVisibilityChange()

    expect(onSignal).toHaveBeenCalledTimes(1)
  })

  it('does not re-sync on a visibilitychange that leaves the tab hidden', () => {
    const { client } = fakeClient()
    const doc = fakeDoc('hidden')
    const onSignal = vi.fn()
    startTracesSubscription(client, onSignal, { doc: doc.doc })

    doc.fireVisibilityChange()

    expect(onSignal).not.toHaveBeenCalled()
  })

  it('removes the visibilitychange listener on stop', () => {
    const { client } = fakeClient()
    const doc = fakeDoc('visible')
    const stop = startTracesSubscription(client, () => {}, { doc: doc.doc })

    stop()

    expect(doc.removeEventListener).toHaveBeenCalledWith(
      'visibilitychange',
      expect.any(Function),
    )
  })
})
