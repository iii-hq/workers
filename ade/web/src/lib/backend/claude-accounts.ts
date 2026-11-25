/**
 * Claude Code accounts in the Chat (chat-projects §4, D30, D143): the one
 * client type every account surface goes through — the provider screen, the
 * picker's cards, the project dialog, Conversation details, the header chip,
 * the usage footer and the limit card.
 *
 * tmux owns the accounts in this phase (D20). A later project moves them into
 * `provider-claude-code`; then only this file changes. It wraps:
 * - `tmux::accounts::{list, global, add, update, remove, rules, dismiss,
 *   login_start, login_code, login_cancel}` and `tmux::usage::read`;
 * - the `tmux::on-changed` feed, keeping the newest `usage.generated_at_ms`
 *   (iii doesn't order deliveries across calls);
 * - `harness::retry` and `harness::metrics::account-split`;
 * - `session::patch-meta` for a conversation's `claude_account`.
 *
 * Feature detection is §4.3.6 (D144): `claudeAccountsGate` reads the
 * `harnessRetry`, `tmuxAccounts` and `tmuxCredential` capabilities.
 *
 * One store for the tab, read with `useSyncExternalStore`. It wakes only when
 * the engine has the whole feature and a component asks for it; mock and
 * Storybook backends seed it with `seedClaudeAccountsForTests`.
 */
import { useEffect, useMemo, useSyncExternalStore } from 'react'
import {
  type ChatCapabilities,
  useChatCapabilities,
} from '@/lib/chat-capabilities'
import { errText } from '@/lib/errors'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { patchSessionMeta } from '@/lib/sessions/api'

export const ACCOUNTS_LIST_FUNCTION_ID = 'tmux::accounts::list'
export const ACCOUNTS_GLOBAL_FUNCTION_ID = 'tmux::accounts::global'
export const ACCOUNTS_ADD_FUNCTION_ID = 'tmux::accounts::add'
export const ACCOUNTS_UPDATE_FUNCTION_ID = 'tmux::accounts::update'
export const ACCOUNTS_REMOVE_FUNCTION_ID = 'tmux::accounts::remove'
export const ACCOUNTS_RULES_FUNCTION_ID = 'tmux::accounts::rules'
export const ACCOUNTS_DISMISS_FUNCTION_ID = 'tmux::accounts::dismiss'
export const LOGIN_START_FUNCTION_ID = 'tmux::accounts::login_start'
export const LOGIN_CODE_FUNCTION_ID = 'tmux::accounts::login_code'
export const LOGIN_CANCEL_FUNCTION_ID = 'tmux::accounts::login_cancel'
export const USAGE_READ_FUNCTION_ID = 'tmux::usage::read'
export const TMUX_CHANGED_FUNCTION_ID = 'tmux::on-changed'
export const RETRY_FUNCTION_ID = 'harness::retry'
export const ACCOUNT_SPLIT_FUNCTION_ID = 'harness::metrics::account-split'

/** `metadata.claude_account` that follows the global account. */
export const FOLLOW_GLOBAL = 'global'

/** The provider whose conversations run on these accounts. */
export const CLAUDE_CODE_PROVIDER = 'claude-code'

/** The turn failure code of a usage limit (D147). */
export const USAGE_LIMIT_CODE = 'llm.usage_limit'
/** The turn failure code of a pin to a removed account. */
export const UNKNOWN_ACCOUNT_CODE = 'harness.unknown_account'

/* ── the wire ───────────────────────────────────────────────────────── */

export interface UsageLimit {
  /** 0 to 100. */
  usedPercent: number
  resetsAtMs: number | null
  /** The window reset after the reading: 0 % until the next one. */
  resetSinceRead: boolean
}

export interface ClaudeUsage {
  fiveHour: UsageLimit | null
  week: UsageLimit | null
  weekOpus: UsageLimit | null
  weekSonnet: UsageLimit | null
  asOfMs: number
  /** Why the latest read failed (`rate_limited`, `unauthorized`, …). */
  error: string | null
}

export interface ClaudeAccount {
  id: string
  name: string
  /** `blue`, `green`, `violet`, `orange` or `pink`. */
  color: string
  isDefault: boolean
  email: string | null
  plan: string | null
  tier: string | null
  signedIn: boolean
  tokenExpired: boolean
  /** Its folder, `~` for the home folder. */
  dirDisplay: string | null
  /** Claude Code panes using it now. */
  panes: number
  usage: ClaudeUsage | null
}

export type RulesMode = 'off' | 'suggest' | 'switch'

