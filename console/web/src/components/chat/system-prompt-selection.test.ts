import { describe, expect, it } from 'vitest'
import {
  choiceToValue,
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SystemPromptState,
  selectionForSend,
  skillSelectionForSend,
  toggleSkillSelection,
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
        addons: [],
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
        addons: [],
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
        addons: [],
      }),
    ).toEqual({ body: 'Arr.', strategy: 'enrich' })
  })

  it('addons on the default choice send alone, always as enrich', () => {
    expect(
      toSelection({
        choice: 'default',
        strategy: 'override',
        namedBody: '',
        customText: '',
        addons: [
          { kind: 'prompt', name: 'review', body: 'Review checklist.' },
          { kind: 'skill', name: 'coder/index', body: 'Coder skill.' },
        ],
      }),
    ).toEqual({
      body: 'Review checklist.\n\nCoder skill.',
      strategy: 'enrich',
    })
  })

  it('addons append after the named body, strategy preserved', () => {
    expect(
      toSelection({
        choice: { named: 'pirate' },
        strategy: 'override',
        namedBody: 'Arr.',
        customText: '',
        addons: [{ kind: 'prompt', name: 'review', body: 'Review checklist.' }],
      }),
    ).toEqual({ body: 'Arr.\n\nReview checklist.', strategy: 'override' })
  })

  it('a blank base never ships as override, even with a named choice', () => {
    expect(
      toSelection({
        choice: { named: 'pirate' },
        strategy: 'override',
        namedBody: '   ',
        customText: '',
        addons: [{ kind: 'prompt', name: 'review', body: 'Review checklist.' }],
      }),
    ).toEqual({ body: 'Review checklist.', strategy: 'enrich' })
  })

  it('blank addon bodies are filtered out', () => {
    expect(
      toSelection({
        choice: 'default',
        strategy: 'enrich',
        namedBody: '',
        customText: '',
        addons: [{ kind: 'prompt', name: 'empty', body: '   ' }],
      }),
    ).toBeNull()
  })
})

describe('selectionForSend', () => {
  const named: SystemPromptState = {
    choice: { named: 'pirate' },
    strategy: 'enrich',
    namedBody: 'Arr.',
    customText: '',
    addons: [],
  }

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

describe('skill selection', () => {
  it('treats undefined as all, creates subsets, and resets after clearing the last id', () => {
    expect(toggleSkillSelection(undefined, 'review')).toEqual(['review'])
    expect(toggleSkillSelection(['review'], 'release')).toEqual([
      'review',
      'release',
    ])
    expect(toggleSkillSelection(['review'], 'review')).toBeUndefined()
  })

  it('sends a non-empty subset only before the session is established', () => {
    const firstTurn = { turnEstablished: false, willQueue: false }
    expect(skillSelectionForSend(undefined, firstTurn)).toBeUndefined()
    expect(skillSelectionForSend([], firstTurn)).toBeUndefined()
    expect(skillSelectionForSend(['review'], firstTurn)).toEqual(['review'])
    expect(
      skillSelectionForSend(['review'], {
        turnEstablished: true,
        willQueue: false,
      }),
    ).toBeUndefined()
  })

  it('resends the subset after a user-only failed turn, then omits it for established or queued turns', () => {
    expect(
      skillSelectionForSend(['review'], {
        turnEstablished: false,
        willQueue: false,
      }),
    ).toEqual(['review'])
    expect(
      skillSelectionForSend(['review'], {
        turnEstablished: true,
        willQueue: false,
      }),
    ).toBeUndefined()
    expect(
      skillSelectionForSend(['review'], {
        turnEstablished: false,
        willQueue: true,
      }),
    ).toBeUndefined()
  })
})
