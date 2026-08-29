import { describe, expect, it } from 'vitest'
import { singleFlight, waitForHttp } from '../src/lifecycle.js'

// Rejects only when the caller aborts, so the probe hangs forever without a
// deadline of its own - the shape of a dashboard that accepts the connection
// and then never answers.
const hangingFetch = ((_url: string, init?: RequestInit) =>
  new Promise((_resolve, reject) => {
    init?.signal?.addEventListener('abort', () => reject(new Error('aborted')))
  })) as unknown as typeof fetch

describe('waitForHttp', () => {
  it('gives up at the deadline even when a probe never answers', async () => {
    const outcome = await waitForHttp({
      url: 'http://127.0.0.1:1/',
      timeoutMs: 60,
      intervalMs: 10,
      exited: () => false,
      fetch: hangingFetch,
    })
    expect(outcome).toBe('timeout')
  })

  it('never sleeps past the deadline between probes', async () => {
    const slept: number[] = []
    let clock = 0
    const outcome = await waitForHttp({
      url: 'http://127.0.0.1:1/',
      timeoutMs: 60,
      intervalMs: 250,
      exited: () => false,
      fetch: (async () => new Response(null, { status: 503 })) as unknown as typeof fetch,
      now: () => clock,
      sleep: async (ms) => {
        slept.push(ms)
        clock += ms
      },
    })
    expect(outcome).toBe('timeout')
    expect(slept).toEqual([60])
  })

  it('reports ready as soon as a probe answers', async () => {
    const outcome = await waitForHttp({
      url: 'http://127.0.0.1:1/',
      timeoutMs: 1_000,
      exited: () => false,
      fetch: (async () => new Response(null, { status: 200 })) as unknown as typeof fetch,
    })
    expect(outcome).toBe('ready')
  })
})

describe('singleFlight', () => {
  it('shares one run between overlapping callers', async () => {
    let runs = 0
    let release: (() => void) | null = null
    const start = singleFlight(async () => {
      runs += 1
      await new Promise<void>((done) => {
        release = done
      })
      return runs
    })

    const first = start()
    const second = start()
    release?.()

    expect(await first).toBe(1)
    expect(await second).toBe(1)
    expect(runs).toBe(1)
  })

  it('runs again once the previous run settles', async () => {
    let runs = 0
    const start = singleFlight(async () => {
      runs += 1
      return runs
    })

    expect(await start()).toBe(1)
    expect(await start()).toBe(2)
  })

  it('clears the in-flight run after a failure so the next caller retries', async () => {
    let runs = 0
    const start = singleFlight(async () => {
      runs += 1
      throw new Error(`run ${runs} failed`)
    })

    await expect(start()).rejects.toThrow('run 1 failed')
    await expect(start()).rejects.toThrow('run 2 failed')
  })
})
