import { describe, expect, it } from 'vitest'
import type { AgentEntry } from '@/lib/backend/directory-prompts'
import {
  agentProfileFromEntry,
  agentToPreselect,
  COMPOSER_PLACEHOLDER_MAX_CHARS,
  DEFAULT_AGENT_ID,
  GENERIC_COMPOSER_PLACEHOLDER,
  idleComposerPlaceholder,
  normalizeComposerPlaceholder,
} from './agent-defaults'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  withAgentChoice,
} from './system-prompt-selection'

function entry(id: string, extra: Partial<AgentEntry> = {}): AgentEntry {
  return {
    id,
    name: id,
    description: '',
    logo: null,
    icon: null,
    model: null,
    skill_count: null,
    modified_at: '',
    ...extra,
  }
}

describe('agentToPreselect', () => {
  const catalog = [
    entry('builder'),
    entry(DEFAULT_AGENT_ID, { name: 'Default' }),
  ]

  it('selects the visible default profile for an untouched new session', () => {
    expect(
      agentToPreselect(catalog, null, DEFAULT_SYSTEM_PROMPT_STATE)?.id,
    ).toBe(DEFAULT_AGENT_ID)
  })

  it('never overrides a choice the user already made', () => {
    expect(
      agentToPreselect(catalog, 'builder', DEFAULT_SYSTEM_PROMPT_STATE),
    ).toBeNull()
    const named = {
      ...DEFAULT_SYSTEM_PROMPT_STATE,
      choice: { named: 'my-prompt' },
    }
    expect(agentToPreselect(catalog, null, named)).toBeNull()
    const custom = { ...DEFAULT_SYSTEM_PROMPT_STATE, choice: 'custom' as const }
    expect(agentToPreselect(catalog, null, custom)).toBeNull()
    // A legacy agent choice encoded in the prompt state is a selection too.
    expect(
      agentToPreselect(
        catalog,
        'builder',
        withAgentChoice(DEFAULT_SYSTEM_PROMPT_STATE, 'builder'),
      ),
    ).toBeNull()
  })

  it('keeps the old behavior when Directory serves no visible default', () => {
    expect(
      agentToPreselect([entry('builder')], null, DEFAULT_SYSTEM_PROMPT_STATE),
    ).toBeNull()
    expect(
      agentToPreselect(
        [entry(DEFAULT_AGENT_ID, { hidden: true })],
        null,
        DEFAULT_SYSTEM_PROMPT_STATE,
      ),
    ).toBeNull()
  })
})

describe('composer placeholder', () => {
  it('normalizes to plain collapsed text and treats blank as absent', () => {
    expect(normalizeComposerPlaceholder('  Example:\n  a   list ')).toBe(
      'Example: a list',
    )
    for (const blank of ['', '   ', null, undefined, 42]) {
      expect(normalizeComposerPlaceholder(blank)).toBeUndefined()
    }
    const long = 'x'.repeat(COMPOSER_PLACEHOLDER_MAX_CHARS + 20)
    expect(normalizeComposerPlaceholder(long)).toHaveLength(
      COMPOSER_PLACEHOLDER_MAX_CHARS,
    )
  })

  it('freezes the example onto the selected profile snapshot', () => {
    const snapshot = agentProfileFromEntry(
      entry('builder', {
        name: ' Builder ',
        composer_placeholder: 'Example: a task list',
      }),
    )
    expect(snapshot).toEqual({
      id: 'builder',
      name: 'Builder',
      composerPlaceholder: 'Example: a task list',
    })
    // Older Directory responses omit the field entirely.
    expect(agentProfileFromEntry(entry('plain'))).toEqual({
      id: 'plain',
      name: 'plain',
    })
  })

  it('shows the example only before the first message, else the generic hint', () => {
    const profile = {
      id: 'builder',
      name: 'Builder',
      composerPlaceholder: 'Example: a task list',
    }
    expect(idleComposerPlaceholder(profile, false)).toBe('Example: a task list')
    expect(idleComposerPlaceholder(profile, true)).toBe(
      GENERIC_COMPOSER_PLACEHOLDER,
    )
    expect(idleComposerPlaceholder({ id: 'plain', name: 'plain' }, false)).toBe(
      GENERIC_COMPOSER_PLACEHOLDER,
    )
    expect(idleComposerPlaceholder(undefined, false)).toBe(
      GENERIC_COMPOSER_PLACEHOLDER,
    )
  })
})
