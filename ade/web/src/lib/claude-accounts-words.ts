/**
 * The words of Claude Code accounts in the Chat (chat-projects §4.3), pure:
 * what the provider screen, the account cards, the usage footer, the project
 * dialog, Conversation details and the limit card say. The views only draw
 * them.
 *
 * Ported from the apps (`AccountsWords`, `UsageWords`, `AccountLook` in
 * `android/…/ui/tmux/`), with the same English; the Chat's own lines (the
 * split, the limit card, the resolution) are the spec's. Times follow the
 * browser: its zone, and 24-hour or 12-hour as its locale writes them.
 */
import type {
  AccountSplit,
  ClaudeAccount,
  ClaudeSnapshot,
  SplitSegment,
  UsageLimit,
} from '@/lib/backend/claude-accounts'
import type { ProjectAccount } from '@/lib/backend/projects'

/* ── numbers and levels ─────────────────────────────────────────────── */

/** How full a window is: the default ink under 80 %, amber up to 99 %, red at 100 %. */
export type UsageLevel = 'normal' | 'high' | 'full'

/** A percentage (0–100) as the screens show it, rounded. */
export function percent(value: number): number {
  if (!Number.isFinite(value)) return 0
  return Math.round(Math.min(100, Math.max(0, value)))
}

export function level(
  value: number | UsageLimit | null | undefined,
): UsageLevel {
  if (value == null) return 'normal'
  const rounded = percent(typeof value === 'number' ? value : value.usedPercent)
  if (rounded >= 100) return 'full'
  if (rounded >= 80) return 'high'
  return 'normal'
}

/** "1 window", "3 windows". */
function plural(n: number, word: string): string {
  return n === 1 ? `1 ${word}` : `${n} ${word}s`
}

/* ── times ──────────────────────────────────────────────────────────── */

/** How the browser writes times: its zone (tests pass one), 24-hour or not. */
export interface TimeStyle {
  now: number
  timeZone?: string
  hour24?: boolean
  locale?: string
}

/** The browser's way: 24-hour unless its locale writes "2:00 PM". */
export function browserHour24(locale?: string): boolean {
  try {
    const options = new Intl.DateTimeFormat(locale, {
      hour: 'numeric',
    }).resolvedOptions()
    return options.hour12 !== true
  } catch {
    return true
  }
}

function parts(ms: number, style: TimeStyle) {
  const format = new Intl.DateTimeFormat(style.locale ?? 'en-US', {
    timeZone: style.timeZone,
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
    hourCycle: 'h23',
  })
  const out: Record<string, string> = {}
  for (const part of format.formatToParts(new Date(ms))) {
    out[part.type] = part.value
  }
  return out
}

/** Calendar days from `now` to `at` in the style's zone (0 today, 1 tomorrow). */
function daysBetween(at: number, style: TimeStyle): number {
  const a = parts(at, style)
  const b = parts(style.now, style)
  const day = (p: Record<string, string>) =>
    Date.UTC(Number(p.year), Number(p.month) - 1, Number(p.day))
  return Math.round((day(a) - day(b)) / 86_400_000)
}

/** "14:00", or "2:00 PM" where the browser writes 12-hour times. */
export function clock(ms: number, style: TimeStyle): string {
  const hour24 = style.hour24 ?? browserHour24(style.locale)
  return new Intl.DateTimeFormat(style.locale ?? 'en-US', {
    timeZone: style.timeZone,
    hour: hour24 ? '2-digit' : 'numeric',
    minute: '2-digit',
    hourCycle: hour24 ? 'h23' : 'h12',
  }).format(new Date(ms))
}

function weekday(ms: number, style: TimeStyle): string {
  return new Intl.DateTimeFormat(style.locale ?? 'en-US', {
    timeZone: style.timeZone,
    weekday: 'short',
  }).format(new Date(ms))
}

function monthDay(ms: number, style: TimeStyle): string {
  return new Intl.DateTimeFormat(style.locale ?? 'en-US', {
    timeZone: style.timeZone,
    month: 'short',
    day: 'numeric',
  }).format(new Date(ms))
}

/**
 * When a window resets, short (the footer): "today 16:40", "tomorrow 09:00",
 * "Sat 14:00" within the week, "Nov 22" past it.
 */
export function resetShort(resetsAtMs: number, style: TimeStyle): string {
  const days = daysBetween(resetsAtMs, style)
  const time = clock(resetsAtMs, style)
  if (days <= 0) return `today ${time}`
  if (days === 1) return `tomorrow ${time}`
  if (days < 7) return `${weekday(resetsAtMs, style)} ${time}`
  return monthDay(resetsAtMs, style)
}

/**
 * When a window resets, as the accounts list says it: "16:40" today,
 * "tomorrow 08:00", "Thu 09:00", "Nov 22".
 */
