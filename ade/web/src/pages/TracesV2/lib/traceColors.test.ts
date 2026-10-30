import { afterEach, describe, expect, it } from 'vitest'
import {
  getWorkerColor,
  resetWorkerColors,
  WORKER_PALETTE,
  workerSwatch,
} from './traceColors'

afterEach(() => resetWorkerColors())

/** Worker names whose hashes all land on the same preferred slot. */
function colliding(count: number): string[] {
  const out: string[] = []
  const target = workerSwatch('seed').index
  resetWorkerColors()
  for (let i = 0; out.length < count; i++) {
    const name = `worker-${i}`
    if (workerSwatch(name).index === target) out.push(name)
    resetWorkerColors()
  }
  return out
}

describe('worker colors', () => {
  it('keeps one colour per worker for the session', () => {
    const first = getWorkerColor('llm-router')
    getWorkerColor('state')
    getWorkerColor('judge')
    expect(getWorkerColor('llm-router')).toBe(first)
    expect(workerSwatch('llm-router').reused).toBe(false)
  })

  it('gives the first eight workers eight distinct hues even when their names collide', () => {
    const names = colliding(8)
    const indices = new Set(names.map((name) => workerSwatch(name).index))
    expect(indices.size).toBe(WORKER_PALETTE.length)
    for (const name of names) expect(workerSwatch(name).reused).toBe(false)
  })

  it('keeps a worker on its hashed hue while that hue is free', () => {
    const [a, b] = colliding(2)
    const preferred = workerSwatch(a).index
    resetWorkerColors()
    // Seen in the other order, the first one still takes the hashed slot.
    expect(workerSwatch(b).index).toBe(preferred)
    expect(workerSwatch(a).index).not.toBe(preferred)
  })

  it('flags the ninth worker as a reused hue and spreads reuse evenly', () => {
    const names = Array.from({ length: 12 }, (_, i) => `w${i}`)
    const swatches = names.map((name) => workerSwatch(name))
    expect(swatches.slice(0, 8).every((s) => !s.reused)).toBe(true)
    expect(swatches.slice(8).every((s) => s.reused)).toBe(true)
    // The four repeats land on four different hues.
    expect(new Set(swatches.slice(8).map((s) => s.index)).size).toBe(4)
  })

  it('returns palette tokens', () => {
    expect(WORKER_PALETTE).toContain(getWorkerColor('anything'))
  })
})