export interface SwitchRules {
  mode: RulesMode
  at: string
  fiveHour: boolean
  backAfterReset: boolean
  order: string[]
}

export interface Suggestion {
  id: string
  from: string
  to: string
  reason: string
  following: number
  atMs: number
}

export interface PendingGlobal {
  account: string
  by: string
  waitingFor: string[]
  atMs: number
}

export type LoginState =
  | 'waiting_code'
  | 'checking'
  | 'done'
  | 'failed'
  | 'expired'

export interface ClaudeLogin {
  loginId: string
  account: string
  state: LoginState
  url: string
  expiresAtMs: number
  email: string | null
  error: { code: string; message: string } | null
}

export interface ClaudeSnapshot {
  /** A `claude` binary is on PATH or `~/.claude` exists. */
  available: boolean
  managed: boolean
  /** Id of the global account. */
  global: string
  accounts: ClaudeAccount[]
  rules: SwitchRules
  suggestion: Suggestion | null
  pendingGlobal: PendingGlobal | null
  /** 0 on Linux, 30 on macOS. */
  switchDelayS: number
  logins: ClaudeLogin[]
}

export interface UsageSnapshot {
  claude: ClaudeSnapshot
  generatedAtMs: number
}

export type SplitSegment =
  | {
      kind: 'project'
      projectId: string
      name: string
      color: string
      percent: number
    }
  | { kind: 'no_project'; percent: number }
  | { kind: 'outside'; percent: number }

export interface AccountSplit {
  account: string
  weekPercent: number
  resetsAtMs: number | null
  /** False when the outside reading failed: the Chat's segments split the week alone. */
  outsideMeasured: boolean
  segments: SplitSegment[]
}

function record(raw: unknown): Record<string, unknown> {
  return raw && typeof raw === 'object' && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : {}
}

