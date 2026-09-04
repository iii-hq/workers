import type { Host } from '@iii-dev/console-ui'
import type { GitFileStatus } from './git'

export interface TurnAgentRef {
  session_id: string
  name?: string | null
}

export interface TurnFileHead {
  path: string
  kind: string
  root?: string | null
  /** The sub-agent that made the change, when it was not the turn's own. */
  agent?: TurnAgentRef | null
}

export interface SessionTurnSummary {
  turn_id: string
  started_at: number
  ended_at?: number | null
  /** First characters of the message that started the turn. */
  title?: string | null
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
  /** The body the turn left behind, when a later turn kept it. Absent
      means the working copy is the after side. */
  after?: TurnPreImage | null
}

export interface SessionTurn {
  turn_id: string
  started_at: number
  ended_at?: number | null
  title?: string | null
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

/** What a timeline calls a turn: its message preview, else its ordinal. */
export function turnTitle(turn: { title?: string | null }, ordinal: number): string {
  const title = turn.title?.trim()
  return title ? title : `Turn ${ordinal}`
}

export function statusFor(kind: string): GitFileStatus {
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