export function resetCompact(resetsAtMs: number, style: TimeStyle): string {
  const days = daysBetween(resetsAtMs, style)
  const time = clock(resetsAtMs, style)
  if (days <= 0) return time
  if (days === 1) return `tomorrow ${time}`
  if (days < 7) return `${weekday(resetsAtMs, style)} ${time}`
  return monthDay(resetsAtMs, style)
}

/** A window that reset after the worker's last reading: it reads 0 % until the next one. */
export const RESET_SINCE_READ = 'Reset since the last reading'

/** "Resets Sat 14:00", "Resets today 16:40", [RESET_SINCE_READ]; null without a time. */
export function resets(
  limit: UsageLimit | null | undefined,
  style: TimeStyle,
): string | null {
  if (limit?.resetSinceRead) return RESET_SINCE_READ
  if (limit?.resetsAtMs == null) return null
  return `Resets ${resetShort(limit.resetsAtMs, style)}`
}

/**
 * Under a meter of the accounts list: "Resets 16:40", "Limit reached · 18:10"
 * at 100 %, [RESET_SINCE_READ], "Not started" for a window nobody opened yet.
 */
export function meterNote(
  limit: UsageLimit | null | undefined,
  style: TimeStyle,
): string {
  if (limit?.resetSinceRead) return RESET_SINCE_READ
  if (limit?.resetsAtMs == null) return 'Not started'
  const time = resetCompact(limit.resetsAtMs, style)
  return level(limit) === 'full' ? `Limit reached · ${time}` : `Resets ${time}`
}

