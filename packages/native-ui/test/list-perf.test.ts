/**
 * createList().set on 3,000 items with one changed, timed in the harness's
 * QuickJS (Date.now in the VM): unchanged items are skipped by identity, and
 * a fresh copy of the data is compared field by field, faster than the JSON
 * of each pair that it replaced (about 26 ms here for both cases).
 */
import { describe, expect, it } from 'vitest'
import { mounted } from './helpers'

const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[xs.length >> 1]

describe('createList().set on 3,000 items', () => {
  it('skips the unchanged items and sends one splice of the changed one', async () => {
    const h = await mounted('set3000', { stepBudgetMs: 10_000 })
    const times: number[] = []
    for (let i = 0; i < 5; i++) {
      const from = h.messages.length
      await h.emitTrigger('demo::one', {})
      times.push(Number(h.prop('ms', 'text')))
      const ops = h.messages.slice(from).flatMap((m) => JSON.parse(m) as unknown[][])
      expect(ops.filter((o) => o[0] === 8).map((o) => o.slice(2, 4))).toEqual([[1500, 1]])
    }
    // Measured 2 ms (it was 26 with the JSON of every pair).
    expect(median(times)).toBeLessThan(10)
  })

  it('compares a fresh copy of every item faster than their JSON', async () => {
    const h = await mounted('set3000', { stepBudgetMs: 10_000 })
    const fresh: number[] = []
    const json: number[] = []
    for (let i = 0; i < 5; i++) {
      await h.emitTrigger('demo::fresh', {})
      fresh.push(Number(h.prop('ms', 'text')))
      await h.emitTrigger('demo::json', {})
      json.push(Number(h.prop('ms', 'text')))
    }
    expect(h.keys('tickets')).toHaveLength(3000)
    expect((h.items('tickets')[1500] as { title: string }).title).toBe('changed 5')
    // Measured 14 ms against 26 (the whole set, against the comparison alone).
    expect(median(fresh)).toBeLessThan(median(json))
  })
})
