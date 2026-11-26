import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import {
  AT_MODEL_LIMIT,
  autocompactingNotice,
  autocompactOf,
  COMPACTS_AFTER_TURN,
  chipText,
  compactionMarkerText,
  conversationAutocompact,
  effectiveCell,
  effectiveTokens,
  formatTokens,
  type GlobalAutocompact,
  globalNow,
  historyChange,
  INHERIT,
  inheritChoiceLabel,
  limitCell,
  MODEL_LIMIT_ONLY,
  markedCompaction,
  meterLabel,
  meterLine,
  modelNumbers,
  modelOutcome,
  parseAutocompactSetting,
  parseCompactionFacts,
  parseGlobalAutocompact,
  parseSnapshotAutocompact,
  parseTokens,
  resolveAutocompact,
  ruleName,
  SETTINGS_HINT,
  sameSetting,
  sessionsCount,
  sessionWhere,
  settingsRowText,
  sideChatLine,
  TOKENS_RANGE_ERROR,
  tickRatio,
} from './autocompact'

const GLOBAL_400K: GlobalAutocompact = { mode: 'tokens', tokens: 400_000 }
const GLOBAL_OFF: GlobalAutocompact = { mode: 'off', tokens: 400_000 }
const ACME = {
  id: 'p-acme',
  name: 'acme-web',
  autocompact: { mode: 'tokens', tokens: 160_000 },
}
const BILLING = { id: 'p-billing', name: 'billing-api' }
const MOBILE = { id: 'p-mobile', name: 'mobile', autocompact: { mode: 'off' } }

function conv(
  id: string,
  over: Partial<Pick<Conversation, 'sessionMetadata' | 'projectId'>> = {},
): Pick<Conversation, 'id' | 'sessionMetadata' | 'projectId'> {
  return { id, ...over }
}

describe('parseTokens', () => {
  it.each([
    ['400000', 400_000],
    ['400,000', 400_000],
    ['400k', 400_000],
    ['400K', 400_000],
    [' 160 k ', 160_000],
    ['1.2M', 1_200_000],
    ['1.2m', 1_200_000],
    ['2M', 2_000_000],
    ['20k', 20_000],
    ['.5m', 500_000],
  ])('reads %s as %d', (text, tokens) => {
    expect(parseTokens(text)).toEqual({ ok: true, tokens })
  })

  it.each(['19999', '19k', '2.1M', '3m', '', 'lots', '400kk', '-100k'])(
    'refuses %j with the range error',
    (text) => {
      expect(parseTokens(text)).toEqual({
        ok: false,
        error: TOKENS_RANGE_ERROR,
      })
    },
  )

  it('says the range in the harness words', () => {
    expect(TOKENS_RANGE_ERROR).toBe('Between 20k and 2M tokens')
  })
})

describe('formatTokens', () => {
  it.each([
    [9_500, '9.5k'],
    [1_000, '1k'],
    [34_000, '34k'],
    [160_000, '160k'],
    [161_400, '161k'],
    [999_499, '999k'],
    [999_500, '1M'],
    [1_000_000, '1M'],
    [1_200_000, '1.2M'],
    [0, '0'],
  ])('writes %d as %s', (n, text) => {
    expect(formatTokens(n)).toBe(text)
  })
})

describe('the settings', () => {
  it('reads a malformed project or conversation value as inherit', () => {
    expect(parseAutocompactSetting(undefined)).toEqual(INHERIT)
    expect(parseAutocompactSetting({ mode: 'tokens' })).toEqual(INHERIT)
    expect(parseAutocompactSetting({ mode: 'tokens', tokens: 5 })).toEqual(
      INHERIT,
    )
    expect(parseAutocompactSetting({ mode: 'off' })).toEqual({ mode: 'off' })
    expect(
      parseAutocompactSetting({ mode: 'tokens', tokens: 160_000 }),
    ).toEqual({ mode: 'tokens', tokens: 160_000 })
  })

  it('reads a malformed global as off, keeping the last size', () => {
    expect(parseGlobalAutocompact(null)).toEqual({ mode: 'off' })
    expect(parseGlobalAutocompact({ mode: 'tokens' })).toEqual({ mode: 'off' })
    expect(
      parseGlobalAutocompact({
        mode: 'off',
        tokens: 400_000,
        instructions: 'Keep the PRs.',
      }),
    ).toEqual({ mode: 'off', tokens: 400_000, instructions: 'Keep the PRs.' })
  })

  it('compares settings', () => {
    expect(sameSetting(INHERIT, { mode: 'inherit' })).toBe(true)
    expect(
      sameSetting({ mode: 'tokens', tokens: 1 }, { mode: 'tokens', tokens: 2 }),
    ).toBe(false)
    expect(sameSetting({ mode: 'off' }, INHERIT)).toBe(false)
  })
})

