import { describe, expect, it } from 'vitest'
import { effortStopLabel } from './ReasoningEffortSlider'
import { forkStripText, lockPresentation } from './side-chats'

const options = [
  { id: 'claude-code::claude-code/claude-sonnet-5', label: 'claude sonnet 5' },
  {
    id: 'claude-code::claude-code/claude-haiku-5-5',
    label: 'claude haiku 5.5',
  },
]

describe('the locked model label (chat-projects §2.3.2, D97)', () => {
  it('names the effort the lock carries', () => {
    const lock = lockPresentation(
      {
        model: 'claude-code/claude-haiku-5-5',
        provider: 'claude-code',
        thinkingLevel: 'high',
      },
      { model: null },
      options,
    )
    expect(lock.label).toBe('Claude Haiku 5.5 · High')
    expect(lock.short).toBe('Haiku 5.5 · High')
    expect(lock.model).toBe('claude-code::claude-code/claude-haiku-5-5')
  })

  it('names the model default effort when the lock carries no thinking_level', () => {
    // The conversation never set an effort: it ran at the model's default,
    // which the composer's model menu and effort slider call "Default".
    const lock = lockPresentation(
      { model: 'claude-code/claude-sonnet-5', provider: 'claude-code' },
      // The conversation's later choice does not reach the side chat.
      {
        model: 'claude-code::claude-code/claude-haiku-5-5',
        thinkingLevel: 'high',
      },
      options,
    )
    expect(lock.label).toBe('Claude Sonnet 5 · Default')
    expect(lock.short).toBe('Sonnet 5 · Default')
    expect(lock.meta).toBe('Claude Sonnet 5 · Default')
    expect(effortStopLabel('default')).toBe('Default')
  })

  it('names it too for a side chat made before the engine locked it', () => {
    const unset = lockPresentation(
      null,
      { model: 'claude-code::claude-code/claude-sonnet-5' },
      options,
    )
    expect(unset.label).toBe('Claude Sonnet 5 · Default')
    const chosen = lockPresentation(
      null,
      {
        model: 'claude-code::claude-code/claude-sonnet-5',
        thinkingLevel: 'low',
      },
      options,
    )
    expect(chosen.label).toBe('Claude Sonnet 5 · Low')
  })

  it('adds the account to the meta line (phase 4)', () => {
    const lock = lockPresentation(
      {
        model: 'claude-code/claude-sonnet-5',
        provider: 'claude-code',
        thinkingLevel: 'medium',
        account: 'Work',
      },
      { model: null },
      options,
    )
    expect(lock.meta).toBe('Claude Sonnet 5 · Medium · Work')
  })
})

describe('the fork strip', () => {
  it('says where it was forked, wide and on a phone', () => {
    const at = new Date(2026, 10, 24, 14, 2).getTime()
    expect(
      forkStripText({
        at,
        fromUser: false,
        parentTitle: 'Webhook retries',
        compact: false,
      }),
    ).toMatch(
      /^Knows the conversation up to the agent's reply at .+ and runs on its model and effort, so it reuses its cache\. Nothing here goes into Webhook retries\.$/,
    )
    expect(
      forkStripText({
        at,
        fromUser: true,
        parentTitle: 'Webhook retries',
        compact: true,
      }),
    ).toMatch(
      /^Forked from your message at .+ · same model and effort, cached$/,
    )
  })
})
