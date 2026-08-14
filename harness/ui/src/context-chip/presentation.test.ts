import { describe, expect, it } from 'vitest'
import type { ContextSnapshot } from '../lib/metrics'
import {
  accountingProvenance,
  outputUsagePresentation,
  promptUsagePresentation,
} from './presentation'

const snapshot = (overrides: Partial<ContextSnapshot> = {}): ContextSnapshot => ({
  session_id: 's_1',
  turn_id: 't_1',
  step: 31,
  model: 'glm-5.3',
  provider: 'zai',
  estimator: 'tokenizer',
  usable: 948_000,
  effective_max_output_tokens: 32_000,
  total: 86_280,
  free: 861_720,
  categories: {
    system_prompt: 7_005,
    tools: 75,
    messages: {
      user: 79_200,
      assistant: 0,
      function_result: 0,
      custom: 0,
    },
    overhead: 0,
  },
  compacted: false,
  timestamp: 1,
  usage: {
    input: 1_544,
    output: 1_141,
    cache_read: 84_736,
    reasoning: 41,
  },
  ...overrides,
})

describe('accountingProvenance', () => {
  it('separates the provider prompt total from the local tokenizer breakdown', () => {
    expect(accountingProvenance(snapshot())).toBe(
      'prompt total: provider usage · breakdown: local tokenizer',
    )
  })

  it('separates provider totals from heuristic and pre-response breakdowns', () => {
    expect(
      accountingProvenance(snapshot({ estimator: 'heuristic' })),
    ).toBe('prompt total: provider usage · breakdown: chars/4')
    expect(
      accountingProvenance(
        snapshot({ estimator: 'heuristic', usage: undefined }),
      ),
    ).toBe('prompt total: estimated · breakdown: chars/4')
    expect(
      accountingProvenance(snapshot({ estimator: 'provider' })),
    ).toBe('prompt total: provider usage · breakdown: provider tokenizer')
  })
})

describe('promptUsagePresentation', () => {
  it('reconciles the live GLM-5.3 fresh and cached prompt usage', () => {
    expect(promptUsagePresentation(snapshot().usage)).toEqual({
      fresh: 1_544,
      cached: 84_736,
      cacheCreation: undefined,
      hitPct: 98,
      total: 86_280,
    })
  })

  it('keeps an absent cache-write metric distinct from a reported zero', () => {
    expect(promptUsagePresentation({ input: 10, cache_read: 5 })).toEqual({
      fresh: 10,
      cached: 5,
      cacheCreation: undefined,
      hitPct: 33,
      total: 15,
    })
    expect(
      promptUsagePresentation({ input: 10, cache_read: 5, cache_write: 0 }),
    ).toEqual({
      fresh: 10,
      cached: 5,
      cacheCreation: 0,
      hitPct: 33,
      total: 15,
    })
  })

  it('does not invent a cache-hit percentage when cache reads were not reported', () => {
    expect(promptUsagePresentation({ input: 10, cache_write: 5 })).toEqual({
      fresh: 10,
      cached: undefined,
      cacheCreation: 5,
      hitPct: undefined,
      total: 15,
    })
  })

  it('does not invent prompt usage when the provider reported none', () => {
    expect(promptUsagePresentation(undefined)).toBeNull()
    expect(promptUsagePresentation({ output: 12 })).toBeNull()
  })
})

describe('outputUsagePresentation', () => {
  it('keeps reasoning as a reported subset of output', () => {
    expect(outputUsagePresentation(snapshot().usage)).toEqual({
      output: 1_141,
      reasoning: 41,
    })
  })

  it('omits absent and zero reasoning without inventing output', () => {
    expect(outputUsagePresentation({ output: 12 })).toEqual({
      output: 12,
      reasoning: undefined,
    })
    expect(outputUsagePresentation({ output: 12, reasoning: 0 })).toEqual({
      output: 12,
      reasoning: undefined,
    })
    expect(outputUsagePresentation({ reasoning: 4 })).toBeNull()
  })
})
