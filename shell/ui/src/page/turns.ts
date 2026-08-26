import type { Host } from '@iii-dev/console-ui'
import type { GitFileStatus } from './git'
import type { ReviewEntry } from './review'

export interface TurnFileHead {
  path: string
  kind: string
  root?: string | null
}

export interface SessionTurnSummary {
  turn_id: string
  started_at: number
  ended_at?: number | null
  file_count: number
  files: TurnFileHead[]
}

export interface TurnPreImage {
  revision?: string | null
  content?: string | null
  truncated?: boolean
  missing?: boolean
  binary?: boolean
}

export interface TurnFileRecord extends TurnFileHead {
  cause: string
  first_seen: number
  last_seen: number
  from?: string | null
  before?: TurnPreImage | null
  after_revision?: string | null
}

export interface SessionTurn {
  turn_id: string
  started_at: number
  ended_at?: number | null
  files: TurnFileRecord[]
}

export async function fetchSessionTurns(
  host: Host,
  sessionId: string,
): Promise<SessionTurnSummary[]> {
  const out = await host.iii.trigger<{ turns?: SessionTurnSummary[] }>(
    'shell::turns::list',
    { session_id: sessionId },
  )
  return out.turns ?? []
}

export async function fetchSessionTurn(
  host: Host,
  sessionId: string,
  turnId: string,
): Promise<SessionTurn | null> {
  const out = await host.iii.trigger<{ turn?: SessionTurn | null }>(
    'shell::turns::get',
    { session_id: sessionId, turn_id: turnId },
  )
  return out.turn ?? null
}

export function turnLabel(turn: { started_at: number; file_count: number }): string {
  const when = new Date(turn.started_at)
  const time = Number.isFinite(when.getTime())
    ? when.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : 'turn'
  const files = turn.file_count === 1 ? '1 file' : `${turn.file_count} files`
  return `${time} · ${files}`
}

function statusFor(kind: string): GitFileStatus {
  switch (kind) {
    case 'created':
      return 'added'
    case 'deleted':
      return 'deleted'
    case 'moved':
      return 'renamed'
    default:
      return 'modified'
  }
}

/** Root-relative path for an absolute one under `root`, else null. */
export function relativeToRoot(path: string, root: string): string | null {
  const base = root.replace(/\/+$/, '')
  if (path === base) return null
  if (!path.startsWith(`${base}/`)) return null
  const rel = path.slice(base.length + 1)
  return rel === '' ? null : rel
}

/** The stored pre-image as the review baseline: the body when it is whole,
    an empty body for a file the turn created, and `null` (committed
    fallback) when the body was truncated, binary, or unreadable. */
export function baselineFor(before: TurnPreImage | null | undefined): string | null {
  if (!before) return null
  if (before.missing) return ''
  if (before.binary || before.truncated) return null
  return typeof before.content === 'string' ? before.content : null
}

export interface TurnEntries {
  entries: ReadonlyMap<string, ReviewEntry>
  /** Files the turn changed outside the browsed root. */
  outside: number
  outsideRoot: string | null
}

export interface SessionActivitySummary {
  inside: number
  outside: number
  outsideRoot: string | null
}

function hasNoDurableChange(file: TurnFileRecord): boolean {
  if (file.before?.missing && file.kind === 'deleted') return true
  return (
    typeof file.before?.revision === 'string' &&
    file.before.revision === file.after_revision
  )
}

export function reviewEntriesFromTurn(turn: SessionTurn, root: string): TurnEntries {
  const entries = new Map<string, ReviewEntry>()
  const outsidePaths: string[] = []
  for (const file of turn.files) {
    if (hasNoDurableChange(file)) continue
    const rel = relativeToRoot(file.path, root)
    if (rel === null) {
      outsidePaths.push(file.path)
      continue
    }
    const from = file.from ? relativeToRoot(file.from, root) : null
    const status = statusFor(file.kind)
    const entry: ReviewEntry = {
      path: rel,
      change: {
        path: rel,
        status,
        staged: false,
        ...(from ? { from } : {}),
      },
      // An observed creation has no stored pre-image, but its true
      // baseline is known anyway: the file did not exist before the turn.
      baseline:
        file.before == null && file.kind === 'created'
          ? ''
          : baselineFor(file.before),
    }
    entries.set(rel, entry)
  }
  return {
    entries,
    outside: new Set(outsidePaths).size,
    outsideRoot: sharedFolder([...new Set(outsidePaths)]),
  }
}

function parentPath(path: string): string | null {
  const normalized = path.replace(/\/+$/, '')
  const separator = normalized.lastIndexOf('/')
  if (separator <= 0) return null
  return normalized.slice(0, separator)
}

function sharedFolder(paths: readonly string[]): string | null {
  const parents = paths.map(parentPath).filter((path): path is string => path !== null)
  if (parents.length !== paths.length || parents.length === 0) return null
  const parts = parents.map((path) => path.split('/').filter(Boolean))
  const common: string[] = []
  for (let index = 0; index < parts[0].length; index += 1) {
    const segment = parts[0][index]
    if (parts.some((path) => path[index] !== segment)) break
    common.push(segment)
  }
  return common.length >= 2 ? `/${common.join('/')}` : null
}

export function summarizeSessionActivity(
  turns: readonly SessionTurnSummary[],
  root: string,
): SessionActivitySummary {
  const paths = new Set(turns.flatMap((turn) => turn.files.map((file) => file.path)))
  const outsidePaths = [...paths].filter((path) => relativeToRoot(path, root) === null)
  return {
    inside: paths.size - outsidePaths.length,
    outside: outsidePaths.length,
    outsideRoot: sharedFolder(outsidePaths),
  }
}

export function reviewEntriesFromSession(
  turns: readonly SessionTurn[],
  root: string,
): TurnEntries {
  const chronological = [...turns].sort((left, right) => left.started_at - right.started_at)
  const history = new Map<
    string,
    { first: TurnFileRecord; latest: TurnFileRecord }
  >()
  const outside = new Set<string>()

  for (const turn of chronological) {
    for (const file of turn.files) {
      if (hasNoDurableChange(file)) continue
      if (relativeToRoot(file.path, root) === null) {
        outside.add(file.path)
        continue
      }
      const previous = history.get(file.path)
      history.set(file.path, {
        first: previous?.first ?? file,
        latest: file,
      })
    }
  }

  const entries = new Map<string, ReviewEntry>()
  for (const [path, { first, latest }] of history) {
    const rel = relativeToRoot(path, root)
    if (rel === null) continue
    const firstBaseline =
      first.before == null && first.kind === 'created'
        ? ''
        : baselineFor(first.before)
    if (firstBaseline === '' && latest.kind === 'deleted') continue

    const status: GitFileStatus =
      latest.kind === 'deleted'
        ? 'deleted'
        : firstBaseline === ''
          ? 'added'
          : statusFor(latest.kind)
    const from = latest.from ? relativeToRoot(latest.from, root) : null
    entries.set(rel, {
      path: rel,
      change: {
        path: rel,
        status,
        staged: false,
        ...(from ? { from } : {}),
      },
      baseline: firstBaseline,
    })
  }
  return {
    entries,
    outside: outside.size,
    outsideRoot: sharedFolder([...outside]),
  }
}
