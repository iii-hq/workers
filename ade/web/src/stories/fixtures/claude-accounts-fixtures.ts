/**
 * Claude Code accounts as tmux and the harness answer them (the Acme demo,
 * never real accounts): three accounts, Work the global one at 64 % of its
 * week, Personal at its 5-hour limit, "Side project" not signed in; and the
 * week's split of each. Stories and tests seed the store with them.
 */
import {
  type AccountSplit,
  type ClaudeAccounts,
  type ClaudeAccountsGate,
  parseAccountSplits,
  parseUsageSnapshot,
  type UsageSnapshot,
} from '@/lib/backend/claude-accounts'

const HOUR = 3_600_000

/** The raw `tmux::usage::read` answer at `now`. */
export function rawUsage(now: number, overrides: Record<string, unknown> = {}) {
  return {
    generated_at_ms: now - 4 * 60_000,
    claude: {
      available: true,
      managed: true,
      global: 'default',
      switch_delay_s: 0,
      accounts: [
        {
          id: 'default',
          name: 'Work',
          color: 'blue',
          is_default: true,
          email: 'me@acme.dev',
          plan: 'team',
          signed_in: true,
          token_expired: false,
          dir_display: '~/.claude',
          panes: 3,
          usage: {
            five_hour: { used_percent: 38, resets_at_ms: now + 2 * HOUR },
            week: { used_percent: 64, resets_at_ms: now + 50 * HOUR },
            week_opus: { used_percent: 41, resets_at_ms: now + 50 * HOUR },
            as_of_ms: now - 4 * 60_000,
          },
        },
        {
          id: 'personal',
          name: 'Personal',
          color: 'violet',
          is_default: false,
          email: 'me@home.dev',
          plan: 'max',
          tier: 'default_claude_max_20x',
          signed_in: true,
          token_expired: false,
          dir_display: '~/.local/share/iii/claude-accounts/personal',
          panes: 1,
          usage: {
            five_hour: { used_percent: 100, resets_at_ms: now + HOUR },
            week: { used_percent: 22, resets_at_ms: now + 90 * HOUR },
            as_of_ms: now - 6 * 60_000,
          },
        },
        {
          id: 'side-project',
          name: 'Side project',
          color: 'green',
          is_default: false,
          signed_in: false,
          token_expired: false,
          dir_display: '~/.local/share/iii/claude-accounts/side-project',
          panes: 0,
        },
      ],
      rules: {
        mode: 'suggest',
        at: 'week_90',
        five_hour: true,
        back_after_reset: false,
        order: [],
      },
      suggestion: null,
      pending_global: null,
      logins: [],
      ...overrides,
    },
  }
}

export function usageFixture(
  now: number,
  overrides: Record<string, unknown> = {},
): UsageSnapshot {
  const parsed = parseUsageSnapshot(rawUsage(now, overrides))
  if (!parsed) throw new Error('fixture: no claude snapshot')
  return parsed
}

/** The raw `harness::metrics::account-split` answer. */
export function rawSplits(now: number) {
  return [
    {
      account: 'default',
      week_percent: 64,
      resets_at_ms: now + 50 * HOUR,
      outside_measured: true,
      segments: [
        {
          kind: 'project',
          project_id: 'prj_acme_web',
          name: 'acme-web',
          color: 'blue',
          percent: 33,
        },
        {
          kind: 'project',
          project_id: 'prj_mobile',
          name: 'mobile',
          color: 'purple',
          percent: 12,
        },
        { kind: 'no_project', percent: 5 },
        { kind: 'outside', percent: 14 },
      ],
    },
    {
      account: 'personal',
      week_percent: 22,
      resets_at_ms: now + 90 * HOUR,
      outside_measured: false,
      segments: [
        {
          kind: 'project',
          project_id: 'prj_billing',
          name: 'billing-api',
          color: 'green',
          percent: 15,
        },
        { kind: 'outside', percent: 7 },
      ],
    },
    {
      account: 'side-project',
      week_percent: 9,
      resets_at_ms: now + 100 * HOUR,
      outside_measured: true,
      segments: [{ kind: 'outside', percent: 9 }],
    },
  ]
}

export function splitsFixture(now: number): AccountSplit[] {
  return parseAccountSplits(rawSplits(now))
}

/** What `useClaudeAccounts` hands a view, for a component rendered without the store. */
export function accountsData(
  usage: UsageSnapshot | null,
  splits: AccountSplit[] | null = null,
  gate: ClaudeAccountsGate = 'full',
): ClaudeAccounts {
  const claude = usage?.claude ?? null
  const byId = new Map(claude?.accounts.map((a) => [a.id, a]) ?? [])
  const splitById = new Map(splits?.map((s) => [s.account, s]) ?? [])
  return {
    loaded: true,
    usage,
    error: null,
    refreshing: false,
    splits,
    splitsError: null,
    gate,
    capsLoaded: true,
    claude,
    account: (id) => (id ? (byId.get(id) ?? null) : null),
    nameOf: (id) => byId.get(id)?.name ?? id,
    split: (id) => splitById.get(id) ?? null,
  }
}
