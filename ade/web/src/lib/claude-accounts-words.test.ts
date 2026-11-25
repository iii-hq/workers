import { describe, expect, it } from 'vitest'
import {
  splitsFixture,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import type { ClaudeAccount } from './backend/claude-accounts'
import {
  accountUse,
  accountWeek,
  ago,
  atLimit,
  blocked,
  cardResetLine,
  claudePlan,
  defaultChoiceLabel,
  foldersCell,
  footerTitle,
  globalRefusal,
  globalToast,
  level,
  limitHit,
  limitResetLine,
  limitTitle,
  meterNote,
  mostRoom,
  NO_CHAT_SPEND,
  noReading,
  OUTSIDE_NOT_MEASURED,
  percent,
  projectAccountCell,
  resetCompact,
  resets,
  resolutionText,
  resolveAccount,
  sessionsCell,
  splitSpoken,
  splitView,
  suggestionBody,
  suggestionTitle,
  waitingBanner,
} from './claude-accounts-words'

/* Sat 2026-11-21 10:00 UTC. */
const NOW = Date.UTC(2026, 10, 21, 10, 0)
const HOUR = 3_600_000
const UTC = { now: NOW, timeZone: 'UTC', hour24: true }
const UTC12 = { now: NOW, timeZone: 'UTC', hour24: false }

const usage = usageFixture(NOW)
const [work, personal, side] = usage.claude.accounts
const nameOf = (id: string) =>
  usage.claude.accounts.find((a) => a.id === id)?.name ?? id

describe('levels', () => {
  it('rounds, clamps and colours a window', () => {
    expect(percent(64.4)).toBe(64)
    expect(percent(140)).toBe(100)
    expect(percent(Number.NaN)).toBe(0)
    expect(level(79.4)).toBe('normal')
    expect(level(80)).toBe('high')
    expect(level(99.4)).toBe('high')
    expect(level(99.6)).toBe('full')
    expect(level(null)).toBe('normal')
  })

  it('names the plan as people say it', () => {
    expect(claudePlan('max', 'default_claude_max_20x')).toBe('Max 20×')
    expect(claudePlan('max', null)).toBe('Max')
    expect(claudePlan('team')).toBe('Team')
    expect(claudePlan('custom')).toBe('Custom')
    expect(claudePlan(null)).toBeNull()
  })
})

describe('reset times', () => {
  it('writes them in the browser zone, 24-hour or 12-hour', () => {
    expect(resetCompact(NOW + 6 * HOUR, UTC)).toBe('16:00')
    expect(resetCompact(NOW + 6 * HOUR, UTC12)).toBe('4:00 PM')
    expect(resetCompact(NOW + 23 * HOUR, UTC)).toBe('tomorrow 09:00')
    expect(resetCompact(NOW + 50 * HOUR, UTC)).toBe('Mon 12:00')
    expect(resetCompact(NOW + 9 * 24 * HOUR, UTC)).toBe('Nov 30')
  })

  it('says when a window resets, or that it already did', () => {
    expect(resets(work.usage?.week, UTC)).toBe('Resets Mon 12:00')
    expect(
      resets({ usedPercent: 0, resetsAtMs: null, resetSinceRead: true }, UTC),
    ).toBe('Reset since the last reading')
    expect(meterNote(personal.usage?.fiveHour, UTC)).toBe(
      'Limit reached · 11:00',
    )
    expect(meterNote(null, UTC)).toBe('Not started')
    expect(cardResetLine(work, UTC)).toBe('Resets Mon 12:00 · Opus 41%')
  })

  it('ages a reading', () => {
    expect(ago(NOW - 30_000, NOW)).toBe('just now')
    expect(ago(NOW - 4 * 60_000, NOW)).toBe('4 min ago')
    expect(ago(NOW - 3 * HOUR, NOW)).toBe('3 h ago')
    expect(ago(NOW - 49 * HOUR, NOW)).toBe('2 days ago')
    expect(footerTitle(NOW - 4 * 60_000, NOW)).toBe(
      'Usage this week · 4 min ago',
    )
    expect(footerTitle(null, NOW)).toBe('Usage this week')
  })
})

describe('accounts', () => {
  it('says why an account has no meters', () => {
    expect(noReading(side)).toBe('Not signed in')
    expect(noReading(work)).toBeNull()
    expect(
      noReading({ ...work, usage: null, tokenExpired: true } as ClaudeAccount),
    ).toBe('Signed out on this host')
  })

  it('picks the account with the most room, never one at its 5-hour limit', () => {
    // Personal has the lowest week but is at its 5-hour limit; Side isn't signed in.
    expect(mostRoom(usage.claude)?.id).toBe('default')
    expect(mostRoom(usage.claude, 'default')).toBeNull()
  })

  it('never picks an account whose week is full', () => {
    const fullWeek = {
      ...work,
      usage: work.usage && {
        ...work.usage,
        week: { usedPercent: 100, resetsAtMs: NOW + 50 * HOUR },
      },
    } as ClaudeAccount
    expect(atLimit(fullWeek)).toBe(true)
    expect(atLimit(personal)).toBe(true)
    expect(atLimit(work)).toBe(false)
    expect(mostRoom({ accounts: [fullWeek, personal, side] })).toBeNull()
  })

  it("writes Other…'s rows and the pin's week", () => {
    expect(accountUse(personal)).toBe('5 hours 100% · This week 22%')
    expect(accountUse(side)).toBe('Not signed in')
    expect(accountWeek(work)).toBe('Work · 64% this week')
    expect(accountWeek(side)).toBe('Side project')
  })
})

describe('the global switch', () => {
  it("says the worker's refusals in the apps' words", () => {
    expect(
      globalRefusal(
        { code: 'blocked', message: 'x', blockedBy: ['3', '5'] },
        'Personal',
      ),
    ).toBe(
      'Personal is in use by a Claude Code you started yourself (windows 3, 5). Close or move them, then try again.',
    )
    expect(
      globalRefusal({ code: 'busy', message: '', blockedBy: [] }, 'Personal'),
    ).toBe('Claude Code is renewing its sign-in; try again in a moment.')
    expect(
      globalRefusal(
        { code: 'signing_in', message: 'Work is signing in.', blockedBy: [] },
        'Personal',
      ),
    ).toBe('Work is signing in.')
    expect(
      globalRefusal({ code: null, message: 'boom', blockedBy: [] }, 'P'),
    ).toBe('Could not switch the global account: boom')
    expect(blocked('Work', ['2'])).toBe(
      'Work is in use by a Claude Code you started yourself (window 2). Close it or move it, then try again.',
    )
  })

  it('reports a switch and a waiting one', () => {
    expect(globalToast('Personal', 3, 0, 0)).toBe(
      'Personal is now global. 3 Claude Code sessions switch with their next message.',
    )
    expect(globalToast('Personal', 0, 1, 2)).toBe(
      'New Claude Code sessions start on Personal. 1 pinned window restarts. 2 restart when their turn ends.',
    )
    expect(waitingBanner('Work', 1)).toBe(
      'Switching to Work when 1 window pinned to it finishes its turn.',
    )
  })

  it('writes the suggestion card', () => {
    expect(suggestionTitle(work, 'default', 'week_90')).toBe(
      'Work, the global account, is at 64%',
    )
    expect(suggestionTitle(work, 'default', 'five_hour')).toBe(
      'Work, the global account, reached its 5-hour limit',
    )
    expect(suggestionBody(personal, 'personal', true, 2, true)).toBe(
      'Personal has the most room this week: 22%. 2 sessions follow the global account.',
    )
  })
})

describe('the week split', () => {
  const splits = splitsFixture(NOW)
  const of = (id: string) => splits.find((s) => s.account === id)

  it('keeps the engine order with a grey segment for outside the Chat', () => {
    const view = splitView(of('default'))
    expect(view.items.map((item) => [item.label, item.tone])).toEqual([
      ['acme-web 33%', 'blue'],
      ['mobile 12%', 'purple'],
      ['no project 5%', 'no-project'],
      ['Outside the Chat 14%', 'outside'],
    ])
    expect(view.note).toBeNull()
    expect(splitSpoken(view)).toBe(
      'This week by project: acme-web 33%, mobile 12%, no project 5%, Outside the Chat 14%',
    )
  })

  it('drops the grey segment when outside_measured is false, and says so', () => {
    const view = splitView(of('personal'))
    expect(view.items.map((item) => item.tone)).toEqual(['green'])
    expect(view.note).toBe(OUTSIDE_NOT_MEASURED)
    expect(view.empty).toBe(false)
  })

  it('shows an all-grey bar when no project used the account', () => {
    const view = splitView(of('side-project'))
    expect(view.items.map((item) => item.tone)).toEqual(['outside'])
    expect(view.note).toBe(NO_CHAT_SPEND)
  })

  it('shows nothing without a split or a use', () => {
    expect(splitView(null).empty).toBe(true)
    expect(
      splitView({
        account: 'x',
        weekPercent: 0,
        resetsAtMs: null,
        outsideMeasured: true,
        segments: [{ kind: 'outside', percent: 0 }],
      }),
    ).toEqual({ items: [], note: null, empty: true })
  })
})

describe('where a conversation runs (§4.3.5)', () => {
  const acme = { name: 'acme-web', account: { mode: 'global' as const } }
  const billing = {
    name: 'billing-api',
    account: { mode: 'pinned' as const, account_id: 'personal' },
  }

  it("follows the harness's order", () => {
    expect(
      resolveAccount({
        claudeAccount: 'personal',
        project: acme,
        global: 'default',
      }),
    ).toEqual({ account: 'personal', source: 'session' })
    expect(
      resolveAccount({
        claudeAccount: 'global',
        project: billing,
        global: 'default',
      }),
    ).toEqual({ account: 'default', source: 'global' })
    expect(resolveAccount({ project: billing, global: 'default' })).toEqual({
      account: 'personal',
      source: 'project',
      projectName: 'billing-api',
    })
    expect(resolveAccount({ project: null, global: 'default' })).toEqual({
      account: 'default',
      source: 'global',
    })
  })

  it('says it as the row does', () => {
    const text = (
      input: Parameters<typeof resolveAccount>[0],
      explicit = false,
    ) => resolutionText(resolveAccount(input), nameOf, explicit)
    expect(text({ project: acme, global: 'default' })).toBe(
      'Runs on Work, the global account, because acme-web follows the global.',
    )
    expect(text({ project: null, global: 'default' })).toBe(
      'Runs on Work, the global account.',
    )
    expect(
      text(
        { claudeAccount: 'global', project: billing, global: 'default' },
        true,
      ),
    ).toBe('Runs on Work, the global account.')
    expect(text({ project: billing, global: 'default' })).toBe(
      'Runs on Personal, pinned by billing-api.',
    )
    expect(
      text({ claudeAccount: 'personal', project: acme, global: 'default' }),
    ).toBe('Runs on Personal, pinned for this conversation.')
  })

  it("labels the select's default option", () => {
    expect(
      defaultChoiceLabel({ hasProject: false, globalName: 'Work', nameOf }),
    ).toBe('Global (Work)')
    expect(
      defaultChoiceLabel({
        hasProject: true,
        projectAccount: { mode: 'global' },
        globalName: 'Work',
        nameOf,
      }),
    ).toBe("Project's (Work, global)")
    expect(
      defaultChoiceLabel({
        hasProject: true,
        projectAccount: billing.account,
        globalName: 'Work',
        nameOf,
      }),
    ).toBe("Project's (Personal)")
  })
})

describe('the Projects tab', () => {
  it('writes its cells', () => {
    expect(projectAccountCell({ mode: 'global' }, 'Work', nameOf)).toBe(
      'Follow global (Work)',
    )
    expect(
      projectAccountCell(
        { mode: 'pinned', account_id: 'personal' },
        'Work',
        nameOf,
      ),
    ).toBe('Personal (pinned)')
    expect(sessionsCell(4, 1)).toBe('4 · 1 working')
    expect(sessionsCell(6, 0)).toBe('6')
    const tilde = (p: string) => p.replace('/home/me', '~')
    expect(
      foldersCell(
        ['/home/me/work/acme/web/', '/home/me/work/acme/api', '/srv/infra'],
        tilde,
      ),
    ).toBe('~/work/acme/web, api, /srv/infra')
  })
})

describe('the limit card', () => {
  it('finds the full window, the 5-hour one first', () => {
    expect(limitHit(personal)?.window).toBe('five_hour')
    expect(limitHit(work)).toBeNull()
    expect(
      limitHit({
        ...work,
        usage: work.usage && {
          ...work.usage,
          week: { usedPercent: 100, resetsAtMs: NOW, resetSinceRead: false },
        },
      })?.window,
    ).toBe('week')
  })

  it('names the window and its reset', () => {
    expect(limitTitle('Personal', 'five_hour')).toBe(
      'Personal reached its 5-hour limit',
    )
    expect(limitTitle('Personal', 'week')).toBe(
      'Personal reached its weekly limit',
    )
    expect(limitTitle('Personal', null)).toBe(
      'Personal reached its usage limit',
    )
    expect(limitResetLine(NOW + 6 * HOUR + 40 * 60_000, UTC12)).toBe(
      'Resets 4:40 PM',
    )
    expect(limitResetLine(null, UTC)).toBeNull()
  })
})
