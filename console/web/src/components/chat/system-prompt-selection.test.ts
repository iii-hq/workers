import { describe, expect, it } from 'vitest'
import {
  AGENT_CHOICE_PREFIX,
  agentIdForSend,
  agentIdFromSystemPrompt,
  choiceToValue,
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SystemPromptState,
  selectionForSend,
  skillSelectionForSend,
  toggleSkillSelection,
  toSelection,
  valueToChoice,
  withAgentChoice,
  withoutAgentChoice,
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

  it('legacy skill addons on the default choice send alone, always as enrich', () => {
    expect(
      toSelection({
        choice: 'default',
        strategy: 'override',
        namedBody: '',
        customText: '',
        addons: [{ kind: 'skill', name: 'coder/index', body: 'Coder skill.' }],
      }),
    ).toEqual({
      body: 'Coder skill.',
      strategy: 'enrich',
    })
  })

  it('legacy skill addons append after the named body, strategy preserved', () => {
    expect(
      toSelection({
        choice: { named: 'pirate' },
        strategy: 'override',
        namedBody: 'Arr.',
        customText: '',
        addons: [{ kind: 'skill', name: 'review', body: 'Review checklist.' }],
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
        addons: [{ kind: 'skill', name: 'review', body: 'Review checklist.' }],
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
        addons: [{ kind: 'skill', name: 'empty', body: '   ' }],
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

describe('agentIdForSend', () => {
  const agentState: SystemPromptState = {
    ...DEFAULT_SYSTEM_PROMPT_STATE,
    choice: { named: `${AGENT_CHOICE_PREFIX}tech-leader` },
  }

  it('returns the id only on a first, non-queued send', () => {
    expect(
      agentIdForSend(agentState, { turnEstablished: false, willQueue: false }),
    ).toBe('tech-leader')
    // Once a turn exists the harness refuses options.agent.
    expect(
      agentIdForSend(agentState, { turnEstablished: true, willQueue: false }),
    ).toBeUndefined()
    // A queued mid-stream send targets a session with a prior turn — the
    // willQueue gate is load-bearing, not symmetry with selectionForSend.
    expect(
      agentIdForSend(agentState, { turnEstablished: false, willQueue: true }),
    ).toBeUndefined()
  })

  it('ignores non-agent choices', () => {
    expect(
      agentIdForSend(DEFAULT_SYSTEM_PROMPT_STATE, {
        turnEstablished: false,
        willQueue: false,
      }),
    ).toBeUndefined()
    expect(
      agentIdForSend(
        { ...DEFAULT_SYSTEM_PROMPT_STATE, choice: { named: 'pirate' } },
        { turnEstablished: false, willQueue: false },
      ),
    ).toBeUndefined()
  })

  it('an agent choice with an empty namedBody yields no prompt selection', () => {
    expect(toSelection(agentState)).toBeNull()
  })
})

describe('agent choice state', () => {
  it('selects an agent and returns to a clean manual default', () => {
    const agent = withAgentChoice(DEFAULT_SYSTEM_PROMPT_STATE, 'engineer')

    expect(agentIdFromSystemPrompt(agent)).toBe('engineer')
    expect(agent.choice).toEqual({
      named: `${AGENT_CHOICE_PREFIX}engineer`,
    })
    expect(agent.strategy).toBe('enrich')

    const manual = withoutAgentChoice(agent)
    expect(agentIdFromSystemPrompt(manual)).toBeNull()
    expect(manual.choice).toBe('default')
  })

  it('leaves an existing manual prompt unchanged', () => {
    const manual: SystemPromptState = {
      ...DEFAULT_SYSTEM_PROMPT_STATE,
      choice: { named: 'reviewer' },
      namedBody: 'Review carefully.',
    }

    expect(withoutAgentChoice(manual)).toBe(manual)
  })
})
