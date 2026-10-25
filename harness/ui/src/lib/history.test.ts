import { describe, expect, it } from 'vitest'
import { compactions, growth, stepPoints } from './history'

const assistant = (
  timestamp: number,
  usage: Record<string, number | undefined> | undefined,
) => ({
  entry_id: `e${timestamp}`,
  message: { role: 'assistant', usage, timestamp, model: 'm', provider: 'p' },
})
const user = (timestamp: number) => ({
  entry_id: `u${timestamp}`,
  message: { role: 'user', timestamp },
})

describe('stepPoints', () => {
  it('numbers assistant steps oldest first across pages, prompt = input + cache', () => {
    const page1 = {
      messages: [
        user(1),
        assistant(2, { input: 100, cache_read: 900, output: 10, cost_usd: 0.01 }),
      ],
    }
    const page2 = {
      messages: [assistant(4, { input: 50, cache_read: 1000, cache_write: 200, output: 20 })],
    }
    const points = stepPoints([page2, page1])
    expect(points).toEqual([
      { step: 1, prompt: 1000, output: 10, cost: 0.01, at: 2 },
      { step: 2, prompt: 1250, output: 20, cost: undefined, at: 4 },
    ])
  })

  it('skips steps without a prompt count and tolerates odd shapes', () => {
    const page = {
      messages: [assistant(1, undefined), assistant(2, { output: 5 }), null, 'x'],
    }
    expect(stepPoints([page, null, 'nope', { messages: 'no' }])).toEqual([])
  })

  it('reads a wrapped payload', () => {
    const page = { content: [], details: { messages: [assistant(1, { input: 10 })] } }
    expect(stepPoints([page]).map((p) => p.prompt)).toEqual([10])
  })
})

describe('compactions', () => {
  it('marks a step whose window shrank by more than the drop share', () => {
    const points = stepPoints([
      {
        messages: [
          assistant(1, { input: 100 }),
          assistant(2, { input: 150 }),
          assistant(3, { input: 60 }),
          assistant(4, { input: 55 }),
        ],
      },
    ])
    expect(compactions(points)).toEqual([2])
  })
})

describe('growth', () => {
  const series = (prompts: number[]) =>
    stepPoints([{ messages: prompts.map((p, i) => assistant(i + 1, { input: p })) }])

  it('needs two steps', () => {
    expect(growth(series([100]), 1000, 100)).toBeNull()
  })

  it('averages the recent positive deltas and projects the steps left', () => {
    const g = growth(series([100, 200, 300, 400]), 1000, 400)
    expect(g).toEqual({ perStep: 100, stepsLeft: 6 })
  })

  it('ignores compaction drops when measuring pace', () => {
    const g = growth(series([100, 200, 50, 150]), 1000, 150)
    expect(g).toEqual({ perStep: 100, stepsLeft: 8 })
  })

  it('reports no pace when the window never grew', () => {
    expect(growth(series([300, 300, 300]), 1000, 300)).toEqual({ perStep: 0, stepsLeft: null })
  })

  it('reports zero steps left when the window is already full', () => {
    expect(growth(series([900, 1000]), 1000, 1000)).toEqual({ perStep: 100, stepsLeft: 0 })
  })
})
