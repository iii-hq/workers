import { describe, expect, it } from 'vitest'
import {
  ACCOUNT_DEFAULT,
  applyPatchToMetadata,
  buildDetailsPatch,
  claudeAccountOf,
  cleanDescription,
  descriptionHistory,
  detailsDraftOf,
} from './conversation-details'

const base = {
  title: 'Webhook retries',
  description: 'Backoff with jitter',
  descriptionBy: 'agent' as const,
  descriptionAtMs: 100,
  sessionMetadata: {
    description_history: [{ text: 'Reading the code', by: 'agent', at_ms: 50 }],
  } as Record<string, unknown>,
}

describe('Conversation details: the one patch-meta of a Save (§1.3.5)', () => {
  it('writes nothing when nothing changed', () => {
    expect(buildDetailsPatch(base, detailsDraftOf(base), 999)).toBeNull()
  })

  it('a new name sets title_manual', () => {
    const patch = buildDetailsPatch(
      base,
      { ...detailsDraftOf(base), title: '  Retries  ' },
      999,
    )
    expect(patch).toEqual({
      title: 'Retries',
      set: { title_manual: true },
      unset: [],
    })
  })

  it("a typed description is the user's: manual, and the old one pushed into the history", () => {
    const patch = buildDetailsPatch(
      base,
      {
        ...detailsDraftOf(base),
        description: 'Mine   now',
        agentKeepsIt: false,
      },
      999,
    )
    expect(patch?.description).toBe('Mine now')
    expect(patch?.set).toMatchObject({
      description_by: 'user',
      description_at_ms: 999,
      description_manual: true,
    })
    expect(patch?.set.description_history).toEqual([
      { text: 'Backoff with jitter', by: 'agent', at_ms: 100 },
      { text: 'Reading the code', by: 'agent', at_ms: 50 },
    ])
  })

  it('keeps five descriptions in the history', () => {
    const full = {
      ...base,
      sessionMetadata: {
        description_history: [1, 2, 3, 4, 5].map((n) => ({
          text: `d${n}`,
          by: 'agent',
          at_ms: n,
        })),
      },
    }
    const patch = buildDetailsPatch(
      full,
      { ...detailsDraftOf(full), description: 'new' },
      9,
    )
    expect(patch?.set.description_history).toHaveLength(5)
  })

  it('the switch alone writes description_manual', () => {
    expect(
      buildDetailsPatch(
        base,
        { ...detailsDraftOf(base), agentKeepsIt: false },
        1,
      )?.set,
    ).toEqual({ description_manual: true })
    const manual = { ...base, sessionMetadata: { description_manual: true } }
    expect(detailsDraftOf(manual).agentKeepsIt).toBe(false)
    expect(
      buildDetailsPatch(
        manual,
        { ...detailsDraftOf(manual), agentKeepsIt: true },
        1,
      )?.set,
    ).toEqual({ description_manual: false })
  })

  it('cleans a description to one line of 140 characters', () => {
    expect(cleanDescription(' a\n b ')).toBe('a b')
    expect(cleanDescription('x'.repeat(200))).toHaveLength(140)
  })

  it('reads the history defensively', () => {
    expect(
      descriptionHistory({
        description_history: [{ text: '' }, 'x', { text: 'ok', by: 'user' }],
      }),
    ).toEqual([{ text: 'ok', by: 'user', at_ms: 0 }])
  })

  it('applies a patch to a metadata map (the set-meta fallback)', () => {
    expect(
      applyPatchToMetadata({ a: 1, b: 2 }, { set: { c: 3 }, unset: ['b'] }),
    ).toEqual({ a: 1, c: 3 })
  })
})

describe('Conversation details: the Claude Code account (§4.3.5)', () => {
  it('leaves claude_account alone until the row is changed', () => {
    const pinned = {
      ...base,
      sessionMetadata: { ...base.sessionMetadata, claude_account: 'personal' },
    }
    expect(claudeAccountOf(pinned.sessionMetadata)).toBe('personal')
    expect(claudeAccountOf(base.sessionMetadata)).toBe(ACCOUNT_DEFAULT)
    expect(detailsDraftOf(pinned).claudeAccount).toBeUndefined()
    expect(buildDetailsPatch(pinned, detailsDraftOf(pinned), 999)).toBeNull()
    expect(
      buildDetailsPatch(
        pinned,
        { ...detailsDraftOf(pinned), claudeAccount: 'personal' },
        999,
      ),
    ).toBeNull()
  })

  it('pins an account, follows the global, or goes back to the project', () => {
    const pinned = {
      ...base,
      sessionMetadata: { ...base.sessionMetadata, claude_account: 'personal' },
    }
    expect(
      buildDetailsPatch(
        base,
        { ...detailsDraftOf(base), claudeAccount: 'work' },
        999,
      ),
    ).toEqual({ set: { claude_account: 'work' }, unset: [] })
    expect(
      buildDetailsPatch(
        pinned,
        { ...detailsDraftOf(pinned), claudeAccount: 'global' },
        999,
      ),
    ).toEqual({ set: { claude_account: 'global' }, unset: [] })
    expect(
      buildDetailsPatch(
        pinned,
        { ...detailsDraftOf(pinned), claudeAccount: ACCOUNT_DEFAULT },
        999,
      ),
    ).toEqual({ set: {}, unset: ['claude_account'] })
  })
})
