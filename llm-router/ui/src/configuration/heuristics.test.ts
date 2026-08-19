import { describe, expect, it } from 'vitest'
import { moveItem, winningHeuristicIndex } from './heuristics'

describe('winningHeuristicIndex', () => {
  const rows = [
    { pattern: '^gpt-', provider: 'openai' },
    { pattern: 'claude', provider: 'anthropic' },
    { pattern: '(', provider: 'broken' },
    { pattern: '', provider: 'openai' },
  ]

  it('returns the first regex that matches the model id', () => {
    expect(winningHeuristicIndex('gpt-4.1', rows)).toBe(0)
    expect(winningHeuristicIndex('claude-sonnet-4', rows)).toBe(1)
  })

  it('skips invalid regex and empty patterns', () => {
    expect(winningHeuristicIndex('nope', rows)).toBe(null)
  })

  it('returns null when the model id is empty', () => {
    expect(winningHeuristicIndex('', rows)).toBe(null)
    expect(winningHeuristicIndex('   ', rows)).toBe(null)
  })
})

describe('moveItem', () => {
  it('moves a row up or down and no-ops out of range', () => {
    expect(moveItem(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b'])
    expect(moveItem(['a', 'b', 'c'], 0, 1)).toEqual(['b', 'a', 'c'])
    expect(moveItem(['a', 'b', 'c'], 0, -1)).toEqual(['a', 'b', 'c'])
    expect(moveItem(['a', 'b', 'c'], 0, 3)).toEqual(['a', 'b', 'c'])
  })
})
