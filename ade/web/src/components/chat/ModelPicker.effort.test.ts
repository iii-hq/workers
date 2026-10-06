import { describe, expect, it } from 'vitest'
import type { ModelOption } from '@/types/chat'
import { lowestSupportedEffort } from './ModelPicker'

const codexLuna: ModelOption = {
  id: 'openai-codex::codex/gpt-6-luna',
  label: 'GPT-6 Luna',
  supportsThinking: true,
  reasoningEfforts: ['low', 'medium', 'high', 'xhigh', 'max'].map((effort) => ({
    effort,
  })),
}

describe('lowestSupportedEffort', () => {
  it('keeps a level the model offers', () => {
    expect(lowestSupportedEffort(codexLuna, 'medium')).toBe('medium')
  })

  it("falls back to the model's lowest effort when it lacks the level", () => {
    // The onboarding tour asks for `minimal`; Codex rejects it natively.
    expect(lowestSupportedEffort(codexLuna, 'minimal')).toBe('low')
  })

  it('uses Default for a model with no effort choices', () => {
    const fixed: ModelOption = { id: 'x::fixed', label: 'Fixed' }
    expect(lowestSupportedEffort(fixed, 'minimal')).toBe('default')
  })

  it('passes the level through while the catalog has no row', () => {
    expect(lowestSupportedEffort(undefined, 'minimal')).toBe('minimal')
  })
})