/** "2 min ago" for a reading taken at `atMs`: "just now" under a minute, then minutes, hours, days. */
export function ago(atMs: number, nowMs: number): string {
  const s = Math.max(0, Math.floor((nowMs - atMs) / 1000))
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)} min ago`
  if (s < 86_400) return `${Math.floor(s / 3600)} h ago`
  return `${plural(Math.floor(s / 86_400), 'day')} ago`
}

/* ── accounts ───────────────────────────────────────────────────────── */

/**
 * Claude Code's plan as people say it: "Pro", "Team", "Enterprise", "Max 5×",
 * "Max 20×" (the tier names the multiple: `default_claude_max_20x`); anything
 * else as the worker sent it, capitalised.
 */
export function claudePlan(
  plan: string | null | undefined,
  tier?: string | null,
): string | null {
  const p = plan?.trim().toLowerCase()
  if (!p) return null
  switch (p) {
    case 'pro':
      return 'Pro'
    case 'team':
      return 'Team'
    case 'enterprise':
      return 'Enterprise'
    case 'free':
      return 'Free'
    case 'max': {
      const match = /(\d+)x/i.exec(tier ?? '')
      return match ? `Max ${match[1]}×` : 'Max'
    }
    default:
      return p.charAt(0).toUpperCase() + p.slice(1)
  }
}

/** Why there is no reading, in a few words ("Usage unavailable" + the worker's reason). */
export function usageError(error: string | null | undefined): string {
  switch (error) {
    case null:
    case undefined:
      return 'Usage unavailable'
    case 'rate_limited':
      return 'Usage unavailable: asked too often, trying again later'
    case 'unauthorized':
      return 'Usage unavailable: sign in again'
    case 'network':
      return 'Usage unavailable: no network'
    default:
      return `Usage unavailable: ${error.replace(/_/g, ' ')}`
  }
}

/** "you@company.com · Team" under an account's name. */
export function emailAndPlan(a: ClaudeAccount): string | null {
  const text = [a.email, claudePlan(a.plan, a.tier)]
    .filter((part): part is string => Boolean(part))
    .join(' · ')
  return text || null
}

/** Why an account has no meters to show: not signed in, signed out on the host, or the reading's error. */
export function noReading(a: ClaudeAccount): string | null {
  if (a.usage?.week || a.usage?.fiveHour) return null
  if (!a.signedIn) return 'Not signed in'
  if (a.tokenExpired) return 'Signed out on this host'
  return usageError(a.usage?.error)
}

/** An account's week, rounded; null without a reading. */
export function weekPercent(
  a: ClaudeAccount | null | undefined,
): number | null {
  const week = a?.usage?.week
  return week ? percent(week.usedPercent) : null
}

/** An account's 5 hours, rounded; null without a reading. */
export function fiveHourPercent(
  a: ClaudeAccount | null | undefined,
): number | null {
  const five = a?.usage?.fiveHour
  return five ? percent(five.usedPercent) : null
}

/** An account that can run a turn now: signed in with a fresh token. */
export function usable(a: ClaudeAccount): boolean {
  return a.signedIn && !a.tokenExpired
}

/** An account at a limit now: its 5-hour window or its week is full. */
export function atLimit(a: ClaudeAccount): boolean {
  return level(a.usage?.fiveHour) === 'full' || level(a.usage?.week) === 'full'
}

/**
 * The account with the most room this week (D14's tiebreak): signed in with a
 * fresh token and a reading, neither its 5-hour window nor its week full, the
 * lowest week; `except` left out. Null when none qualifies.
 */
export function mostRoom(
  c: Pick<ClaudeSnapshot, 'accounts'> | null | undefined,
  except?: string | null,
): ClaudeAccount | null {
  let best: ClaudeAccount | null = null
  for (const a of c?.accounts ?? []) {
    if (a.id === except || !usable(a) || !a.usage?.week) continue
    if (atLimit(a)) continue
    if (!best) {
      best = a
      continue
    }
    const ours = a.usage.week.usedPercent
    const theirs = best.usage?.week?.usedPercent ?? 0
    if (
      ours < theirs ||
      (ours === theirs &&
        (a.usage.fiveHour?.usedPercent ?? 0) <
          (best.usage?.fiveHour?.usedPercent ?? 0))
    ) {
      best = a
    }
  }
  return best
}

/** The longest name the form takes (the worker's id is its slug, at most 32). */
export const MAX_NAME = 40

/** The id the worker makes of a name: lower case, `[a-z0-9-]`, at most 32. */
export function slug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]/g, '-')
    .split('-')
    .filter(Boolean)
    .join('-')
    .slice(0, 32)
    .replace(/-+$/, '')
}

/** Why a new account (or a rename of `self`) can't take `name`; null when it can. */
export function nameProblem(
  name: string,
  accounts: readonly Pick<ClaudeAccount, 'id' | 'name'>[],
  self?: string,
): string | null {
  const n = name.trim()
  if (!n) return null
  if ([...n].length > MAX_NAME) return `At most ${MAX_NAME} characters.`
  // biome-ignore lint/suspicious/noControlCharactersInRegex: the check is for control characters
  if (/[\u0000-\u001f\u007f-\u009f]/.test(n)) return 'No control characters.'
  if (!slug(n)) return 'Use a letter or a digit.'
  if (
    accounts.some(
      (a) => a.id !== self && a.name.toLowerCase() === n.toLowerCase(),
    )
  ) {
    return 'Another account has this name.'
  }
  return null
}

/** The tokens the worker stores, in the Add account form's order. */
export const ACCOUNT_COLORS = [
  'blue',
  'green',
  'violet',
  'orange',
  'pink',
] as const
export type AccountColor = (typeof ACCOUNT_COLORS)[number]

/** The colour picker's names. */
export function colorName(token: string): string {
  switch (token) {
    case 'blue':
      return 'Blue'
    case 'green':
      return 'Green'
    case 'violet':
      return 'Violet'
    case 'orange':
      return 'Orange'
    case 'pink':
      return 'Pink'
    default:
      return 'Gray'
  }
}

/** The first colour no account has yet, else round the five. */
export function nextColor(
  accounts: readonly Pick<ClaudeAccount, 'color'>[],
): AccountColor {
  const used = new Set(accounts.map((a) => a.color))
  return (
    ACCOUNT_COLORS.find((token) => !used.has(token)) ??
    ACCOUNT_COLORS[accounts.length % ACCOUNT_COLORS.length]
  )
}

/** An account menu's sign-in: "Sign in" before its first, "Sign in again" after. */
export function signInLabel(a: ClaudeAccount): string {
  return a.signedIn || a.tokenExpired ? 'Sign in again' : 'Sign in'
}

export const RENAME = 'Rename…'
export const COLOR = 'Color…'
export const REMOVE = 'Remove…'
/** Why the built-in account's Remove… is dimmed: the worker keeps `default`, ~/.claude itself. */
export const BUILT_IN_STAYS = "The built-in account can't be removed."

/** Remove… on the global account. */
export function removeGlobalFirst(name: string): string {
  return `Make another account global before removing ${name}.`
}

/** Remove's confirm: what stays, or, with "Also delete its folder", what goes for good. */
export function removeDescription(deleteFiles: boolean): string {
  return deleteFiles
    ? "Its folder goes for good: its sign-in and the conversations it doesn't share."
    : "Its windows keep running. New windows can't start on it."
}

/** Remove's button: "Remove", or "Remove and delete" with the folder. */
export function removeButton(deleteFiles: boolean): string {
  return deleteFiles ? 'Remove and delete' : 'Remove'
}

/** The Add account form (AddAccount). */
export const ADD_TITLE = 'Add a Claude Code account'
export const NAME_LABEL = 'Name'
export const NAME_PLACEHOLDER = 'Side project'
export const COLOR_LABEL = 'Color'
export const ADD_EXPLAIN = 'Each account keeps its own sign-in.'
export const CONTINUE_TO_SIGN_IN = 'Continue to sign-in'
export const RENAME_TITLE = 'Rename account'
export const RENAME_BUTTON = 'Rename'
export const DELETE_FOLDER = 'Also delete its folder'

/** "Color of Personal". */
export function colorTitle(name: string): string {
  return `Color of ${name}`
}

/** "Remove Personal?". */
export function removeTitle(name: string): string {
  return `Remove ${name}?`
}

/** The radio on an account that isn't signed in: sign in first (D20). */
export function signInFirstTitle(name: string): string {
  return `Sign in to ${name} first`
}

export function signInFirstBody(name: string): string {
  return `${name} isn't signed in on this host, so new windows would start at a sign-in prompt. Sign in, then make it global.`
}

