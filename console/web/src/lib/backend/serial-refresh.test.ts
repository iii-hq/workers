import { describe, expect, it, vi } from 'vitest'
import { serialRefresh } from './serial-refresh'

function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const flush = () => new Promise((r) => setTimeout(r, 0))

describe('serialRefresh', () => {
  it('coalesces refreshes during a flight into one trailing rerun', async () => {
    const gates: Array<ReturnType<typeof deferred<number>>> = []
    const load = vi.fn(() => {
      const gate = deferred<number>()
      gates.push(gate)
      return gate.promise
    })
    const apply = vi.fn()
    const { refresh } = serialRefresh(load, apply)

    refresh()
    refresh()
    refresh()
    refresh()
    expect(load).toHaveBeenCalledTimes(1)

    gates[0].resolve(1)
    await flush()
    // The three extra signals collapsed into exactly one trailing rerun.
    expect(load).toHaveBeenCalledTimes(2)
    gates[1].resolve(2)
    await flush()
    expect(apply.mock.calls).toEqual([[1], [2]])
    expect(load).toHaveBeenCalledTimes(2)
  })

  it('never overlaps requests, so responses apply in order', async () => {
    const gates: Array<ReturnType<typeof deferred<string>>> = []
    const { refresh } = serialRefresh(
      () => {
        const gate = deferred<string>()
        gates.push(gate)
        return gate.promise
      },
      (v) => applied.push(v),
    )
    const applied: string[] = []

    refresh()
    refresh() // queued behind the first — not a concurrent request
    expect(gates).toHaveLength(1)
    gates[0].resolve('old')
    await flush()
    gates[1].resolve('new')
    await flush()
    expect(applied).toEqual(['old', 'new'])
  })

  it('reset discards the in-flight response and its queued rerun', async () => {
    const gate = deferred<string>()
    const load = vi.fn(() => gate.promise)
    const apply = vi.fn()
    const { refresh, reset } = serialRefresh(load, apply)

    refresh()
    refresh() // pending rerun
    reset()
    gate.resolve('stale')
    await flush()
    expect(apply).not.toHaveBeenCalled()
    expect(load).toHaveBeenCalledTimes(1)
  })

  it('keeps working after a load failure', async () => {
    let fail = true
    const apply = vi.fn()
    const { refresh } = serialRefresh(
      () => (fail ? Promise.reject(new Error('nope')) : Promise.resolve('ok')),
      apply,
    )
    refresh()
    await flush()
    fail = false
    refresh()
    await flush()
    expect(apply.mock.calls).toEqual([['ok']])
  })
})
