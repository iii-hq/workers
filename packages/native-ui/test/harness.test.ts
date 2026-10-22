/** The harness gives a bundle what the native VM gives it, and nothing more. */
import { describe, expect, it } from 'vitest'
import { createTestHost, VmFailure } from '../src/testing/index.mjs'
import { fixture, mounted } from './helpers'

describe('the VM sandbox', () => {
  it("has the host globals, no eval and no web APIs; __host_send is the SDK's alone", async () => {
    const h = await mounted('sandbox')
    expect(JSON.parse(String(h.prop('report', 'text')))).toEqual({
      hostSend: 'undefined',
      rawRequest: 'undefined',
      vmReceive: 'function',
      eval: 'EvalError',
      newFunction: 'EvalError',
      ctor: 'EvalError',
      asyncCtor: 'EvalError',
      missing: [],
      present: [
        'console',
        'setTimeout',
        'clearTimeout',
        'setInterval',
        'clearInterval',
        'queueMicrotask',
        'performance',
      ],
      regexp: true,
    })
  })

  it('runs timers on the fake clock, in due order, and microtasks in the same turn', async () => {
    const h = await mounted('sandbox')
    await h.press('timers')
    expect(h.prop('ticks', 'text')).toBe('micro')
    await h.tick(8)
    expect(h.prop('ticks', 'text')).toBe('micro,i,i')
    await h.tick(100)
    expect(h.prop('ticks', 'text')).toBe('micro,i,i,t50@50,t100@100')
  })

  it('stops a script that passes the step deadline', async () => {
    const h = await mounted('sandbox', { stepBudgetMs: 50 })
    await expect(h.press('spin')).rejects.toBeInstanceOf(VmFailure)
    expect(h.failed?.kind).toBe('watchdog')
  })

  it('stops a script that passes the heap cap', async () => {
    // Near its cap this wasm build of QuickJS slows down; the roomy step
    // budget keeps the test about the memory stop, not the deadline.
    const h = await mounted('sandbox', { memoryLimit: 8 * 1024 * 1024, stepBudgetMs: 20_000 })
    await expect(h.press('hog')).rejects.toBeInstanceOf(VmFailure)
    expect(h.failed?.kind).toBe('memory')
  })

  it('refuses a bundle that never calls defineApp', async () => {
    await expect(createTestHost({ bundle: 'globalThis.x = 1' })).rejects.toThrow(/didn't call defineApp/)
  })

  it('sends init with the first action, in one message with it', async () => {
    const h = await createTestHost({ bundle: await fixture('sandbox'), env: { windowClass: 'wide' } })
    expect(h.toVm).toEqual([])
    await h.mount('root')
    expect(h.toVm).toHaveLength(1)
    const [init, mount] = JSON.parse(h.toVm[0])
    expect(init[0]).toBe(1)
    expect(init[1].env.windowClass).toBe('wide')
    expect(init[1].app).toEqual({ key: 'worker:demo', ns: 'demo' })
    expect(mount).toEqual([6, 1, 'root', {}])
  })

  it('reports memory use', async () => {
    const h = await mounted('sandbox')
    expect(h.memory().memory_used_size).toBeGreaterThan(0)
  })
})
