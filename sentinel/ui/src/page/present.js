/**
 * The page's pure vocabulary: what a state is called, what it lets you do,
 * and how a row is drawn. Kept out of the components so the rules can be
 * read — and tested — without mounting anything.
 */

import { formatRelative } from '@iii-dev/console-ui/format'

/** @typedef {'new'|'investigating'|'diagnosed'|'resolved'|'regressed'|'ignored'} GroupStatus */

/** The states the list shows when nobody has narrowed it. @type {GroupStatus[]} */
export const OPEN_STATES = ['new', 'investigating', 'diagnosed', 'regressed']

/** @type {Record<GroupStatus, { label: string, tone: 'neutral'|'accent'|'success'|'warning'|'danger' }>} */
export const STATUS_PRESENTATION = {
  new: { label: 'new', tone: 'danger' },
  investigating: { label: 'investigating', tone: 'accent' },
  diagnosed: { label: 'diagnosed', tone: 'warning' },
  // A regression is the one piece of news in the list: somebody fixed this
  // and it came back.
  regressed: { label: 'regressed', tone: 'danger' },
  resolved: { label: 'resolved', tone: 'success' },
  ignored: { label: 'ignored', tone: 'neutral' },
}

/**
 * Which group actions are offered. Resolving and ignoring are human
 * decisions and only make sense on an open group; reopening only on a
 * closed one.
 * @param {GroupStatus} status
 */
export function availableActions(status) {
  switch (status) {
    case 'new':
    case 'diagnosed':
    case 'regressed':
      return ['investigate', 'resolve', 'ignore']
    case 'investigating':
      // A first pass is running: stopping it is the action, not relabelling
      // the group underneath it.
      return ['stop']
    case 'resolved':
      // The lifecycle refuses to investigate a closed group; reopening is
      // the way back to everything else.
      return ['reopen']
    case 'ignored':
      return ['unignore']
    default:
      return []
  }
}

/**
 * The window filter, as milliseconds before now. `null` means "all time".
 * @param {string} id
 * @param {number} now
 */
export function sinceMs(id, now) {
  const hours = { '1h': 1, '24h': 24, '7d': 24 * 7, '30d': 24 * 30 }[id]
  return hours === undefined ? null : now - hours * 3_600_000
}

/**
 * Bar heights for the 24-hour sparkline, as percentages of the busiest
 * hour. An hour with occurrences never draws as nothing: a one-pixel floor
 * is the difference between "quiet" and "none".
 * @param {number[]} counts
 */
export function sparklineBars(counts, hotFrom = counts.length) {
  const peak = Math.max(0, ...counts)
  return counts.map((count, index) => ({
    count,
    height: count === 0 ? 0 : Math.max(8, Math.round((count / peak) * 100)),
    hot: index >= hotFrom && count > 0,
  }))
}

/**
 * The first sparkline bar that belongs to a regression — the hours since
 * the fix stopped holding. The line has no hot bars for any other state.
 * @param {{ status: string, regressed_at_ms?: number }} group
 * @param {number} bars
 * @param {number} now
 */
export function hotFrom(group, bars, now) {
  if (group.status !== 'regressed' || !group.regressed_at_ms) return bars
  const hours = Math.floor(now / 3_600_000) - Math.floor(group.regressed_at_ms / 3_600_000)
  return Math.max(0, bars - 1 - hours)
}

/**
 * What the session button offers. The host never says whether the chat
 * column is open, so the answer is the same call either way: with the
 * session already in front of the user it is a quiet live marker, otherwise
 * it is an invitation to open it.
 * @param {string|undefined|null} sessionId
 * @param {string|undefined|null} conversationId
 */
export function sessionAffordance(sessionId, conversationId) {
  if (!sessionId) return null
  return sessionId === conversationId
    ? { variant: 'live', label: 'session open' }
    : { variant: 'open', label: 'Open session' }
}

/**
 * A `file:line` an editor can be opened at, or null when the diagnosis
 * pointed at something that is not a place in the code.
 * @param {{ kind: string, path?: string, line?: number }} evidence
 * @param {string|null|undefined} repositoryPath
 */