function str(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function strings(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string')
    : []
}

export function parseLimit(raw: unknown): UsageLimit | null {
  if (!raw || typeof raw !== 'object') return null
  const r = record(raw)
  const used = num(r.used_percent)
  if (used === null) return null
  return {
    usedPercent: used,
    resetsAtMs: num(r.resets_at_ms),
    resetSinceRead: r.reset_since_read === true,
  }
}

function parseUsage(raw: unknown): ClaudeUsage | null {
  if (!raw || typeof raw !== 'object') return null
  const r = record(raw)
  return {
    fiveHour: parseLimit(r.five_hour),
    week: parseLimit(r.week),
    weekOpus: parseLimit(r.week_opus),
    weekSonnet: parseLimit(r.week_sonnet),
    asOfMs: num(r.as_of_ms) ?? 0,
    error: str(r.error),
  }
}

export function parseAccount(raw: unknown): ClaudeAccount | null {
  const r = record(raw)
  const id = str(r.id)
  if (!id) return null
  return {
    id,
    name: str(r.name) ?? id,
    color: str(r.color) ?? 'blue',
    isDefault: r.is_default === true,
    email: str(r.email),
    plan: str(r.plan),
    tier: str(r.tier),
    signedIn: r.signed_in === true,
    tokenExpired: r.token_expired === true,
    dirDisplay: str(r.dir_display),
    panes: num(r.panes) ?? 0,
    usage: parseUsage(r.usage),
  }
}

function parseLogin(raw: unknown): ClaudeLogin | null {
  const r = record(raw)
  const loginId = str(r.login_id)
  const account = str(r.account)
  if (!loginId || !account) return null
  const state = str(r.state)
  const error = record(r.error)
  return {
    loginId,
    account,
    state:
      state === 'checking' ||
      state === 'done' ||
      state === 'failed' ||
      state === 'expired'
        ? state
        : 'waiting_code',
    url: str(r.url) ?? '',
    expiresAtMs: num(r.expires_at_ms) ?? 0,
    email: str(r.email),
    error: str(error.code)
      ? { code: str(error.code) ?? '', message: str(error.message) ?? '' }
      : null,
  }
}

/** `tmux::accounts::list`'s answer, or the `claude` of a usage snapshot. */
export function parseClaudeSnapshot(raw: unknown): ClaudeSnapshot {
  const r = record(raw)
  const rules = record(r.rules)
  const suggestion = record(r.suggestion)
  const pending = record(r.pending_global)
  const mode = str(rules.mode)
  return {
    available: r.available !== false,
    managed: r.managed === true,
    global: str(r.global) ?? 'default',
    accounts: Array.isArray(r.accounts)
      ? r.accounts.flatMap((a) => {
          const parsed = parseAccount(a)
          return parsed ? [parsed] : []
        })
      : [],
    rules: {
      mode: mode === 'off' || mode === 'switch' ? mode : 'suggest',
      at: str(rules.at) ?? 'week_90',
      fiveHour: rules.five_hour === true,
      backAfterReset: rules.back_after_reset === true,
      order: strings(rules.order),
    },
    suggestion:
      str(suggestion.id) && str(suggestion.to) && str(suggestion.from)
        ? {
            id: str(suggestion.id) ?? '',
            from: str(suggestion.from) ?? '',
            to: str(suggestion.to) ?? '',
            reason: str(suggestion.reason) ?? '',
            following: num(suggestion.following) ?? 0,
            atMs: num(suggestion.at_ms) ?? 0,
          }
        : null,
    pendingGlobal: str(pending.account)
      ? {
          account: str(pending.account) ?? '',
          by: str(pending.by) ?? 'person',
          waitingFor: strings(pending.waiting_for),
          atMs: num(pending.at_ms) ?? 0,
        }
      : null,
    switchDelayS: num(r.switch_delay_s) ?? 0,
    logins: Array.isArray(r.logins)
      ? r.logins.flatMap((l) => {
          const parsed = parseLogin(l)
          return parsed ? [parsed] : []
        })
      : [],
  }
}

/** `tmux::usage::read`'s answer, or the `usage` of a `tmux::on-changed` overview. */
export function parseUsageSnapshot(raw: unknown): UsageSnapshot | null {
  const r = record(raw)
  if (!r.claude || typeof r.claude !== 'object') return null
  return {
    claude: parseClaudeSnapshot(r.claude),
    generatedAtMs: num(r.generated_at_ms) ?? 0,
  }
}

function parseSegment(raw: unknown): SplitSegment | null {
  const r = record(raw)
  const pct = num(r.percent) ?? 0
  switch (r.kind) {
    case 'project':
      return {
        kind: 'project',
        projectId: str(r.project_id) ?? '',
        name: str(r.name) ?? str(r.project_id) ?? 'project',
        color: str(r.color) ?? 'blue',
        percent: pct,
      }
    case 'no_project':
      return { kind: 'no_project', percent: pct }
    case 'outside':
      return { kind: 'outside', percent: pct }
    default:
      return null
  }
}

/** `harness::metrics::account-split`'s answer: an array (or `{accounts}`). */
export function parseAccountSplits(raw: unknown): AccountSplit[] {
  const rows = Array.isArray(raw)
    ? raw
    : Array.isArray(record(raw).accounts)
      ? (record(raw).accounts as unknown[])
      : Array.isArray(record(raw).splits)
        ? (record(raw).splits as unknown[])
        : []
  return rows.flatMap((row) => {
    const r = record(row)
    const account = str(r.account)
    if (!account) return []
    return [
      {
        account,
        weekPercent: num(r.week_percent) ?? 0,
        resetsAtMs: num(r.resets_at_ms),
        outsideMeasured: r.outside_measured !== false,
        segments: Array.isArray(r.segments)
          ? r.segments.flatMap((s) => {
              const parsed = parseSegment(s)
              return parsed ? [parsed] : []
            })
          : [],
      },
    ]
  })
}

/* ── feature detection (§4.3.6, D144) ──────────────────────────────── */

/**
 * - `full`: `harness::retry` and `tmux::accounts::credential`, everything;
 * - `update`: `harness::retry` and an older tmux ("Update tmux");
 * - `install`: `harness::retry` and no tmux ("Install tmux");
 * - `none`: no `harness::retry`, today's screens.
 * Short of `full`, the sign-in of today stays, and the dialog section, the
 * rows, the chip, the footer and the card are hidden.
 */
export type ClaudeAccountsGate = 'full' | 'update' | 'install' | 'none'

export function claudeAccountsGate(
  caps: Pick<
    ChatCapabilities,
    'harnessRetry' | 'tmuxAccounts' | 'tmuxCredential'
  >,
): ClaudeAccountsGate {
  if (!caps.harnessRetry) return 'none'
  if (caps.tmuxCredential) return 'full'
  if (caps.tmuxAccounts) return 'update'
  return 'install'
}

/** The worker the "Install tmux" offer adds. */
export const TMUX_WORKER = 'tmux'

/* ── errors ─────────────────────────────────────────────────────────── */

export interface AccountsError {
  code: string | null
  message: string
  /** `blocked`: the windows in the way. */
  blockedBy: string[]
}

function texts(err: unknown, depth = 0): string[] {
  if (depth > 3 || err == null) return []
  if (typeof err === 'string') return [err]
  if (err instanceof Error) return [err.message]
  if (typeof err !== 'object') return []
  const r = err as Record<string, unknown>
  return ['message', 'error', 'reason', 'detail'].flatMap((key) =>
    typeof r[key] === 'string'
      ? [r[key] as string]
      : r[key] && typeof r[key] === 'object'
        ? texts(r[key], depth + 1)
        : [],
  )
}

function jsonIn(text: string): Record<string, unknown> | null {
  for (
    let brace = text.indexOf('{');
    brace !== -1;
    brace = text.indexOf('{', brace + 1)
  ) {
    try {
      const parsed: unknown = JSON.parse(text.slice(brace))
      if (parsed && typeof parsed === 'object') {
        const r = parsed as Record<string, unknown>
        const inner = typeof r.message === 'string' ? jsonIn(r.message) : null
        return inner && typeof inner.code === 'string' ? inner : r
      }
    } catch {
      // not JSON from here
    }
  }
  return null
}

/**
 * The worker's JSON error (`{code, message, blocked_by?}` in the error's
 * message, the way tmux reports them), or the plain text.
 */
export function parseAccountsError(err: unknown): AccountsError {
  if (err && typeof err === 'object' && !(err instanceof Error)) {
    const r = err as Record<string, unknown>
    if (typeof r.code === 'string' && typeof r.message === 'string') {
      return {
        code: r.code,
        message: r.message,
        blockedBy: strings(r.blocked_by),
      }
    }
  }
  for (const text of texts(err)) {
    const parsed = jsonIn(text)
    if (parsed && typeof parsed.code === 'string') {
      return {
        code: parsed.code,
        message: str(parsed.message) ?? errText(err),
        blockedBy: strings(parsed.blocked_by),
      }
    }
  }
  return { code: null, message: errText(err), blockedBy: [] }
}

/* ── calls ──────────────────────────────────────────────────────────── */

async function client(provided?: IiiClient): Promise<IiiClient> {
  return provided ?? (await getIiiClient())
}

export async function listAccounts(
  provided?: IiiClient,
): Promise<ClaudeSnapshot> {
  const c = await client(provided)
  return parseClaudeSnapshot(await c.call(ACCOUNTS_LIST_FUNCTION_ID, {}))
}

/** `refresh`: read every signed-in account again now (at most once a minute each). */
export async function readUsage(
  refresh = false,
  provided?: IiiClient,
): Promise<UsageSnapshot | null> {
  const c = await client(provided)
  return parseUsageSnapshot(
    await c.call(USAGE_READ_FUNCTION_ID, refresh ? { refresh: true } : {}),
  )
}

export interface GlobalResult {
  global: string
  state: 'switched' | 'waiting'
  following: number
  restarted: string[]
  afterTurn: string[]
  waitingFor: string[]
}

/** Make `account` the global one (like /login). Rejects with the worker's JSON error. */
export async function setGlobalAccount(account: string): Promise<GlobalResult> {
  const c = await client()
  const r = record(await c.call(ACCOUNTS_GLOBAL_FUNCTION_ID, { account }))
  return {
    global: str(r.global) ?? account,
    state: r.state === 'waiting' ? 'waiting' : 'switched',
    following: num(r.following) ?? 0,
    restarted: strings(r.restarted),
    afterTurn: strings(r.after_turn),
    waitingFor: strings(r.waiting_for),
  }
}

export async function addAccount(input: {
  name: string
  color: string
}): Promise<ClaudeAccount> {
  const c = await client()
  const r = record(await c.call(ACCOUNTS_ADD_FUNCTION_ID, { ...input }))
  const account = parseAccount(r.account)
  if (!account) throw new Error('tmux answered without an account')
  return account
}

export async function updateAccount(input: {
  account: string
  name?: string
  color?: string
}): Promise<ClaudeAccount | null> {
  const c = await client()
  const r = record(await c.call(ACCOUNTS_UPDATE_FUNCTION_ID, { ...input }))
  return parseAccount(r.account)
}

export async function removeAccount(
  account: string,
  deleteFiles = false,
): Promise<void> {
  const c = await client()
  await c.call(ACCOUNTS_REMOVE_FUNCTION_ID, {
    account,
    ...(deleteFiles ? { delete_files: true } : {}),
  })
}

export async function dismissSuggestion(id: string): Promise<void> {
  const c = await client()
  await c.call(ACCOUNTS_DISMISS_FUNCTION_ID, { suggestion: id })
}

export interface LoginStart {
  loginId: string
  account: string
  url: string
  expiresAtMs: number
  email: string | null
}

/** Start signing `account` in; the page it opens shows the code to paste. */
export async function startLogin(account: string): Promise<LoginStart> {
  const c = await client()
  const r = record(
    await c.call(LOGIN_START_FUNCTION_ID, { account, browser: 'none' }),
  )
  return {
    loginId: str(r.login_id) ?? '',
    account: str(r.account) ?? account,
    url: str(r.url) ?? '',
    expiresAtMs: num(r.expires_at_ms) ?? 0,
    email: str(r.email),
  }
}

/** Pass the page's code; resolves with the account signed in, rejects with `{code, message}`. */
export async function submitLoginCode(
  loginId: string,
  code: string,
): Promise<ClaudeAccount | null> {
  const c = await client()
  const r = record(
    await c.call(
      LOGIN_CODE_FUNCTION_ID,
      { login_id: loginId, code: code.replace(/\s+/g, '') },
      { timeoutMs: 90_000 },
    ),
  )
  return parseAccount(r.account)
}

export async function cancelLogin(loginId: string): Promise<void> {
  const c = await client()
  await c.call(LOGIN_CANCEL_FUNCTION_ID, { login_id: loginId })
}

/** Re-run the session's failed last turn on a newly resolved account. */
export async function retryTurn(
  sessionId: string,
): Promise<{ turnId: string | null }> {
  const c = await client()
  const r = record(await c.call(RETRY_FUNCTION_ID, { session_id: sessionId }))
  return { turnId: str(r.turn_id) }
}

export async function readAccountSplits(
  provided?: IiiClient,
): Promise<AccountSplit[]> {
  const c = await client(provided)
  return parseAccountSplits(await c.call(ACCOUNT_SPLIT_FUNCTION_ID, {}))
}

/**
 * A conversation's own account (§4.3.5): an id pins it, `"global"` follows
 * the global, null goes back to its project's.
 */
export async function setConversationAccount(
  sessionId: string,
  account: string | null,
): Promise<void> {
  await patchSessionMeta(
    account === null
      ? { session_id: sessionId, unset: ['claude_account'] }
      : { session_id: sessionId, set: { claude_account: account } },
  )
}

/* ── the store ──────────────────────────────────────────────────────── */

export interface ClaudeAccountsState {
  /** The first reading has answered. */
  loaded: boolean
  usage: UsageSnapshot | null
  error: string | null
  /** A `refresh: true` read is on its way. */
  refreshing: boolean
  splits: AccountSplit[] | null
  splitsError: string | null
}

const EMPTY: ClaudeAccountsState = {
  loaded: false,
  usage: null,
  error: null,
  refreshing: false,
  splits: null,
  splitsError: null,
}

const WATCH_FN = 'ade::claude-accounts-watch'
/** The split is read again on a newer reading at most this often. */
const SPLIT_MIN_INTERVAL_MS = 60_000

let state: ClaudeAccountsState = EMPTY
let active = false
let seeded = false
let stopWatch: (() => void) | null = null
let usageSeq = 0
let splitSeq = 0
let splitReadAt = 0
let splitWanted = 0
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of listeners) listener()
}

