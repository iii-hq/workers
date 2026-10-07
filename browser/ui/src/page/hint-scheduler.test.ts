import { describe, expect, it, vi } from 'vitest'
import { createHintScheduler, type HintPoint } from './hint-scheduler'

/** A manual frame queue standing in for requestAnimationFrame. */
function frames() {
  const queue = new Map<number, () => void>()
  let next = 1
  return {
    frame: (callback: () => void) => {
      const id = next++
      queue.set(id, callback)
      return id
    },
    cancelFrame: (id: number) => {
      queue.delete(id)
    },
    get pending() {
      return queue.size
    },
    run() {
      const callbacks = [...queue.values()]
      queue.clear()
      for (const callback of callbacks) callback()
    },
  }
}

/** A request whose answers the test releases by hand. */
function requests() {
  const calls: { point: HintPoint; resolve: (v: string) => void; reject: () => void }[] = []
  return {
    calls,
    request: (x: number, y: number) =>
      new Promise<string>((resolve, reject) => {
        calls.push({ point: { x, y }, resolve, reject: () => reject(new Error('x')) })
      }),
  }
}

const flush = () => new Promise((r) => setTimeout(r, 0))

describe('createHintScheduler', () => {
  it('asks once per frame, for the newest point', async () => {
    const f = frames()
    const r = requests()
    const onResult = vi.fn()
    const s = createHintScheduler({ request: r.request, onResult, ...f })
    s.move({ x: 1, y: 1 })
    s.move({ x: 2, y: 2 })
    s.move({ x: 3, y: 3 })
    expect(f.pending).toBe(1)
    expect(r.calls).toHaveLength(0)
    f.run()
    expect(r.calls.map((c) => c.point)).toEqual([{ x: 3, y: 3 }])
    r.calls[0]?.resolve('div')
    await flush()
    expect(onResult).toHaveBeenCalledWith('div', { x: 3, y: 3 })
  })

  it('does nothing while the pointer rests', async () => {
    const f = frames()
    const r = requests()
    const s = createHintScheduler({ request: r.request, onResult: () => {}, ...f })
    s.move({ x: 5, y: 5 })
    f.run()
    r.calls[0]?.resolve('a')
    await flush()
    // The same point again (a jitter that maps to the same page pixel), and
    // then nothing at all: no frame, no request.
    s.move({ x: 5, y: 5 })
    expect(f.pending).toBe(0)
    await flush()
    expect(r.calls).toHaveLength(1)
  })

  it('keeps one request in flight and follows up with the newest point', async () => {
    const f = frames()
    const r = requests()
    const s = createHintScheduler({ request: r.request, onResult: () => {}, ...f })
    s.move({ x: 1, y: 1 })
    f.run()
    s.move({ x: 2, y: 2 })
    s.move({ x: 4, y: 4 })
    // Still answering the first: nothing queued meanwhile.
    expect(f.pending).toBe(0)
    r.calls[0]?.reject()
    await flush()
    expect(f.pending).toBe(1)
    f.run()
    expect(r.calls.map((c) => c.point)).toEqual([
      { x: 1, y: 1 },
      { x: 4, y: 4 },
    ])
  })

  it('reports a failed request as null and asks again after leaving', async () => {
    const f = frames()
    const r = requests()
    const onResult = vi.fn()
    const s = createHintScheduler({ request: r.request, onResult, ...f })
    s.move({ x: 7, y: 7 })
    f.run()
    r.calls[0]?.reject()
    await flush()
    expect(onResult).toHaveBeenCalledWith(null, { x: 7, y: 7 })
    s.move(null)
    s.move({ x: 7, y: 7 })
    f.run()
    expect(r.calls).toHaveLength(2)
  })

  it('stops on dispose', async () => {
    const f = frames()
    const r = requests()
    const onResult = vi.fn()
    const s = createHintScheduler({ request: r.request, onResult, ...f })
    s.move({ x: 1, y: 1 })
    f.run()
    s.move({ x: 2, y: 2 })
    s.dispose()
    r.calls[0]?.resolve('late')
    await flush()
    expect(onResult).not.toHaveBeenCalled()
    expect(f.pending).toBe(0)
  })
})