export const SWITCH_FAILED = 'Could not switch the global account'

/**
 * A switch the worker refused (G12), in the apps' words: `blocked`, `busy`,
 * `owner_check_on`, `signing_in` (the worker's sentence says whose), else
 * "Could not switch the global account: …".
 */
export function globalRefusal(
  error: { code: string | null; message: string; blockedBy: readonly string[] },
  name: string,
): string {
  switch (error.code) {
    case 'blocked':
      return blocked(name, error.blockedBy)
    case 'busy':
      return BUSY
    case 'owner_check_on':
      return ownerCheckOn(name)
    case 'signing_in':
      return error.message.trim() || signingIn(name)
    default:
      return error.message.trim()
        ? `${SWITCH_FAILED}: ${error.message.trim()}`
        : SWITCH_FAILED
  }
}

/* ── switching the global account ───────────────────────────────────── */

/** `signing_in` without the worker's sentence. */
export function signingIn(name: string): string {
  return `A sign-in is under way for ${name} or the global account; finish or cancel it, then switch.`
}

/** `busy`: Claude Code holds its sign-in's locks. */
export const BUSY =
  'Claude Code is renewing its sign-in; try again in a moment.'

/** `owner_check_on`: this Claude Code version would stop the sessions a swap changes the account of. */
export function ownerCheckOn(name: string): string {
  return `This Claude Code version ties running sessions to their account, so switching would stop them. Restart them on ${name} instead.`
}

/** A second switch while one is on its way. */
export const STILL_SWITCHING = 'Still switching the global account.'

/** `blocked`: a hand-started Claude Code on `name`'s folder that can't be stopped. */
export function blocked(name: string, windows: readonly string[]): string {
  const which =
    windows.length === 0
      ? ''
      : windows.length === 1
        ? ` (window ${windows[0]})`
        : ` (windows ${windows.join(', ')})`
  const close =
    windows.length > 1
      ? 'Close or move them, then try again.'
      : 'Close it or move it, then try again.'
  return `${name} is in use by a Claude Code you started yourself${which}. ${close}`
}

/** "3 sessions follow the global account."; "" for none. */
export function following(n: number): string {
  if (n <= 0) return ''
  if (n === 1) return '1 session follows the global account.'
  return `${n} sessions follow the global account.`
}

/** "1 Claude Code session switches with its next message.", "3 … switch within 30 seconds." */
export function sessionsSwitch(n: number, delayS = 0): string {
  const one = n === 1
  const what = one
    ? '1 Claude Code session switches'
    : `${n} Claude Code sessions switch`
  if (delayS > 0) return `${what} within ${delayS} seconds.`
  return `${what} ${one ? 'with its next message.' : 'with their next message.'}`
}

/** The pinned windows a switch restarts; null for none. */
export function pinnedRestart(
  restarted: number,
  afterTurn: number,
): string | null {
  const now =
    restarted <= 0
      ? null
      : restarted === 1
        ? '1 pinned window restarts.'
        : `${restarted} pinned windows restart.`
  const after =
    afterTurn <= 0
      ? null
      : now !== null
        ? afterTurn === 1
          ? '1 restarts when its turn ends.'
          : `${afterTurn} restart when their turn ends.`
        : afterTurn === 1
          ? '1 pinned window restarts when its turn ends.'
          : `${afterTurn} pinned windows restart when their turn ends.`
  const text = [now, after].filter(Boolean).join(' ')
  return text || null
}

/**
 * The line after a switch: "Personal is now global. 3 Claude Code sessions
 * switch with their next message."; no follower: "New Claude Code sessions
 * start on Personal."; then the pinned windows.
 */
export function globalToast(
  name: string,
  followers: number,
  restarted: number,
  afterTurn: number,
  delayS = 0,
): string {
  const first =
    followers <= 0
      ? `New Claude Code sessions start on ${name}.`
      : `${name} is now global. ${sessionsSwitch(followers, delayS)}`
  return [first, pinnedRestart(restarted, afterTurn)].filter(Boolean).join(' ')
}

/** The banner while a switch waits for the windows pinned to its account. */
export function waitingBanner(toName: string, waitingFor: number): string {
  if (waitingFor === 1) {
    return `Switching to ${toName} when 1 window pinned to it finishes its turn.`
  }
  if (waitingFor > 1) {
    return `Switching to ${toName} when ${waitingFor} windows pinned to it finish their turn.`
  }
  return `Switching to ${toName} when the windows pinned to it finish their turn.`
}