describe('resolveAutocompact: conversation, then project, then global', () => {
  it.each([
    [
      'the conversation wins',
      { mode: 'tokens', tokens: 600_000 } as const,
      ACME,
      GLOBAL_400K,
      600_000,
      'session',
    ],
    [
      'the conversation turns it off',
      { mode: 'off' } as const,
      ACME,
      GLOBAL_400K,
      null,
      'session',
    ],
    [
      'the project wins over the global',
      INHERIT,
      ACME,
      GLOBAL_400K,
      160_000,
      'project',
    ],
    ['the project turns it off', INHERIT, MOBILE, GLOBAL_400K, null, 'project'],
    [
      'a project that inherits falls to the global',
      INHERIT,
      BILLING,
      GLOBAL_400K,
      400_000,
      'global',
    ],
    ['no project: the global', INHERIT, null, GLOBAL_400K, 400_000, 'global'],
    ['the global off', INHERIT, null, GLOBAL_OFF, null, 'global'],
  ])('%s', (_name, session, project, global, tokens, scope) => {
    const resolved = resolveAutocompact({ session, project, global })
    expect(resolved.tokens).toBe(tokens)
    expect(resolved.rule.scope).toBe(scope)
  })

  it('names the project on a project rule', () => {
    expect(
      resolveAutocompact({ session: INHERIT, project: ACME, global: null })
        .rule,
    ).toEqual({
      scope: 'project',
      project_id: 'p-acme',
      project_name: 'acme-web',
    })
  })

  it('reads a conversation from its metadata and its project', () => {
    const c = conv('c1', { projectId: 'p-acme' })
    expect(
      conversationAutocompact({
        conversation: c,
        projects: [ACME, BILLING],
        global: GLOBAL_400K,
      }).tokens,
    ).toBe(160_000)
    expect(autocompactOf(c)).toEqual(INHERIT)
  })

  it('a side chat follows its loaded conversation, not its frozen copy (D164)', () => {
    const parent = conv('parent', {
      projectId: 'p-billing',
      sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
    })
    const side = conv('side', {
      projectId: 'p-acme',
      sessionMetadata: {
        side_chat_of: 'parent',
        autocompact: { mode: 'off' },
      },
    })
    const resolved = conversationAutocompact({
      conversation: side,
      projects: [ACME, BILLING],
      global: GLOBAL_400K,
      conversations: [parent, side],
    })
    expect(resolved).toEqual({ tokens: 600_000, rule: { scope: 'session' } })
  })

  it('a side chat whose conversation is not loaded reads its own copy', () => {
    const side = conv('side', {
      projectId: 'p-acme',
      sessionMetadata: { side_chat_of: 'gone' },
    })
    expect(
      conversationAutocompact({
        conversation: side,
        projects: [ACME],
        global: GLOBAL_400K,
        conversations: [],
      }).tokens,
    ).toBe(160_000)
  })
})

describe('the effective size and the tick', () => {
  it('is the size below usable, else the model limit (null)', () => {
    expect(effectiveTokens(160_000, 916_000)).toBe(160_000)
    // 160k on a 200k Claude model is past its ~116k budget: the model's limit.
    expect(effectiveTokens(160_000, 116_000)).toBeNull()
    expect(effectiveTokens(116_000, 116_000)).toBeNull()
    expect(effectiveTokens(null, 916_000)).toBeNull()
    expect(effectiveTokens(160_000, undefined)).toBeNull()
    expect(effectiveTokens(160_000, 0)).toBeNull()
  })

  it('places the tick at effective / usable', () => {
    expect(tickRatio(400_000, 916_000)).toBeCloseTo(0.4367, 3)
    expect(tickRatio(null, 916_000)).toBeNull()
    expect(tickRatio(400_000, null)).toBeNull()
  })

  it('reads the snapshot tolerantly', () => {
    expect(parseSnapshotAutocompact(undefined)).toBeNull()
    expect(
      parseSnapshotAutocompact({
        setting_tokens: 160_000,
        effective_tokens: null,
        cap_tokens: 116_000,
        rule: { scope: 'project', project_name: 'acme-web', other: 1 },
      }),
    ).toEqual({
      setting_tokens: 160_000,
      effective_tokens: null,
      cap_tokens: 116_000,
      rule: { scope: 'project', project_name: 'acme-web' },
    })
  })
})

