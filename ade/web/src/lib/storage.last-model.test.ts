// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { savePinnedModels } from './pinned-models'
import {
  loadLastModel,
  loadLastThinkingLevel,
  saveLastModel,
  saveLastThinkingLevel,
} from './storage'

describe('the model a new chat starts on', () => {
  afterEach(() => {
    localStorage.clear()
    savePinnedModels(null)
  })

  it('follows the last chat over the quick menu default slot', () => {
    savePinnedModels([{ id: 'anthropic::claude-opus', effort: 'high' }])
    saveLastModel('claude-code::opus')
    saveLastThinkingLevel('medium')

    expect(loadLastModel()).toBe('claude-code::opus')
    expect(loadLastThinkingLevel()).toBe('medium')
  })

  it('uses the default slot before any chat was started', () => {
    savePinnedModels([{ id: 'anthropic::claude-opus', effort: 'high' }])

    expect(loadLastModel()).toBe('anthropic::claude-opus')
    expect(loadLastThinkingLevel()).toBe('high')
  })

  it('is unset with neither', () => {
    expect(loadLastModel()).toBeNull()
    expect(loadLastThinkingLevel()).toBeNull()
  })
})
