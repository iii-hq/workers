import { afterEach, describe, expect, it, vi } from 'vitest'
import { fakeObserver } from './fake-observer.testing'
import {
  CATCH_UP_READY_WAIT_MS,
  catchUpWhenActive,
  whenObserversReady,
  withObservers,
} from './observer-catch-up'

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

afterEach(() => {
  vi.useRealTimers()
})

describe('catchUpWhenActive', () => {
  it('reads only once the observer is active, so an event missed while pending is caught', async () => {
    const { observer, set } = fakeObserver()
    // The state the read returns; an event that fires while the observer is
    // pending never reaches a handler.
    let current = 'running'
    const seen: string[] = []
    catchUpWhenActive(observer, (cause) => seen.push(`${cause}:${current}`))
    await flush()
    expect(seen).toEqual([])
    current = 'done' // the terminal event fires while pending: lost
    set('active')
    await flush()
    expect(seen).toEqual(['ready:done'])
  })

  it('reads again whenever the observer comes back active after a reconnect', async () => {
    const { observer, set } = fakeObserver()
    const read = vi.fn()
    catchUpWhenActive(observer, read)
    set('active')
    await flush()
    expect(read.mock.calls).toEqual([['ready']])
    set('active') // no transition
    set('suspended')
    expect(read).toHaveBeenCalledTimes(1)
    set('active')
    expect(read.mock.calls).toEqual([['ready'], ['reactivated']])
    set('suspended')
    set('pending')
    set('active')
    expect(read.mock.calls).toEqual([
      ['ready'],
      ['reactivated'],
      ['reactivated'],
    ])
  })

  it('waits for every observer, bounded: reads anyway after the wait, again on a late activation', async () => {
    vi.useFakeTimers()
    const a = fakeObserver('a')
    const b = fakeObserver('b')
    const read = vi.fn()
    catchUpWhenActive([a.observer, b.observer], read)
    a.set('active')
    await vi.advanceTimersByTimeAsync(CATCH_UP_READY_WAIT_MS - 1)
    expect(read).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(read.mock.calls).toEqual([['ready']])
    b.set('active')
    expect(read.mock.calls).toEqual([['ready'], ['reactivated']])
  })

  it('keeps the old behaviour for a disposer without state: one read, right away', () => {
    const read = vi.fn()
    catchUpWhenActive(() => {}, read)
    expect(read.mock.calls).toEqual([['ready']])
  })

  it('stops: no read after stop, listeners dropped', async () => {
    const fake = fakeObserver()
    const read = vi.fn()
    const stop = catchUpWhenActive(fake.observer, read)
    expect(fake.listeners).toBe(1)
    stop()
    expect(fake.listeners).toBe(0)
    fake.set('active')
    await flush()
    expect(read).not.toHaveBeenCalled()
  })
})

describe('whenObserversReady', () => {
  it('settles when a refused observer rejects ready()', async () => {
    const observer = Object.assign(() => {}, {
      state: 'error' as const,
      ready: () => Promise.reject(new Error('FORBIDDEN')),
      onStateChange: () => () => {},
    })
    await expect(whenObserversReady(observer)).resolves.toBeUndefined()
  })
})

describe('withObservers', () => {
  it('flattens nested subscriptions into one observer list', () => {
    const a = fakeObserver('a').observer
    const b = fakeObserver('b').observer
    const inner = withObservers(() => {}, [a])
    const outer = withObservers(() => {}, [inner, b])
    expect(outer.observers).toEqual([a, b])
  })
})
