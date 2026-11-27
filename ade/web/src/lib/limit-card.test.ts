import { afterEach, describe, expect, it } from 'vitest'
import { usageFixture } from '@/stories/fixtures/claude-accounts-fixtures'
import type { Message, SystemMessage } from '@/types/chat'
import type { ClaudeAccount } from './backend/claude-accounts'
import {
  accountFailure,
  dismissLimit,
  lastTurnFailure,
  limitCardModel,
  limitDismissed,
  resetLimitDismissalsForTests,
} from './limit-card'

const NOW = 1_800_000_000_000
const HOUR = 3_600_000
const usage = usageFixture(NOW)
const account = (id: string | null | undefined): ClaudeAccount | null =>
  usage.claude.accounts.find((a) => a.id === id) ?? null

function failure(
  extra: Partial<NonNullable<SystemMessage['failure']>>,
  id = 'f1',
): SystemMessage {
  return {
    id,
    role: 'system',
    kind: 'turn-failure',
    content: 'failed',
    createdAt: NOW,
    failure: extra,
  }
}

const user: Message = { id: 'u1', role: 'user', content: 'hi', createdAt: NOW }
const notice: Message = {
  id: 'n1',
  role: 'system',
  kind: 'notice',
  content: 'note',
  createdAt: NOW,
}

afterEach(() => resetLimitDismissalsForTests())

describe('lastTurnFailure', () => {
  it('finds the failure behind trailing notices only', () => {
    const f = failure({})
    expect(lastTurnFailure([user, f, notice])).toBe(f)
    expect(lastTurnFailure([f, user])).toBeNull()
    expect(lastTurnFailure([])).toBeNull()
  })
})

describe('limitCardModel', () => {
  const base = {
    sessionId: 's1',
    failure: null,
    idle: true,
    resolved: 'default',
    account,
    now: NOW,
  }

  it('shows for a turn that failed with llm.usage_limit, from its details', () => {
    const model = limitCardModel({
      ...base,
      failure: failure({
        usageLimit: {
          window: 'week',
          resetsAtMs: NOW + 5 * HOUR,
          account: 'personal',
        },
      }),
    })
    expect(model).toMatchObject({
      kind: 'failed',
      account: 'personal',
      window: 'week',
      resetsAtMs: NOW + 5 * HOUR,
      reset: false,
    })
  })

  it('turns a failed card to "reset" once its window reset, and drops a stale reading', () => {
    const failed = failure({
      usageLimit: {
        window: 'five_hour',
        resetsAtMs: NOW + HOUR,
        account: 'personal',
      },
    })
    const later = { ...base, now: NOW + HOUR + 1, resolved: 'personal' }
    expect(limitCardModel({ ...later, failure: failed })).toMatchObject({
      kind: 'failed',
      reset: true,
    })
    // Personal's reading still says 100 %, but its window reset since.
    expect(limitCardModel({ ...later, failure: null })).toBeNull()
  })

  it('names the last turn failing on its account', () => {
    expect(
      accountFailure(
        failure({
          usageLimit: { window: null, resetsAtMs: null, account: null },
        }),
      ),
    ).toBe(true)
    expect(accountFailure(failure({ unknownAccount: 'gone' }))).toBe(true)
    expect(accountFailure(failure({ summary: 'billing' }))).toBe(false)
    expect(accountFailure(null)).toBe(false)
  })

  it("fills a failure's missing window from the account's reading", () => {
    const model = limitCardModel({
      ...base,
      resolved: 'personal',
      failure: failure({
        usageLimit: { window: null, resetsAtMs: null, account: null },
      }),
    })
    expect(model).toMatchObject({
      kind: 'failed',
      account: 'personal',
      window: 'five_hour',
      resetsAtMs: NOW + HOUR,
    })
  })

  it('shows when idle on an account at 100 %, and not below it', () => {
    expect(limitCardModel({ ...base, resolved: 'personal' })).toMatchObject({
      kind: 'reading',
      account: 'personal',
      window: 'five_hour',
    })
    expect(limitCardModel(base)).toBeNull()
  })

  it('shows for a pin to a removed account', () => {
    expect(
      limitCardModel({ ...base, failure: failure({ unknownAccount: 'gone' }) }),
    ).toMatchObject({ kind: 'unknown', account: 'gone' })
  })

  it('hides while a turn runs', () => {
    expect(
      limitCardModel({
        ...base,
        idle: false,
        resolved: 'personal',
        failure: failure({
          usageLimit: { window: 'week', resetsAtMs: null, account: 'personal' },
        }),
      }),
    ).toBeNull()
  })

  it('ignores other failures', () => {
    expect(
      limitCardModel({ ...base, failure: failure({ summary: 'billing' }) }),
    ).toBeNull()
  })
})

describe('dismissal', () => {
  it('stays put away until the reading changes', () => {
    const at = (pct: number) =>
      limitCardModel({
        sessionId: 's1',
        failure: null,
        idle: true,
        resolved: 'personal',
        now: NOW,
        account: (id) => {
          const a = account(id)
          if (!a?.usage?.fiveHour) return a
          return {
            ...a,
            usage: {
              ...a.usage,
              fiveHour: { ...a.usage.fiveHour, usedPercent: pct },
            },
          }
        },
      })
    const first = at(100)
    if (!first) throw new Error('no card')
    dismissLimit('s1', first.key)
    expect(limitDismissed('s1', first.key)).toBe(true)
    expect(limitDismissed('s2', first.key)).toBe(false)
    // The same reading again keeps it away…
    const same = at(100)
    expect(same && limitDismissed('s1', same.key)).toBe(true)
    // …a new reading (99.8 %, still full once rounded) brings it back.
    const next = at(99.8)
    expect(next).not.toBeNull()
    expect(next && limitDismissed('s1', next.key)).toBe(false)
  })
})