export function codeLocation(evidence, repositoryPath) {
  if (evidence.kind !== 'code' || !evidence.path || !repositoryPath) return null
  const path = evidence.path.replace(/^\.?\//, '')
  return { path: `${repositoryPath.replace(/\/$/, '')}/${path}`, line: evidence.line ?? 1 }
}

/**
 * The ignore rule in words, for the line under an ignored group.
 * @param {{ kind: string, count?: number }|undefined|null} rule
 */
export function ignoreSummary(rule) {
  if (!rule) return ''
  if (rule.kind === 'forever') return 'ignored'
  if (rule.kind === 'version_change') return 'ignored until the version changes'
  if (rule.kind === 'occurrences') {
    const count = rule.count ?? 0
    return `ignored for ${count} more occurrence${count === 1 ? '' : 's'}`
  }
  return 'ignored'
}

/**
 * One move, in words. The reason the worker recorded is more specific than
 * the pair of states, so it leads when there is one.
 * @param {{ from_status?: string, to_status: string, reason?: string, actor: string }} row
 */
export function transitionSentence(row) {
  if (!row.from_status) return 'first seen'
  const reason = {
    regression: 'came back after a fix',
    ignore_expired: 'the ignore ran out',
    resolved: 'marked resolved',
    ignored: 'ignored',
    diagnosed: 'a diagnosis was recorded',
    reopened: 'reopened',
    investigating: 'an investigation started',
  }[row.reason ?? '']
  if (reason) return reason
  // No reason recorded: the pair of states is still the truth.
  return `${row.from_status} → ${row.to_status}`
}

/**
 * What a mixed selection can be asked to do: only what every group in it
 * can. Offering Resolve over a selection that includes an ignored group
 * would promise something that refuses halfway through, and the person would
 * have to work out which half.
 * @param {GroupStatus[]} statuses
 */
export function bulkActions(statuses) {
  if (statuses.length === 0) return []
  const sets = statuses.map((status) => availableActions(status))
  return sets[0].filter(
    (action) =>
      // Investigating and stopping are one group at a time: each opens or
      // ends a conversation somebody is meant to watch.
      action !== 'investigate' && action !== 'stop' && sets.every((set) => set.includes(action)),
  )
}

/**
 * The outcome of applying one action across a selection, as a sentence.
 * @param {string} action
 * @param {number} done
 * @param {string[]} failures
 */
export function bulkOutcome(action, done, failures) {
  const verb = { resolve: 'resolved', ignore: 'ignored', reopen: 'reopened', unignore: 'unignored' }[action] ?? action
  if (failures.length === 0) return `${done} ${verb}.`
  const refused = failures.length === 1 ? '1 refused' : `${failures.length} refused`
  return `${done} ${verb}, ${refused}: ${failures[0]}`
}

/** The list's scopes, as the design names them. */
export const SCOPES = [
  { value: 'open', label: 'Open' },
  { value: 'regressed', label: 'Regressed' },
  { value: 'ignored', label: 'Ignored' },
  { value: 'resolved', label: 'Resolved' },
]

/** @type {Record<string, GroupStatus[]>} */
export const SCOPE_STATES = {
  open: OPEN_STATES,
  regressed: ['regressed'],
  ignored: ['ignored'],
  resolved: ['resolved'],
}

/** @param {GroupStatus[]} statuses */
export function scopeOf(statuses) {
  for (const [scope, states] of Object.entries(SCOPE_STATES)) {
    if (states.length === statuses.length && states.every((state) => statuses.includes(state))) {
      return scope
    }
  }
  return 'open'
}

export const WINDOWS = [
  { value: '24h', label: '24 h' },
  { value: '7d', label: '7 d' },
  { value: '30d', label: '30 d' },
  { value: 'all', label: 'All' },
]

/** `1 284`: thousands set apart with a space, the way the design counts. @param {number} count */
export function spaced(count) {
  return Math.round(count).toLocaleString('en-US').replace(/,/g, ' ')
}

/**
 * How long ago: the console's own relative time, read as a phrase.
 * @param {number} at_ms
 * @param {number} now
 */
export function ago(at_ms, now) {
  const relative = formatRelative(at_ms, now)
  return relative === 'just now' ? relative : `${relative} ago`
}

/** The absolute half of a fact, on the reader's own clock. @param {number} at_ms */
export function stamp(at_ms) {
  const at = new Date(at_ms)
  const two = (/** @type {number} */ value) => String(value).padStart(2, '0')
  return `${at.getFullYear()}-${two(at.getMonth() + 1)}-${two(at.getDate())} ${two(at.getHours())}:${two(at.getMinutes())}:${two(at.getSeconds())}`
}

/** @param {string|undefined} first @param {string|undefined} last */
export function versionRange(first, last) {
  if (!first && !last) return 'unknown version'
  if (!first || !last || first === last) return last ?? first ?? ''
  return `${first} → ${last}`
}

/**
 * The exception type, set apart from the message it prefixes, so a row can
 * lead with it in bold. A title without one stays whole.
 * @param {string} title
 * @param {string|undefined} exceptionType
 */
export function splitTitle(title, exceptionType) {
  const prefix = exceptionType ? `${exceptionType}: ` : ''
  if (prefix && title.startsWith(prefix)) return { type: exceptionType, rest: title.slice(prefix.length) }
  return { type: null, rest: title }
}

/**
 * How a state is drawn: the badge tone, the glyph beside the word, and the
 * dot that leads a row.
 * @param {GroupStatus} status
 */
export function statusLook(status) {
  switch (status) {
    case 'regressed':
      return { badge: 'alert', glyph: 'alert', dot: 'alert' }
    case 'investigating':
      return { badge: 'accent', glyph: 'live', dot: 'accent' }
    case 'diagnosed':
      return { badge: 'default', glyph: 'file', dot: 'ok' }
    case 'resolved':
      return { badge: 'ok', glyph: 'check', dot: 'ok' }
    case 'ignored':
      return { badge: 'default', glyph: 'ban', dot: 'ghost' }
    default:
      return { badge: 'default', glyph: null, dot: 'ghost' }
  }
}

/**
 * The line under a regressed group: what was resolved, and what brought it
 * back.
 * @param {{ service_name: string, last_version?: string, regressed_at_ms?: number, resolved_at_ms?: number, resolved_version?: string, resolve_until_version_change?: boolean }} group
 * @param {number} now
 */
export function regressedNote(group, now) {
  const back = group.regressed_at_ms ? ` ${ago(group.regressed_at_ms, now)}` : ''
  const on = group.last_version ? ` on ${group.last_version}` : ''
  if (!group.resolved_at_ms) return `It was resolved, and an occurrence${on} reopened it${back}.`
  const where = group.resolved_version ? ` in ${group.service_name} ${group.resolved_version}` : ''
  const scope = group.resolve_until_version_change ? ' with until version change' : ''
  const kept =
    group.resolve_until_version_change && group.resolved_version
      ? ` Occurrences on ${group.resolved_version} kept counting without reopening.`
      : ''
  return `Resolved ${ago(group.resolved_at_ms, now)}${where}${scope}; the first occurrence${on} reopened it${back}.${kept}`
}

/**
 * The line under an ignored group.
 * @param {{ service_name: string, last_version?: string, ignore_rule?: { kind: string, count?: number } }} group
 */
export function ignoreNote(group) {
  const rule = group.ignore_rule
  if (rule?.kind === 'version_change') {
    const baseline = group.last_version ? ` (baseline ${group.last_version})` : ''
    return `Ignored until the ${group.service_name} version changes${baseline}. Occurrences still count.`
  }
  if (rule?.kind === 'occurrences') {
    return `Ignored until ${rule.count ?? 0} more occurrences. Occurrences still count.`
  }
  return 'Ignored forever. Occurrences still count; nothing surfaces in the open list.'
}

/**
 * One row of the history: the state it moved to, in a word, and why.
 * @param {{ from_status?: GroupStatus, to_status: GroupStatus, reason?: string, actor: string }} row
 */
export function transitionLook(row) {
  if (!row.from_status) return { what: 'First seen', note: '', dot: 'ghost' }
  const what =
    row.reason === 'reopened'
      ? 'Reopened'
      : { new: 'Back to new', investigating: 'Investigating', diagnosed: 'Diagnosed', resolved: 'Resolved', regressed: 'Regressed', ignored: 'Ignored' }[row.to_status] ?? row.to_status
  const why = transitionSentence(row)
  const who = { console: 'by a person', agent: 'by the agent', investigation: 'by the investigation' }[row.actor]
  // The sentence for a plain pair of states repeats the word on the left.
  const note = [why.includes('→') ? null : why, who].filter(Boolean).join(' · ')
  return { what, note, dot: statusLook(row.to_status).dot }
}
