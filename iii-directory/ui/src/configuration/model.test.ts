import { describe, expect, it } from 'vitest'
import {
  booleanWithDefault,
  functionSearchModeWithDefault,
  semanticModeNeedsModel,
  withFunctionSearchMode,
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
  it('uses lexical when a migrated value omits or corrupts the mode', () => {
    expect(functionSearchModeWithDefault(undefined)).toBe('hybrid')
    expect(functionSearchModeWithDefault('remote')).toBe('hybrid')
  })

  it.each(['lexical', 'shadow', 'hybrid'] as const)('preserves the supported %s mode', (mode) => {
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

  it('requires a configured model only for semantic modes', () => {
    expect(semanticModeNeedsModel('lexical', undefined)).toBe(false)
    expect(semanticModeNeedsModel('lexical', null)).toBe(false)
    // Absent field = worker default bundle path + first-run download.
    expect(semanticModeNeedsModel('hybrid', undefined)).toBe(false)
    // Explicit null disables the semantic lane: that is the stranded case.
    expect(semanticModeNeedsModel('hybrid', null)).toBe(true)
    expect(semanticModeNeedsModel('shadow', null)).toBe(true)
    expect(semanticModeNeedsModel('hybrid', '/models/minilm')).toBe(false)
  })
})
