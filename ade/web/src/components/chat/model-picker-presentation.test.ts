import { describe, expect, it } from 'vitest'
import { formatModelLabel, modelFamily } from './model-picker-presentation'

describe('formatModelLabel', () => {
  it.each([
    ['Claude Fable 5', 'Fable 5'],
    ['Claude Opus 5.5 (1M context)', 'Opus 5.5'],
    ['claude-haiku-4-5', 'Haiku 4 5'],
    ['gpt-6-astra', 'GPT-6 Astra'],
    ['Claude', 'Claude'],
  ])('%s → %s', (label, expected) => {
    expect(formatModelLabel(label)).toBe(expected)
  })
})

describe('modelFamily', () => {
  it('reads the family off the model, not the provider', () => {
    expect(modelFamily('anthropic::claude-fable-5', 'Claude Fable 5')).toBe(
      'claude',
    )
    expect(modelFamily('claude-code::opus', 'Opus')).toBe('claude')
    expect(modelFamily('openai-codex::gpt-6-astra', 'GPT-6 Astra')).toBe('gpt')
    expect(modelFamily('github-copilot::gpt-4.1', 'GPT-4.1')).toBe('gpt')
    expect(modelFamily('claude-code::gpt-5', 'GPT-5')).toBe('gpt')
    expect(modelFamily('google::gemini-3-pro', 'Gemini 3 Pro')).toBeNull()
  })
})