function update(patch: Partial<ClaudeAccountsState>): void {
  state = { ...state, ...patch }
  emit()
}

/** Keep `next` unless the store already holds a newer reading. */
export function applyUsageSnapshot(next: UsageSnapshot | null): void {
  if (!next) return
  if (state.usage && next.generatedAtMs < state.usage.generatedAtMs) return
  update({ loaded: true, usage: next, error: null })
  if (splitWanted > 0 && Date.now() - splitReadAt >= SPLIT_MIN_INTERVAL_MS) {
    void refreshAccountSplits()
  }
}

/** Read the usage now; `refresh` asks tmux to read every account again. */
export async function refreshClaudeAccounts(refresh = false): Promise<void> {
  if (!active || seeded) return
  const mine = ++usageSeq
  if (refresh) update({ refreshing: true })
  try {
    const snapshot = await readUsage(refresh)
    if (mine === usageSeq || refresh) applyUsageSnapshot(snapshot)
    if (refresh) void refreshAccountSplits()
  } catch (err) {
    if (mine === usageSeq) update({ loaded: true, error: errText(err) })
  } finally {
    if (refresh && mine === usageSeq) update({ refreshing: false })
  }
}

export async function refreshAccountSplits(): Promise<void> {
  if (!active || seeded) return
  const mine = ++splitSeq
  splitReadAt = Date.now()
  try {
    const splits = await readAccountSplits()
    if (mine === splitSeq) update({ splits, splitsError: null })
  } catch (err) {
    if (mine === splitSeq) update({ splitsError: errText(err) })
  }
}

