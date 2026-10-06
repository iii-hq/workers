import { describe, expect, it } from 'vitest'
import { draftsAfterConnect, resolveDraft } from './ModelsStep'

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
