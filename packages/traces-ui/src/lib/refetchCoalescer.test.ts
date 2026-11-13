import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createRefetchCoalescer } from './refetchCoalescer'

interface Deferred {
  resolve: () => void
  reject: (error: unknown) => void
}

function makeRun() {
  const pending: Deferred[] = []
  const run = vi.fn(
    () =>
      new Promise<void>((resolve, reject) => {
        pending.push({ resolve, reject })
      }),
  )
  return { run, pending }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('createRefetchCoalescer', () => {
  it('collapses a burst of notifications into one refetch after the debounce', () => {
    const { run } = makeRun()
    const c = createRefetchCoalescer({ run, debounceMs: 250, minIntervalMs: 0 })
    c.request()
    c.request()
    vi.advanceTimersByTime(100)
    c.request()
    expect(run).not.toHaveBeenCalled()
    vi.advanceTimersByTime(250)
    expect(run).toHaveBeenCalledTimes(1)
  })

  it('never overlaps refetches: a mid-flight notification runs exactly once, after settle', async () => {
    const { run, pending } = makeRun()
    const c = createRefetchCoalescer({ run, debounceMs: 250, minIntervalMs: 0 })
    c.request()
    vi.advanceTimersByTime(250)
    expect(run).toHaveBeenCalledTimes(1)
    expect(c.inFlight).toBe(true)

    // ticks keep landing while the engine is still answering
    c.request()
    c.request()
    vi.advanceTimersByTime(2_000)
    expect(run).toHaveBeenCalledTimes(1)

    pending[0].resolve()
    await vi.advanceTimersByTimeAsync(250)
    expect(run).toHaveBeenCalledTimes(2)
    expect(c.inFlight).toBe(true)

    // nothing arrived during the rerun: it settles and stays quiet
    pending[1].resolve()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(run).toHaveBeenCalledTimes(2)
    expect(c.inFlight).toBe(false)
  })

  it('spaces consecutive starts by the minimum interval', async () => {
    const { run, pending } = makeRun()
    const c = createRefetchCoalescer({
      run,
      debounceMs: 250,
      minIntervalMs: 1_000,
    })
    c.request()
    vi.advanceTimersByTime(250)
    expect(run).toHaveBeenCalledTimes(1)
    pending[0].resolve()
    await vi.advanceTimersByTimeAsync(0)

    // a fast answer plus an immediate tick: the floor (1s from the previous
    // START), not the debounce, paces the next refetch
    c.request()
    await vi.advanceTimersByTimeAsync(900)
    expect(run).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(100)
    expect(run).toHaveBeenCalledTimes(2)
  })

  it('reads a dynamic minimum interval when arming', async () => {
    const { run, pending } = makeRun()
    let floor = 0
    const c = createRefetchCoalescer({
      run,
      debounceMs: 100,
      minIntervalMs: () => floor,
    })
    c.request()
    await vi.advanceTimersByTimeAsync(100)
    pending[0].resolve()
    await vi.advanceTimersByTimeAsync(0)
    floor = 5_000
    c.request()
    await vi.advanceTimersByTimeAsync(4_000)
    expect(run).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1_000)
    expect(run).toHaveBeenCalledTimes(2)
  })

  it('drops a refetch the gate refuses and accepts a later one', () => {
    const { run } = makeRun()
    let open = false
    const c = createRefetchCoalescer({
      run,
      debounceMs: 100,
      minIntervalMs: 0,
      shouldRun: () => open,
    })
    c.request()
    vi.advanceTimersByTime(100)
    expect(run).not.toHaveBeenCalled()
    open = true
    c.request()
    vi.advanceTimersByTime(100)
    expect(run).toHaveBeenCalledTimes(1)
  })

  it('frees the slot when the refetch rejects', async () => {
    const { run, pending } = makeRun()
    const c = createRefetchCoalescer({ run, debounceMs: 100, minIntervalMs: 0 })
    c.request()
    vi.advanceTimersByTime(100)
    pending[0].reject(new Error('engine busy'))
    await vi.advanceTimersByTimeAsync(0)
    expect(c.inFlight).toBe(false)
    c.request()
    await vi.advanceTimersByTimeAsync(100)
    expect(run).toHaveBeenCalledTimes(2)
  })

  it('does nothing after dispose, including the trailing rerun', async () => {
    const { run, pending } = makeRun()
    const c = createRefetchCoalescer({ run, debounceMs: 100, minIntervalMs: 0 })
    c.request()
    vi.advanceTimersByTime(100)
    c.request() // would be the trailing rerun
    c.dispose()
    pending[0].resolve()
    await vi.advanceTimersByTimeAsync(1_000)
    c.request()
    await vi.advanceTimersByTimeAsync(1_000)
    expect(run).toHaveBeenCalledTimes(1)
  })
})
