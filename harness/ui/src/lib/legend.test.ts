import { describe, expect, it } from 'vitest'
import { formatCost } from './format'
import { categories, legendRows, percentOf } from './legend'
import type { ContextSnapshot } from './metrics'

function snapshot(overrides: Partial<ContextSnapshot> = {}): ContextSnapshot {
  return {
    session_id: 's',
    turn_id: 't',
    step: 1,
    model: 'm',
    usable: 852_000,
    effective_max_output_tokens: 0,
    total: 112_868,
    free: 739_132,
    categories: {
      system_prompt: 24_674,
      skills: 5617,
      tools: 636,
      messages: { user: 2137, assistant: 40_750, function_result: 39_054, custom: 0 },
      overhead: 0,
      hook_guidance: 0,
    },
    compacted: false,
    timestamp: 0,
    ...overrides,
  }
}

const rowsOf = (snap: ContextSnapshot) =>
  legendRows(snap, categories(snap)).map((row) => [row.key, row.tokens, row.group ?? row.sub ?? false])

describe('legendRows', () => {
  it('groups the conversation parts under a total and drops empty rows', () => {
    expect(rowsOf(snapshot())).toEqual([
      ['system', 24_674, false],
      ['skills', 5617, false],
      ['tools', 636, false],
      ['conversation', 81_941, true],
      ['user', 2137, true],
      ['assistant', 40_750, true],
      ['results', 39_054, true],
      // hooks and overhead are 0: no rows
      ['free', 739_132, false],
    ])
  })

  it('keeps the compaction summary before overhead and skips empty parts', () => {
    const snap = snapshot({ compacted: true, summarized_head_tokens: 900 })
    snap.categories.messages.user = 0
    snap.categories.overhead = 74
    const keys = legendRows(snap, categories(snap)).map((row) => row.key)
    expect(keys).toEqual(['system', 'skills', 'tools', 'conversation', 'assistant', 'results', 'summary', 'overhead', 'free'])
  })
})

describe('percentOf', () => {
  it('never shows 0% for a non-empty sliver', () => {
    expect(percentOf(636, 852_000)).toBe('<1%')
    expect(percentOf(0, 852_000)).toBe('0%')
    expect(percentOf(81_941, 852_000)).toBe('10%')
    expect(percentOf(5, 0)).toBe('0%')
  })
})

describe('formatCost', () => {
  it('uses cents from a dollar up and four places below', () => {
    expect(formatCost(1.4050392)).toBe('$1.41')
    expect(formatCost(0.0530132)).toBe('$0.0530')
  })
})
