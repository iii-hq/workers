import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { onCancel, waitForEvents } from './event-wait'

const subscribe = vi.fn()

vi.mock('@/lib/engine-trigger', () => ({
  subscribeEngineTrigger: (...args: unknown[]) => subscribe(...args),
}))

/** The handler of the most recent subscription. */
let deliver: (payload: unknown) => void = () => undefined

beforeEach(() => {
  vi.useFakeTimers()
  subscribe.mockReset()
  subscribe.mockImplementation(
    async (_type: string, _config: unknown, onEvent: typeof deliver) => {
      deliver = onEvent
      return () => undefined
    },
  )
})

afterEach(() => {
  vi.useRealTimers()
})

describe('waitForEvents', () => {
  it('coalesces a burst of events into one more check', async () => {
    let release: () => void = () => undefined
    const check = vi.fn(
      () =>
        new Promise<null>((resolve) => {
          release = () => resolve(null)
        }),
    )
    void waitForEvents({
      handler: 'test',
      triggers: [{ type: 't' }],
      check,
      timeoutMs: 60_000,
      onTimeout: () => undefined,
    })
    await vi.advanceTimersByTimeAsync(0)
    expect(check).toHaveBeenCalledTimes(1)
    deliver({})
    deliver({})
    deliver({})
    release()
    await vi.advanceTimersByTimeAsync(0)
    expect(check).toHaveBeenCalledTimes(2)
    release()
    await vi.advanceTimersByTimeAsync(0)
    expect(check).toHaveBeenCalledTimes(2)
  })

  it('still checks at start and after a silence when it cannot subscribe', async () => {
    subscribe.mockRejectedValue(new Error('no such trigger type'))
    const check = vi.fn(async () => null)
    const done = waitForEvents({
      handler: 'test',
      triggers: [{ type: 'missing' }],
      check,
      timeoutMs: 60_000,
      onTimeout: () => 'gave up',
      silenceMs: 10_000,
    })
    await vi.advanceTimersByTimeAsync(0)
    expect(check).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(10_000)
    expect(check).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(50_000)
    expect(check).toHaveBeenCalledTimes(2)
    await expect(done).resolves.toBe('gave up')
  })

  it('fails with the error an event handler throws', async () => {
    const done = waitForEvents({
      handler: 'test',
      triggers: [{ type: 't' }],
      onEvent: () => {
        throw new Error('bad event')
      },
      check: async () => null,
      timeoutMs: 60_000,
      onTimeout: () => undefined,
    })
    await vi.advanceTimersByTimeAsync(0)
    deliver({})
    await expect(done).rejects.toThrow('bad event')
  })
})

describe('onCancel', () => {
  it('fires when the flag flips and restores a plain property after', () => {
    const signal = { cancelled: false }
    const listener = vi.fn()
    const off = onCancel(signal, listener)
    signal.cancelled = true
    expect(listener).toHaveBeenCalledTimes(1)
    off()
    expect(Object.getOwnPropertyDescriptor(signal, 'cancelled')).toMatchObject({
      value: true,
      writable: true,
    })
  })

  it('leaves a frozen token alone', () => {
    const signal = Object.freeze({ cancelled: false })
    const off = onCancel(signal, () => undefined)
    expect(() => off()).not.toThrow()
  })
})