function start(): void {
  if (active || seeded) return
  active = true
  void refreshClaudeAccounts()
  let cancelled = false
  let off: (() => void)[] = []
  stopWatch = () => {
    cancelled = true
    for (const dispose of off) dispose()
    off = []
  }
  void (async () => {
    try {
      const c = await getIiiClient()
      if (cancelled) return
      const localFnId = `${WATCH_FN}::${Math.random().toString(36).slice(2, 10)}`
      off.push(
        c.on(localFnId, (overview: unknown) => {
          applyUsageSnapshot(parseUsageSnapshot(record(overview).usage))
        }),
      )
      off.push(
        c.observe(TMUX_CHANGED_FUNCTION_ID, `${localFnId}::${c.browserId}`),
      )
    } catch {
      // The reading still answered (or will on the next start).
    }
  })()
}

function stop(): void {
  if (!active) return
  active = false
  stopWatch?.()
  stopWatch = null
  usageSeq++
  splitSeq++
  splitReadAt = 0
  state = EMPTY
  emit()
}

export function getClaudeAccountsState(): ClaudeAccountsState {
  return state
}

export function subscribeClaudeAccounts(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export interface ClaudeAccounts extends ClaudeAccountsState {
  gate: ClaudeAccountsGate
  /** The capabilities have been read. */
  capsLoaded: boolean
  /** The tmux snapshot's accounts, global and the rest; null before a reading. */
  claude: ClaudeSnapshot | null
  account: (id: string | null | undefined) => ClaudeAccount | null
  nameOf: (id: string) => string
  split: (id: string) => AccountSplit | null
}

/**
 * The accounts, when the engine has the whole feature. `enabled` is false on
 * mock and Storybook backends. `splits` asks for the week's split too (the
 * provider screen and the picker's cards).
 */
export function useClaudeAccounts(
  enabled = true,
  { splits = false }: { splits?: boolean } = {},
): ClaudeAccounts {
  const caps = useChatCapabilities(enabled)
  const gate = seeded && seededGate ? seededGate : claudeAccountsGate(caps)
  useEffect(() => {
    if (!enabled || seeded) return
    if (gate === 'full') start()
    else if (caps.loaded) stop()
  }, [enabled, gate, caps.loaded])
  useEffect(() => {
    if (!splits || !enabled || gate !== 'full') return
    splitWanted++
    if (
      !seeded &&
      (state.splits === null ||
        Date.now() - splitReadAt >= SPLIT_MIN_INTERVAL_MS)
    ) {
      void refreshAccountSplits()
    }
    return () => {
      splitWanted--
    }
  }, [splits, enabled, gate])
  const current = useSyncExternalStore(
    subscribeClaudeAccounts,
    getClaudeAccountsState,
    getClaudeAccountsState,
  )
  return useMemo(() => {
    const claude = current.usage?.claude ?? null
    const byId = new Map(claude?.accounts.map((a) => [a.id, a]) ?? [])
    const splitById = new Map(current.splits?.map((s) => [s.account, s]) ?? [])
    return {
      ...current,
      gate,
      capsLoaded: seeded || caps.loaded,
      claude,
      account: (id) => (id ? (byId.get(id) ?? null) : null),
      nameOf: (id) => byId.get(id)?.name ?? id,
      split: (id) => splitById.get(id) ?? null,
    }
  }, [current, gate, caps.loaded])
}

let seededGate: ClaudeAccountsGate | null = null

/**
 * Tests and stories: put a reading (and splits) in the store and pin the
 * gate, or empty it with null.
 */
export function seedClaudeAccountsForTests(
  seed: {
    usage?: UsageSnapshot | null
    splits?: AccountSplit[] | null
    gate?: ClaudeAccountsGate
  } | null,
): void {
  if (seed === null) {
    seeded = false
    seededGate = null
    state = EMPTY
    emit()
    return
  }
  seeded = true
  seededGate = seed.gate ?? 'full'
  state = {
    ...EMPTY,
    loaded: true,
    usage: seed.usage ?? null,
    splits: seed.splits ?? null,
  }
  emit()
}