/* ── the suggestion card ───────────────────────────────────────────── */

/** "Work, the global account, is at 91%"; at a limit, which one. */
export function suggestionTitle(
  from: ClaudeAccount | null | undefined,
  fromId: string,
  reason: string,
): string {
  const name = from?.name ?? fromId
  if (reason === 'five_hour') {
    return `${name}, the global account, reached its 5-hour limit`
  }
  if (reason === 'limit') {
    return `${name}, the global account, reached its weekly limit`
  }
  const pct = weekPercent(from) ?? (reason === 'week_80' ? 80 : 90)
  return `${name}, the global account, is at ${pct}%`
}

/** "Personal has the most room this week: 12%. 2 sessions follow the global account." */
export function suggestionBody(
  to: ClaudeAccount | null | undefined,
  toId: string,
  isMostRoom: boolean,
  followers: number,
  wide: boolean,
): string {
  const name = to?.name ?? toId
  const pct = weekPercent(to)
  const room =
    pct === null
      ? `${name} has room.`
      : isMostRoom
        ? `${name} has the most room${wide ? ' this week' : ''}: ${pct}%.`
        : `${name} has room: ${pct}%.`
  const more = following(followers)
  return more ? `${room} ${more}` : room
}

/** The suggestion's button: "Make Personal global" (wide), or "Sign in to Personal" when it isn't signed in. */
export function suggestionUse(name: string, signIn: boolean): string {
  return signIn ? `Sign in to ${name}` : `Make ${name} global`
}

export const NOT_NOW = 'Not now'

/* ── the provider screen (§4.3.1) ──────────────────────────────────── */

export const SCREEN_TITLE = 'Claude Code accounts'
export const TAB_ACCOUNTS_WIDE = 'Accounts and usage'
export const TAB_ACCOUNTS_PHONE = 'Accounts'
export const TAB_PROJECTS = 'Projects'
export const INTRO_WIDE =
  "The same accounts as tmux. The global account is the sign-in of ~/.claude: every session that isn't in a pinned project uses it and switches with its next message. A project can be pinned to an account."
export const INTRO_PHONE =
  'The same accounts as tmux. A project follows the global account or is pinned to one.'
export const ADD_ACCOUNT = 'Add account'
export const OPEN_IN_SETTINGS = 'Open in Settings'
export const NOT_SIGNED_IN = 'Not signed in'
export const SIGN_IN = 'Sign in'
export const GLOBAL_TAG = 'global'
export const FIVE_HOURS = '5 hours'
export const THIS_WEEK = 'This week'

/** "Read 4 min ago". */
export function readAgo(asOfMs: number, nowMs: number): string {
  return `Read ${ago(asOfMs, nowMs)}`
}

/** "Resets Sat 2:00 PM · Opus 41%": the card's reset line, the week's reset then Opus. */
export function cardResetLine(
  a: ClaudeAccount,
  style: TimeStyle,
): string | null {
  const week = a.usage?.week ?? null
  const head = week ? meterNote(week, style) : null
  const opus = a.usage?.weekOpus
  const extra = opus ? `Opus ${percent(opus.usedPercent)}%` : null
  const text = [head, extra].filter(Boolean).join(' · ')
  return text || null
}

/** The partial cases of §4.3.6. */
export const NEEDS_NEWER_TMUX = 'Accounts in the Chat need a newer tmux worker.'
export const UPDATE_TMUX = 'Update tmux'
export const NEEDS_TMUX =
  'Several Claude Code accounts need the tmux worker on this engine.'
export const INSTALL_TMUX = 'Install tmux'

/* ── the week's split (D28) ────────────────────────────────────────── */

export const SPLIT_TITLE = 'This week by project (from session tokens)'
export const OUTSIDE_LABEL = 'Outside the Chat'
export const OUTSIDE_HINT =
  'tmux windows and other Claude Code sessions on this account'
export const NO_CHAT_SPEND =
  'No project uses it. Its usage this week came from outside the Chat.'
export const OUTSIDE_NOT_MEASURED =
  "Usage outside the Chat isn't measured here."
export const NO_PROJECT_LABEL = 'no project'

/** The look of a segment: a project's hue, no project, or outside the Chat. */
export type SplitTone =
  | 'blue'
  | 'purple'
  | 'teal'
  | 'green'
  | 'amber'
  | 'rose'
  | 'no-project'
  | 'outside'

export interface SplitItem {
  key: string
  /** "acme-web 33%". */
  label: string
  /** The legend's name alone ("acme-web"), for the bar's accessible name. */
  name: string
  percent: number
  tone: SplitTone
  /** "Outside the Chat"'s hint. */
  hint?: string
}

export interface SplitView {
  items: SplitItem[]
  /** The line under the legend, or in place of the bar. */
  note: string | null
  /** No segment at all: the card shows only the note. */
  empty: boolean
}

const PROJECT_TONES = new Set<SplitTone>([
  'blue',
  'purple',
  'teal',
  'green',
  'amber',
  'rose',
])

