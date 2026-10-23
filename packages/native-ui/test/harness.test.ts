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

  it('stops a script that passes the deadline inside a promise job, as the device does', async () => {
    // QuickJS turns the interrupt into a rejection there; the step still failed.
    for (const button of ['spin-in-job', 'spin-in-await']) {
      const h = await mounted('sandbox', { stepBudgetMs: 50 })
      await expect(h.press(button)).rejects.toBeInstanceOf(VmFailure)
      expect(h.failed).toEqual({ kind: 'watchdog', message: 'a script ran for more than 50 ms (jobs)' })
    }
  })

  it('stops a timer that runs out of memory, instead of logging it', async () => {
    const h = await mounted('sandbox', { memoryLimit: 8 * 1024 * 1024, stepBudgetMs: 20_000 })
    await expect(h.press('hog-in-timer')).rejects.toBeInstanceOf(VmFailure)
    expect(h.failed?.kind).toBe('memory')
    expect(h.exceptions).toEqual([])
  })

  it('stops when a host message does not fit, and when parsing it runs out of memory', async () => {
    const parse = await mounted('sandbox', { memoryLimit: 8 * 1024 * 1024, stepBudgetMs: 20_000 })
    // About 4 MB of text, which fits; its array does not.
    await expect(parse.env({ windowClass: 'wide', junk: new Array(2_000_000).fill(0) } as never)).rejects.toThrow(
      /memory: the VM ran out of memory \(deliver\)/,
    )
    expect(parse.logs.filter((l) => l.level === 'error')).toEqual([])
    const text = await mounted('sandbox', { memoryLimit: 8 * 1024 * 1024 })
    await expect(text.env({ windowClass: 'wide', junk: 'x'.repeat(10_000_000) } as never)).rejects.toThrow(
      /memory: the VM ran out of memory \(deliver\)/,
    )
  })

  it('logs a host message that is not JSON and goes on', async () => {
    const h = await mounted('sandbox')
    ;(h as unknown as { queue: { text: string }[] }).queue.push({ text: '{not json' })
    await h.settle()
    expect(h.failed).toBeNull()
    expect(h.logs.filter((l) => l.level === 'error').map((l) => l.message.split(' SyntaxError')[0])).toEqual([
      'native-ui: the host sent invalid JSON',
    ])
    await h.press('timers')
    expect(h.prop('ticks', 'text')).toBe('micro')
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
