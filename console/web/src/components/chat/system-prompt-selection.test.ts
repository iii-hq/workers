import { describe, expect, it } from 'vitest'
import {
  choiceToValue,
  DEFAULT_SYSTEM_PROMPT_STATE,
  selectionForSend,
  toSelection,
  valueToChoice,
} from './system-prompt-selection'

describe('toSelection', () => {
  it('default choice yields null (send exactly today’s options)', () => {
    expect(toSelection(DEFAULT_SYSTEM_PROMPT_STATE)).toBeNull()
  })

  it('custom choice uses the textarea body', () => {
    expect(
      toSelection({
        choice: 'custom',
        strategy: 'override',
        namedBody: '',
        customText: 'Talk like a pirate.',
      }),
    ).toEqual({ body: 'Talk like a pirate.', strategy: 'override' })
  })

  it('blank custom text yields null', () => {
    expect(
      toSelection({
        choice: 'custom',
        strategy: 'enrich',
        namedBody: '',
        customText: '   ',
      }),
    ).toBeNull()
  })

  it('named choice uses the resolved body fetched at selection time', () => {
    expect(
      toSelection({
        choice: { named: 'pirate' },
        strategy: 'enrich',
        namedBody: 'Arr.',
        customText: 'ignored',
      }),
    ).toEqual({ body: 'Arr.', strategy: 'enrich' })
  })
})

describe('selectionForSend', () => {
  const named = {
    choice: { named: 'pirate' },
    strategy: 'enrich',
    namedBody: 'Arr.',
    customText: '',
  } as const

  it('first send (no turn yet) carries the selection', () => {
    expect(selectionForSend(named, false)).toEqual({
      body: 'Arr.',
      strategy: 'enrich',
    })
  })

  it('later sends omit the prompt fields — the harness inherits', () => {
    expect(selectionForSend(named, true)).toBeNull()
  })
})

describe('choice codec', () => {
  it('round-trips every choice through the select value string', () => {
    for (const choice of ['default', 'custom', { named: 'pirate' }] as const) {
      expect(valueToChoice(choiceToValue(choice))).toEqual(choice)
    }
    expect(choiceToValue({ named: 'pirate' })).toBe('named:pirate')
  })
})