function segmentItem(segment: SplitSegment, index: number): SplitItem | null {
  const pct = percent(segment.percent)
  if (segment.kind === 'project') {
    const tone = PROJECT_TONES.has(segment.color as SplitTone)
      ? (segment.color as SplitTone)
      : 'blue'
    return {
      key: `project:${segment.projectId || index}`,
      name: segment.name,
      label: `${segment.name} ${pct}%`,
      percent: segment.percent,
      tone,
    }
  }
  if (segment.kind === 'no_project') {
    return {
      key: 'no_project',
      name: NO_PROJECT_LABEL,
      label: `${NO_PROJECT_LABEL} ${pct}%`,
      percent: segment.percent,
      tone: 'no-project',
    }
  }
  return {
    key: 'outside',
    name: OUTSIDE_LABEL,
    label: `${OUTSIDE_LABEL} ${pct}%`,
    percent: segment.percent,
    tone: 'outside',
    hint: OUTSIDE_HINT,
  }
}

/**
 * What the card shows of an account's week (§4.3.1): the legend's items in
 * the engine's order, and the note:
 * - only "Outside the Chat" (weekly use, no Chat spend): an all-grey bar and
 *   "No project uses it. …";
 * - `outside_measured` false: no grey segment, and the legend ends with
 *   "Usage outside the Chat isn't measured here.";
 * - no use at all: nothing.
 */
export function splitView(split: AccountSplit | null | undefined): SplitView {
  if (!split) return { items: [], note: null, empty: true }
  const items = split.segments
    .filter((segment) => split.outsideMeasured || segment.kind !== 'outside')
    .map(segmentItem)
    .filter((item): item is SplitItem => item !== null && item.percent > 0)
  const chat = items.filter((item) => item.tone !== 'outside')
  if (items.length === 0) {
    return {
      items,
      note: split.outsideMeasured ? null : OUTSIDE_NOT_MEASURED,
      empty: true,
    }
  }
  if (chat.length === 0) {
    return { items, note: NO_CHAT_SPEND, empty: false }
  }
  return {
    items,
    note: split.outsideMeasured ? null : OUTSIDE_NOT_MEASURED,
    empty: false,
  }
}

/** The bar's accessible name: "This week by project: acme-web 33%, mobile 12%". */
export function splitSpoken(view: SplitView): string {
  if (view.items.length === 0) return SPLIT_TITLE
  return `This week by project: ${view.items.map((item) => item.label).join(', ')}`
}

/* ── the footer (§4.3.3) ───────────────────────────────────────────── */

export const FOOTER_TITLE = 'Usage this week'

/** "Usage this week · 4 min ago". */
export function footerTitle(asOfMs: number | null, nowMs: number): string {
  return asOfMs === null
    ? FOOTER_TITLE
    : `${FOOTER_TITLE} · ${ago(asOfMs, nowMs)}`
}

/** A mini meter's label: "Work · global", "Personal". */
export function footerName(a: ClaudeAccount, global: boolean): string {
  return global ? `${a.name} · ${GLOBAL_TAG}` : a.name
}

/** The oldest reading's time among the accounts that have one. */
export function oldestReading(
  accounts: readonly ClaudeAccount[],
): number | null {
  const times = accounts
    .map((a) => a.usage?.asOfMs)
    .filter((t): t is number => typeof t === 'number' && t > 0)
  return times.length ? Math.min(...times) : null
}

/* ── where a conversation's account comes from (§4.3.5) ─────────────── */

/** A project's account: following the global, or pinned to one. */
export type ProjectAccountChoice = ProjectAccount

export type AccountSource = 'session' | 'project' | 'global'

export interface AccountResolution {
  /** The account the next turn runs on; null when the global is unknown. */
  account: string | null
  source: AccountSource
  /** The project it comes from (pinned), or that follows the global. */
  projectName?: string
  /**
   * A conversation that runs on another's account: a side chat on its
   * lock's, a sub-agent on its parent's.
   */
  locked?: Exclude<LockedAccount, null>
}

/**
 * The harness's order (spec §4.2.4), for what the screens say before a turn
 * runs: `metadata.claude_account` (an id; `"global"` follows the global),
 * else the project's pinned account, else the global.
 */
export function resolveAccount(input: {
  claudeAccount?: unknown
  project?: { name: string; account?: ProjectAccountChoice | null } | null
  global: string | null
}): AccountResolution {
  const pin = input.claudeAccount
  if (typeof pin === 'string' && pin && pin !== 'global') {
    return { account: pin, source: 'session' }
  }
  if (pin === 'global') return { account: input.global, source: 'global' }
  const projectAccount = input.project?.account
  if (projectAccount?.mode === 'pinned' && projectAccount.account_id) {
    return {
      account: projectAccount.account_id,
      source: 'project',
      projectName: input.project?.name,
    }
  }
  return {
    account: input.global,
    source: 'global',
    ...(input.project ? { projectName: input.project.name } : {}),
  }
}

