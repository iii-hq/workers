import { afterEach, describe, expect, it } from 'vitest'
import {
  rawUsage,
  splitsFixture,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import {
  applyUsageSnapshot,
  claudeAccountsGate,
  getClaudeAccountsState,
  parseAccountSplits,
  parseAccountsError,
  parseClaudeSnapshot,
  parseUsageSnapshot,
  seedClaudeAccountsForTests,
} from './claude-accounts'

const NOW = 1_800_000_000_000

afterEach(() => seedClaudeAccountsForTests(null))

describe('parsing tmux', () => {
  it("reads tmux's usage snapshot", () => {
    const usage = usageFixture(NOW)
    expect(usage.generatedAtMs).toBe(NOW - 240_000)
    expect(usage.claude.global).toBe('default')
    expect(usage.claude.rules.mode).toBe('suggest')
    const [work, personal, side] = usage.claude.accounts
    expect(work).toMatchObject({
      id: 'default',
      name: 'Work',
      isDefault: true,
      signedIn: true,
      plan: 'team',
    })
    expect(work.usage?.week).toEqual({
      usedPercent: 64,
      resetsAtMs: NOW + 50 * 3_600_000,
      resetSinceRead: false,
    })
    expect(personal.tier).toBe('default_claude_max_20x')
    expect(side).toMatchObject({ signedIn: false, usage: null })
  })

  it('keeps defaults for what is missing and drops accounts without an id', () => {
    const snapshot = parseClaudeSnapshot({
      accounts: [{ name: 'no id' }, { id: 'a' }],
      rules: { mode: 'weird' },
      suggestion: { id: 's1', from: 'a' },
      pending_global: {
        account: 'a',
        waiting_for: ['%3', 4],
      },
    })
    expect(snapshot.global).toBe('default')
    expect(snapshot.accounts.map((a) => [a.id, a.name, a.color])).toEqual([
      ['a', 'a', 'blue'],
    ])
    expect(snapshot.rules.mode).toBe('suggest')
    expect(snapshot.suggestion).toBeNull()
    expect(snapshot.pendingGlobal).toMatchObject({
      account: 'a',
      by: 'person',
      waitingFor: ['%3'],
    })
    expect(parseUsageSnapshot({ generated_at_ms: 1 })).toBeNull()
  })

  it("reads the harness's split, as an array or {accounts}", () => {
    const splits = splitsFixture(NOW)
    expect(splits.map((s) => [s.account, s.outsideMeasured])).toEqual([
      ['default', true],
      ['personal', false],
      ['side-project', true],
    ])
    expect(splits[0].segments[0]).toEqual({
      kind: 'project',
      projectId: 'prj_acme_web',
      name: 'acme-web',
      color: 'blue',
      percent: 33,
    })
    expect(
      parseAccountSplits({
        accounts: [
          {
            account: 'x',
            segments: [{ kind: 'mystery' }, { kind: 'outside', percent: 3 }],
          },
          { segments: [] },
        ],
      }),
    ).toEqual([
      {
        account: 'x',
        weekPercent: 0,
        resetsAtMs: null,
        outsideMeasured: true,
        segments: [{ kind: 'outside', percent: 3 }],
      },
    ])
  })
})

describe('the feature gate (§4.3.6)', () => {
  it('needs harness::retry and session::patch-meta, then a tmux with the credential', () => {
    const gate = (
      harnessRetry: boolean,
      tmuxAccounts: boolean,
      tmuxCredential: boolean,
      patchMeta = true,
    ) =>
      claudeAccountsGate({
        harnessRetry,
        tmuxAccounts,
        tmuxCredential,
        patchMeta,
      })
    expect(gate(false, true, true)).toBe('none')
    expect(gate(true, true, true)).toBe('full')
    expect(gate(true, true, false)).toBe('update')
    expect(gate(true, false, false)).toBe('install')
    // Never a set-meta for the account: no patch-meta, no accounts.
    expect(gate(true, true, true, false)).toBe('none')
  })
})

describe('the store', () => {
  it('keeps the newest reading', () => {
    seedClaudeAccountsForTests({ usage: null })
    applyUsageSnapshot(usageFixture(NOW))
    expect(getClaudeAccountsState().usage?.generatedAtMs).toBe(NOW - 240_000)
    // An older overview arriving late changes nothing.
    applyUsageSnapshot(parseUsageSnapshot(rawUsage(NOW - 600_000)))
    expect(getClaudeAccountsState().usage?.generatedAtMs).toBe(NOW - 240_000)
    applyUsageSnapshot(parseUsageSnapshot(rawUsage(NOW + 60_000)))
    expect(getClaudeAccountsState().usage?.generatedAtMs).toBe(
      NOW + 60_000 - 240_000,
    )
  })
})

describe("the worker's errors", () => {
  it('reads {code, message, blocked_by} out of the error text', () => {
    const err = new Error(
      'function failed: {"code":"blocked","message":"Personal is in use","blocked_by":["3"]}',
    )
    expect(parseAccountsError(err)).toEqual({
      code: 'blocked',
      message: 'Personal is in use',
      blockedBy: ['3'],
    })
  })

  it('reads a nested message and a plain object', () => {
    const nested = {
      error: {
        message: JSON.stringify({
          message: '{"code":"busy","message":"locks held"}',
        }),
      },
    }
    expect(parseAccountsError(nested).code).toBe('busy')
    expect(
      parseAccountsError({ code: 'not_signed_in', message: 'Sign in first' }),
    ).toEqual({
      code: 'not_signed_in',
      message: 'Sign in first',
      blockedBy: [],
    })
  })

  it('falls back to the text', () => {
    expect(parseAccountsError(new Error('timeout'))).toEqual({
      code: null,
      message: 'timeout',
      blockedBy: [],
    })
  })
})
