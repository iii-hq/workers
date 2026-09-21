/**
 * The page's pure vocabulary: what a state is called, what it lets you do,
 * and how a row is drawn. Kept out of the components so the rules can be
 * read — and tested — without mounting anything.
 */

/** @typedef {'new'|'investigating'|'diagnosed'|'resolved'|'regressed'|'ignored'} GroupStatus */

/** The states the list shows when nobody has narrowed it. */
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
      return ['investigate', 'reopen']
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
export function sparklineBars(counts) {
  const peak = Math.max(0, ...counts)
  return counts.map((count) => ({
    count,
    height: count === 0 ? 0 : Math.max(8, Math.round((count / peak) * 100)),
  }))
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