/**
 * Conversation details' account row:
 * - "Runs on Work, the global account, because acme-web follows the global."
 * - "Runs on Work, the global account." (no project, or the conversation's own choice)
 * - "Runs on Personal, pinned by billing-api."
 * - "Runs on Personal, pinned for this conversation."
 * - "Runs on Personal, locked to its conversation's account." (a side chat)
 * - "Runs on Personal, its conversation's account." (a sub-agent)
 */
export function resolutionText(
  resolution: AccountResolution,
  nameOf: (id: string) => string,
  explicitGlobal = false,
): string {
  const name = resolution.account
    ? nameOf(resolution.account)
    : 'the global account'
  if (resolution.locked && resolution.account) {
    return resolution.locked === 'side-chat'
      ? `Runs on ${name}, locked to its conversation's account.`
      : `Runs on ${name}, its conversation's account.`
  }
  if (resolution.source === 'session') {
    return `Runs on ${name}, pinned for this conversation.`
  }
  if (resolution.source === 'project') {
    return `Runs on ${name}, pinned by ${resolution.projectName ?? 'its project'}.`
  }
  if (!resolution.account) return 'Runs on the global account.'
  if (resolution.projectName && !explicitGlobal) {
    return `Runs on ${name}, the global account, because ${resolution.projectName} follows the global.`
  }
  return `Runs on ${name}, the global account.`
}

/** The select's first option: "Project's (Work, global)", "Project's (Personal)", or "Global (Work)" without a project. */
export function defaultChoiceLabel(input: {
  hasProject: boolean
  projectAccount?: ProjectAccountChoice | null
  globalName: string | null
  nameOf: (id: string) => string
}): string {
  const global = input.globalName ?? 'the global account'
  if (!input.hasProject) return `Global (${global})`
  if (
    input.projectAccount?.mode === 'pinned' &&
    input.projectAccount.account_id
  ) {
    return `Project's (${input.nameOf(input.projectAccount.account_id)})`
  }
  return `Project's (${global}, ${GLOBAL_TAG})`
}

/** "Global (Work)": following the global with a project pinned elsewhere. */
export function globalChoiceLabel(globalName: string | null): string {
  return `Global (${globalName ?? 'the global account'})`
}

export const ACCOUNT_ROW_LABEL = 'Claude Code account'

/* ── the project dialog (§4.3.2) ───────────────────────────────────── */

export const PROJECT_ACCOUNT_LABEL = 'Claude Code account'
export const FOLLOW_GLOBAL = 'Follow the global account'
export const PIN_TO = 'Pin to'
export const OTHER_PROVIDERS_IGNORE =
  'Other providers (Codex, API keys) ignore this.'

/** "Work now", beside "Follow the global account". */
export function globalNow(name: string): string {
  return `${name} now`
}

/** "Personal · 22% this week": the pin's control, and each menu row. */
export function accountWeek(a: ClaudeAccount): string {
  const pct = weekPercent(a)
  return pct === null ? a.name : `${a.name} · ${pct}% this week`
}

/* ── the Projects tab (§4.3.1) ─────────────────────────────────────── */

export const PROJECTS_HEADING = 'Accounts by project'
export const PROJECTS_HINT =
  "A new session takes its project's account. Moving a project moves its idle sessions at their next message."
export const COLUMN_PROJECT = 'Project'
export const COLUMN_FOLDERS = 'Folders'
export const COLUMN_ACCOUNT = 'Account'
export const COLUMN_SESSIONS = 'Sessions'
export const COLUMN_LOW = 'When it runs low (as in tmux)'
export const NO_PROJECT_ROW = 'No project'
export const ANY_OTHER_FOLDER = 'Any other folder'
export const ALWAYS_GLOBAL = 'Always the global'
export const SWITCHES_WITH_GLOBAL = 'Switches with the global'
export const STAYS_PINNED = 'Stays pinned; the limit card offers another'

/** "Follow global (Work)", "Personal (pinned)". */
export function projectAccountCell(
  choice: ProjectAccountChoice | null | undefined,
  globalName: string | null,
  nameOf: (id: string) => string,
): string {
  if (choice?.mode === 'pinned' && choice.account_id) {
    return `${nameOf(choice.account_id)} (pinned)`
  }
  return `Follow global (${globalName ?? 'the global account'})`
}

/**
 * "4 · 1 working", "6"; "6+" when the session list stopped at its page
 * (older sessions aren't counted).
 */
export function sessionsCell(
  total: number,
  working: number,
  more = false,
): string {
  const count = more ? `${total}+` : String(total)
  return working > 0 ? `${count} · ${working} working` : count
}

/**
 * The folders shortened: "~/work/acme/web, api, infra" (a folder beside the
 * first one by its name only).
 */