describe('the meter and header words (§3.3.3)', () => {
  it('names the rules', () => {
    expect(ruleName({ scope: 'global' })).toBe('global rule')
    expect(ruleName({ scope: 'project', project_name: 'acme-web' })).toBe(
      'acme-web rule',
    )
    expect(ruleName({ scope: 'session' })).toBe("this conversation's rule")
  })

  it('writes the label, the line, the chip and the notice', () => {
    expect(meterLabel(400_000)).toBe('auto 400k')
    expect(
      meterLine(160_000, { scope: 'project', project_name: 'acme-web' }),
    ).toBe('Auto-compact at 160k · acme-web rule')
    expect(meterLine(null, null)).toBe(AT_MODEL_LIMIT)
    expect(AT_MODEL_LIMIT).toBe("At the model's limit")
    expect(chipText(160_000)).toBe('Auto-compact 160k')
    expect(autocompactingNotice(160_000)).toBe('Auto-compacting at 160k…')
    expect(COMPACTS_AFTER_TURN).toBe('Compacts after this turn')
  })
})

describe('the marker (contract C, §3.3.5)', () => {
  const rule = {
    scope: 'project',
    project_id: 'p-acme',
    project_name: 'acme-web',
  }

  it.each([
    [
      'auto',
      {
        trigger: 'auto',
        threshold_tokens: 160_000,
        rule,
        context_before: 161_000,
        context_after: 34_000,
      },
      'Auto-compacted at 160k · 161k → 34k tokens · acme-web rule',
    ],
    [
      'inline',
      {
        trigger: 'inline',
        threshold_tokens: 160_000,
        rule,
        context_before: 183_000,
        context_after: 31_000,
      },
      'Auto-compacted during the turn at 160k · 183k → 31k tokens · acme-web rule',
    ],
    [
      'limit',
      {
        trigger: 'limit',
        context_before: 116_000,
        context_after: 30_000,
        rule: { scope: 'global' },
      },
      "Compacted at the model's limit · 116k → 30k tokens",
    ],
    [
      'auto on the global rule',
      {
        trigger: 'auto',
        threshold_tokens: 400_000,
        rule: { scope: 'global' },
        context_before: 401_000,
        context_after: 52_000,
      },
      'Auto-compacted at 400k · 401k → 52k tokens · global rule',
    ],
    [
      "auto on this conversation's rule",
      {
        trigger: 'auto',
        threshold_tokens: 600_000,
        rule: { scope: 'session' },
        context_before: 602_000,
        context_after: 40_000,
      },
      "Auto-compacted at 600k · 602k → 40k tokens · this conversation's rule",
    ],
    [
      'auto without the numbers',
      { trigger: 'auto', threshold_tokens: 160_000 },
      'Auto-compacted at 160k',
    ],
  ])('%s', (_name, record, text) => {
    const facts = markedCompaction(parseCompactionFacts(record))
    expect(facts).not.toBeNull()
    expect(compactionMarkerText(facts as NonNullable<typeof facts>)).toBe(text)
  })

  it('leaves /compact and older records as they read today', () => {
    expect(
      markedCompaction(parseCompactionFacts({ trigger: 'command' })),
    ).toBeNull()
    expect(
      parseCompactionFacts({ summary: 'old', tokens_before: 1200 }),
    ).toBeNull()
    expect(parseCompactionFacts({ trigger: 'surprise' })).toBeNull()
    expect(parseCompactionFacts(null)).toBeNull()
  })
})

