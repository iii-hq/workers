import { describe, expect, it } from 'vitest'
import type { ProviderChoice } from '@/lib/onboarding/plan'
import { draftsAfterConnect, preselected, resolveDraft } from './ModelsStep'

describe('drafts after Connect', () => {
  it('keeps every checkbox as the person left it and drops typed keys', () => {
    const before = new Map([
      ['claude-code', { selected: false }],
      [
        'anthropic',
        { selected: true, key: { mode: 'paste', value: 'sk-ant-x' } as never },
      ],
    ])
    const after = draftsAfterConnect(before)
    expect(after.get('claude-code')).toEqual({ selected: false })
    expect(after.get('anthropic')).toEqual({ selected: true })
  })

  it('a kept checkbox falls back to the default key input', () => {
    const fallback = { selected: true, key: { mode: 'detected' } as never }
    expect(resolveDraft({ selected: false }, fallback)).toEqual({
      selected: false,
      key: { mode: 'detected' },
    })
    expect(resolveDraft(undefined, fallback)).toBe(fallback)
  })
})

describe('preselected', () => {
  const codex = {
    kind: 'subscription',
    providerId: 'openai-codex',
    worker: 'provider-openai-codex',
    title: 'Codex',
    ready: false,
    installed: false,
    recommended: true,
    reason: 'signed in',
    modelCount: 0,
    provider: {} as never,
    tool: null,
    usable: true,
  } satisfies ProviderChoice

  it('checks a usable recommendation before anything is connected', () => {
    expect(preselected(codex, true)).toBe(true)
  })

  it('leaves it unchecked once a provider is connected, so Back and reopening never bring it back', () => {
    expect(preselected(codex, false)).toBe(false)
  })

  it('never checks a provider that already serves models, or one that is not signed in', () => {
    expect(preselected({ ...codex, ready: true }, true)).toBe(false)
    expect(preselected({ ...codex, usable: false }, true)).toBe(false)
  })
})
