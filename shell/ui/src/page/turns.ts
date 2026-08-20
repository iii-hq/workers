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
}

export function reviewEntriesFromTurn(turn: SessionTurn, root: string): TurnEntries {
  const entries = new Map<string, ReviewEntry>()
  let outside = 0
  for (const file of turn.files) {
    const rel = relativeToRoot(file.path, root)
    if (rel === null) {
      outside += 1
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
      baseline: baselineFor(file.before),
    }
    entries.set(rel, entry)
  }
  return { entries, outside }
}