describe('the settings dialog words (§3.3.1)', () => {
  it('says what the row does', () => {
    expect(settingsRowText(GLOBAL_400K)).toBe(
      "At 400k, or at the model's limit when it comes first.",
    )
    expect(settingsRowText(GLOBAL_OFF)).toBe(
      "Off: conversations compact at the model's limit, as today.",
    )
  })

  it('writes the revised hint', () => {
    expect(SETTINGS_HINT).toBe(
      "Whichever comes first: this size or the model's own limit. A model's limit is its input budget, less than its window: about 116k on a 200k Claude model. The grey ticks are the limits of the models used here in the last 7 days.",
    )
  })

  it('writes a model row', () => {
    expect(modelNumbers(200_000, 116_000)).toBe(
      '200k window · 116k input budget',
    )
    expect(modelNumbers(null, 116_000)).toBe('116k input budget')
    expect(modelOutcome(400_000, 916_000)).toBe('At 400k')
    expect(modelOutcome(400_000, 116_000)).toBe('At its own limit, as today')
    expect(modelOutcome(null, 916_000)).toBe('At its own limit, as today')
  })

  it('writes the Limit cell', () => {
    expect(limitCell({ mode: 'tokens', tokens: 160_000 }, GLOBAL_400K)).toBe(
      '160k',
    )
    expect(limitCell({ mode: 'off' }, GLOBAL_400K)).toBe('Off')
    expect(limitCell(INHERIT, GLOBAL_400K)).toBe('Global (400k)')
    expect(limitCell(INHERIT, GLOBAL_OFF)).toBe('Global (off)')
  })

  it('writes the Effective cell per model (D173)', () => {
    const models = [
      { name: 'Sonnet 5.5', usable: 916_000 },
      { name: 'Opus 5.5', usable: 116_000 },
    ]
    expect(effectiveCell(160_000, models)).toBe(
      '160k on Sonnet 5.5 · Opus 5.5 at its own limit',
    )
    expect(effectiveCell(600_000, [models[0]])).toBe('600k on Sonnet 5.5')
    expect(effectiveCell(null, models)).toBe(MODEL_LIMIT_ONLY)
    expect(MODEL_LIMIT_ONLY).toBe("The model's limit only")
    expect(effectiveCell(400_000, [])).toBe("400k, or the model's limit")
    expect(
      effectiveCell(950_000, [...models, { name: 'Haiku 5', usable: 180_000 }]),
    ).toBe('Sonnet 5.5, Opus 5.5 and Haiku 5 at their own limits')
  })

  it('writes the session rows and their count', () => {
    expect(sessionWhere('Ledger reconciliation', 'billing-api')).toBe(
      'Ledger reconciliation · session in billing-api',
    )
    expect(sessionWhere('Scratch', null)).toBe('Scratch · session, no project')
    expect(sessionsCount(1, false)).toBe('1 session sets their own value')
    expect(sessionsCount(2, false)).toBe('2 sessions set their own value')
    expect(sessionsCount(12, true)).toBe('12+ sessions set their own value')
  })

  it('writes a history row', () => {
    expect(historyChange(161_000, 34_000)).toBe('161k → 34k')
    expect(historyChange(undefined, 34_000)).toBe('')
  })
})

describe('the project dialog and Details words (§3.3.2, §3.3.4)', () => {
  it('says what the global does now', () => {
    expect(globalNow(GLOBAL_400K)).toBe('at 400k')
    expect(globalNow(GLOBAL_OFF)).toBe("at the model's limit")
  })

  it("labels Details' first choice", () => {
    expect(inheritChoiceLabel({ project: ACME, global: GLOBAL_400K })).toBe(
      "Project's (160k)",
    )
    expect(inheritChoiceLabel({ project: BILLING, global: GLOBAL_400K })).toBe(
      "Project's (400k)",
    )
    expect(inheritChoiceLabel({ project: MOBILE, global: GLOBAL_400K })).toBe(
      "Project's (model's limit)",
    )
    expect(inheritChoiceLabel({ project: null, global: GLOBAL_400K })).toBe(
      'Global (400k)',
    )
    expect(inheritChoiceLabel({ project: null, global: GLOBAL_OFF })).toBe(
      "Global (model's limit)",
    )
  })

  it("writes a side chat's line", () => {
    expect(sideChatLine(160_000)).toBe(
      'Compacts at 160k, as its conversation does.',
    )
    expect(sideChatLine(null)).toBe(
      "Compacts at the model's limit, as its conversation does.",
    )
  })
})
