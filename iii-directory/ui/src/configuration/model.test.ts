import { describe, expect, it } from 'vitest'
import {
  booleanWithDefault,
  functionSearchModeWithDefault,
  judgeQuestionWithDefault,
  semanticModeNeedsModel,
  withFunctionSearchMode,
  withoutRetiredKeys,
} from './model'

describe('booleanWithDefault', () => {
  it('uses the worker default when a migrated value omits the field', () => {
    expect(booleanWithDefault(undefined, false)).toBe(false)
    expect(booleanWithDefault(undefined, true)).toBe(true)
  })

  it('preserves explicit boolean values', () => {
    expect(booleanWithDefault(true, false)).toBe(true)
    expect(booleanWithDefault(false, true)).toBe(false)
  })
})

describe('function search configuration', () => {
  it('defaults the judge question to noul unless choice is stored', () => {
    expect(judgeQuestionWithDefault(undefined)).toBe('noul')
    expect(judgeQuestionWithDefault('yes_no')).toBe('noul')
    expect(judgeQuestionWithDefault('choice')).toBe('choice')
  })

  it('uses judge when a migrated value omits or corrupts the mode', () => {
    expect(functionSearchModeWithDefault(undefined)).toBe('judge')
    expect(functionSearchModeWithDefault('remote')).toBe('judge')
    expect(functionSearchModeWithDefault('jev')).toBe('judge')
  })

  it.each(['lexical', 'hybrid', 'judge'] as const)('preserves the supported %s mode', (mode) => {
    expect(functionSearchModeWithDefault(mode)).toBe(mode)
  })

  it('changes only the mode in the configuration draft', () => {
    expect(
      withFunctionSearchMode(
        {
          function_search_mode: 'lexical',
          function_search_model_path: '/models/minilm',
          registry_search: true,
        },
        'hybrid',
      ),
    ).toEqual({
      function_search_mode: 'hybrid',
      function_search_model_path: '/models/minilm',
      registry_search: true,
    })
  })

  it('requires a configured local model only for hybrid', () => {
    expect(semanticModeNeedsModel('lexical', undefined)).toBe(false)
    expect(semanticModeNeedsModel('lexical', null)).toBe(false)
    expect(semanticModeNeedsModel('judge', undefined)).toBe(false)
    expect(semanticModeNeedsModel('judge', null)).toBe(false)
    expect(semanticModeNeedsModel('judge', '/models/minilm')).toBe(false)
    // Absent field = worker default bundle path + first-run download.
    expect(semanticModeNeedsModel('hybrid', undefined)).toBe(false)
    // Explicit null disables the semantic lane: that is the stranded case.
    expect(semanticModeNeedsModel('hybrid', null)).toBe(true)
    expect(semanticModeNeedsModel('hybrid', '/models/minilm')).toBe(false)
  })
})

describe('withoutRetiredKeys', () => {
  it('drops keys the worker no longer reads, including the old TypeSafe key', () => {
    expect(
      withoutRetiredKeys({
        function_search_mode: 'judge',
        function_search_jev_api_key: 'old-secret',
        function_search_jev_model: 'jev-1.13.0',
        function_search_judge_timeout_ms: 3000,
      }),
    ).toEqual({ function_search_mode: 'judge', function_search_judge_timeout_ms: 3000 })
  })
})