export function foldersCell(
  folders: readonly string[],
  tilde: (path: string) => string,
): string {
  const shown = folders.map((folder) =>
    tilde(folder.replace(/\/+$/, '') || '/'),
  )
  if (shown.length === 0) return ''
  const first = shown[0]
  const parent = first.slice(0, first.lastIndexOf('/') + 1)
  return shown
    .map((folder, index) => {
      if (index === 0 || !parent) return folder
      const rest = folder.startsWith(parent) ? folder.slice(parent.length) : ''
      return rest && !rest.includes('/') ? rest : folder
    })
    .join(', ')
}

/* ── the limit card (§4.3.4) ───────────────────────────────────────── */

export type LimitWindow = 'five_hour' | 'week'

/** Which of `a`'s windows is full, the 5-hour one first; null while neither is. */
export function limitHit(
  a: ClaudeAccount | null | undefined,
): { window: LimitWindow; limit: UsageLimit } | null {
  const usage = a?.usage
  if (!usage) return null
  if (usage.fiveHour && level(usage.fiveHour) === 'full') {
    return { window: 'five_hour', limit: usage.fiveHour }
  }
  if (usage.week && level(usage.week) === 'full') {
    return { window: 'week', limit: usage.week }
  }
  return null
}

/** "Personal reached its 5-hour limit" / "Personal reached its weekly limit". */
export function limitTitle(name: string, window: LimitWindow | null): string {
  if (window === 'five_hour') return `${name} reached its 5-hour limit`
  if (window === 'week') return `${name} reached its weekly limit`
  return `${name} reached its usage limit`
}

/** "Resets 4:40 PM" (today), "Resets tomorrow 09:00", "Resets Sat 2:00 PM". */
export function limitResetLine(
  resetsAtMs: number | null | undefined,
  style: TimeStyle,
): string | null {
  return resetsAtMs == null ? null : `Resets ${resetCompact(resetsAtMs, style)}`
}

/** A failed card whose window reset since: the line, then the primary button (harness::retry on the same account). */
export const LIMIT_HAS_RESET = 'The limit has reset'
export const TRY_AGAIN = 'Try again'

/** "Continue in Work". */
export function continueIn(name: string): string {
  return `Continue in ${name}`
}

export const OTHER = 'Other…'

/** "Make Work the global". */
export function makeGlobal(name: string): string {
  return `Make ${name} the global`
}

export const LIMIT_HINT =
  'Changing account starts a new prompt cache for this conversation.'

/**
 * A conversation that can't choose its Claude Code account: a side chat
 * keeps its conversation's (D2), a sub-agent runs on its conversation's.
 * Its limit card offers no other account, and its notices say why.
 */
export type LockedAccount = 'side-chat' | 'sub-agent' | null

/** A sidechat's hint: its account is locked (D2). */
export function sideChatLimitHint(name: string): string {
  return `This side chat stays on ${name} so it keeps the conversation's cache.`
}

/** A sub-agent's hint on its card, and its step in the usage-limit notice. */
export const SUB_AGENT_LIMIT_HINT =
  "This sub-agent stays on its conversation's account."

/** The locked card's hint: the side chat names its account. */
export function lockedLimitHint(
  locked: Exclude<LockedAccount, null>,
  name: string,
): string {
  return locked === 'side-chat' ? sideChatLimitHint(name) : SUB_AGENT_LIMIT_HINT
}

/** A side chat's step in the usage-limit notice (no account name there). */
export const SIDE_CHAT_LIMIT_STEP =
  "This side chat stays on its account so it keeps the conversation's cache."

/** The usage-limit notice's step where no other account can take over. */
export const LIMIT_WAIT_STEP = 'Wait until the limit resets, then try again.'

/** "Other…"'s rows: "5 hours 38% · This week 64%", or why there is none. */
export function accountUse(a: ClaudeAccount): string {
  const five = fiveHourPercent(a)
  const week = weekPercent(a)
  if (five === null && week === null) return noReading(a) ?? ''
  return [
    five === null ? null : `${FIVE_HOURS} ${five}%`,
    week === null ? null : `${THIS_WEEK} ${week}%`,
  ]
    .filter(Boolean)
    .join(' · ')
}

/** A pin to an account that no longer exists (`harness.unknown_account`). */
export function unknownAccountTitle(id: string): string {
  return `${id} is no longer a Claude Code account`
}
export const UNKNOWN_ACCOUNT_LINE = 'Choose another account to continue.'

/** A removed account where no other can be chosen: the card's line, the notice's step. */
export function lockedAccountRemoved(
  locked: Exclude<LockedAccount, null>,
): string {
  return locked === 'side-chat'
    ? "Its account was removed from tmux, and a side chat keeps its conversation's account. Continue in the main conversation."
    : 'Its account was removed from tmux. Continue in the main conversation.'
}

/** The close button's name. */
export const HIDE_LIMIT = 'Hide until the reading changes'
