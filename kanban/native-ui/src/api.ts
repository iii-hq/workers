/**
 * Typed wrappers over the kanban worker's functions and its `kanban:change`
 * trigger, as the native app uses them (design §8; the wire source is
 * kanban/src/functions.rs and events.rs, the same ids and payloads the
 * console UI uses in ui/src/shared.tsx). Every answer is read entry by
 * entry: a bad entry is skipped, a field of the wrong type takes its default.
 */

import { IiiError, iii, type JsonObject, type Unsubscribe } from '@iii-dev/native-ui'

export interface Column {
  id: string
  label: string
}

export interface Pill {
  icon: string
  text: string
}

/** A board card (`TicketSummary`, kanban/src/board.rs). */
export interface Summary {
  id: string
  key: string
  title: string
  status: string
  priority: string
  assignee: string | null
  labels: string[]
  pills: Pill[]
  order: number
  created_at: string
  updated_at: string
  deleted_at: string | null
  comment_count: number
  parent_id: string | null
  project_id: string | null
  release_id: string | null
}

/** A ticket in short: a parent, a child (`Brief`). */
export interface Brief {
  id: string
  key: string
  title: string
  status: string
  deleted_at: string | null
  project_id: string | null
  release_id: string | null
}

export interface Comment {
  id: string
  parent_id: string | null
  body: string
  author: string
  created_at: string
}

export interface Change {
  field: string
  from: unknown
  to: unknown
}

export interface Activity {
  id: string
  seq: number
  type: string
  actor: string
  at: string
  changes: Change[]
  comment_id: string | null
}

export interface Ticket extends Summary {
  description: string
  created_by: string
  comments: Comment[]
  activity: Activity[]
  parent: Brief | null
  children: Brief[]
  progress: { done: number; total: number }
  /** Activity entries the worker has; `activity` holds fewer when old changes were left out. */
  activity_total: number
}

export interface Agent {
  id: string
  name: string
  color: string | null
}

export interface Project {
  id: string
  name: string
  color: string | null
  lead: string | null
  archived: boolean
  total: number
  done: number
}

export interface Release {
  id: string
  name: string
  target_at: string | null
  released: boolean
  status: string
  total: number
  done: number
}

/** One column of a board read: its first rows and how many it has. */
export interface LaneRead {
  id: string
  label: string
  rows: Summary[]
  total: number
}

export interface BoardRead {
  columns: LaneRead[]
  ticket_count: number
  priorities: string[]
  default_status: string
  done_status: string
  canceled_status: string | null
  id_prefix: string
  /** The newest write this read holds: one stamped later is not in it (`as_of`; an older worker's read time). */
  as_of: string
  projects: Project[]
  releases: Release[]
}

export interface Config {
  columns: Column[]
  priorities: string[]
  default_status: string
  done_status: string
  canceled_status: string | null
  id_prefix: string
}

export interface Links {
  mentions: Summary[]
  mentioned_by: Summary[]
}

/** A `kanban:change` delivery (`ticket: summary`): a card, or `config.updated` alone. */
export interface ChangeEvent {
  event: string
  at: string
  ticket: Summary | null
  /** The status this write moved the ticket from, when its activity says so. */
  from: string | null
}

export const CHANGE_TRIGGER = 'kanban:change'
export const CONFIG_EVENT = 'config.updated'

// ── Reading values ──────────────────────────────────────────────────────────

type Raw = Record<string, unknown>

const obj = (v: unknown): Raw | null => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Raw) : null)
const str = (v: unknown, fallback = ''): string => (typeof v === 'string' ? v : fallback)
const strOrNull = (v: unknown): string | null => (typeof v === 'string' && v ? v : null)
const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : [])
const strings = (v: unknown): string[] => arr(v).filter((s): s is string => typeof s === 'string')

/** Entries of `v` read by `read`, the bad ones skipped; a repeated id keeps its last entry, where it last is. */
function each<T extends { id: string }>(v: unknown, read: (raw: Raw) => T | null): T[] {
  const byId = new Map<string, T>()
  for (const entry of arr(v)) {
    const raw = obj(entry)
    const item = raw && read(raw)
    if (!item) continue
    byId.delete(item.id)
    byId.set(item.id, item)
  }
  return [...byId.values()]
}

export function summaryOf(raw: Raw): Summary | null {
  const id = raw.id
  const key = raw.key
  if (typeof id !== 'string' || !id || typeof key !== 'string' || !key || typeof raw.status !== 'string') return null
  return {
    id,
    key,
    title: str(raw.title),
    status: raw.status,
    priority: str(raw.priority),
    assignee: strOrNull(raw.assignee),
    labels: strings(raw.labels),
    pills: arr(raw.pills)
      .map(obj)
      .filter((p): p is Raw => !!p && typeof p.text === 'string' && !!p.text)
      .map((p) => ({ icon: str(p.icon), text: p.text as string })),
    order: num(raw.order),
    created_at: str(raw.created_at),
    updated_at: str(raw.updated_at),
    deleted_at: strOrNull(raw.deleted_at),
    comment_count: num(raw.comment_count ?? arr(raw.comments).length),
    parent_id: strOrNull(raw.parent_id),
    project_id: strOrNull(raw.project_id),
    release_id: strOrNull(raw.release_id),
  }
}

export const summariesOf = (v: unknown): Summary[] => each(v, summaryOf)

function briefOf(raw: Raw): Brief | null {
  if (typeof raw.id !== 'string' || !raw.id || typeof raw.key !== 'string') return null
  return {
    id: raw.id,
    key: raw.key,
    title: str(raw.title),
    status: str(raw.status),
    deleted_at: strOrNull(raw.deleted_at),
    project_id: strOrNull(raw.project_id),
    release_id: strOrNull(raw.release_id),
  }
}

function commentOf(raw: Raw): Comment | null {
  if (typeof raw.id !== 'string' || !raw.id) return null
  return {
    id: raw.id,
    parent_id: strOrNull(raw.parent_id),
    body: str(raw.body),
    author: str(raw.author, 'user') || 'user',
    created_at: str(raw.created_at),
  }
}

function activityOf(raw: Raw): Activity | null {
  if (typeof raw.id !== 'string' || !raw.id || typeof raw.type !== 'string') return null
  return {
    id: raw.id,
    seq: num(raw.seq),
    type: raw.type,
    actor: str(raw.actor, 'user') || 'user',
    at: str(raw.at),
    changes: arr(raw.changes)
      .map(obj)
      .filter((c): c is Raw => !!c && typeof c.field === 'string')
      .map((c) => ({ field: c.field as string, from: c.from, to: c.to })),
    comment_id: strOrNull(raw.comment_id),
  }
}

export function ticketOf(v: unknown): Ticket | null {
  const raw = obj(v)
  const summary = raw && summaryOf(raw)
  if (!raw || !summary) return null
  const comments = each(raw.comments, commentOf)
  const parent = obj(raw.parent)
  const progress = obj(raw.progress)
  const activity = each(raw.activity, activityOf)
  return {
    ...summary,
    comment_count: comments.length,
    description: str(raw.description),
    created_by: str(raw.created_by, 'user') || 'user',
    comments,
    activity,
    parent: parent ? briefOf(parent) : null,
    children: each(raw.children, briefOf),
    progress: { done: num(progress?.done), total: num(progress?.total) },
    activity_total: Math.max(num(raw.activity_total), activity.length),
  }
}

export function agentsOf(v: unknown): Agent[] {
  return each(obj(v)?.agents, (raw) =>
    typeof raw.id === 'string' && raw.id
      ? { id: raw.id, name: str(raw.name) || raw.id, color: strOrNull(raw.color) }
      : null,
  )
}

export function projectsOf(v: unknown): Project[] {
  return each(v, (raw) =>
    typeof raw.id === 'string' && raw.id
      ? {
          id: raw.id,
          name: str(raw.name) || raw.id,
          color: strOrNull(raw.color),
          lead: strOrNull(raw.lead),
          archived: raw.status === 'archived' || !!strOrNull(raw.archived_at),
          total: num(raw.total),
          done: num(raw.done),
        }
      : null,
  )
}

export function releasesOf(v: unknown): Release[] {
  return each(v, (raw) =>
    typeof raw.id === 'string' && raw.id
      ? {
          id: raw.id,
          name: str(raw.name) || raw.id,
          target_at: strOrNull(raw.target_at),
          released: !!strOrNull(raw.released_at),
          status: str(raw.status, 'planned'),
          total: num(raw.total),
          done: num(raw.done),
        }
      : null,
  )
}

function columnsOf(v: unknown): Column[] {
  return each(v, (raw) =>
    typeof raw.id === 'string' && raw.id ? { id: raw.id, label: str(raw.label) || raw.id } : null,
  )
}

export function configOf(v: unknown): Config {
  const raw = obj(v) ?? {}
  const columns = columnsOf(raw.columns)
  return {
    columns,
    priorities: strings(raw.priorities),
    default_status: str(raw.default_status) || columns[0]?.id || '',
    done_status: str(raw.done_status) || columns[columns.length - 1]?.id || '',
    canceled_status: strOrNull(raw.canceled_status),
    id_prefix: str(raw.id_prefix),
  }
}

export function boardOf(v: unknown): BoardRead {
  const raw = obj(v) ?? {}
  const config = configOf(raw)
  // A row whose status is not its column's would be drawn in the wrong lane, and one twice on the board
  // (two columns) stays in the first: both are skipped, and a total the worker did not give counts what is left.
  const seen = new Set<string>()
  const columns = each(raw.columns, (c) => {
    if (typeof c.id !== 'string' || !c.id) return null
    const rows = summariesOf(c.tickets).filter((t) => t.status === c.id && !t.deleted_at && !seen.has(t.id))
    for (const t of rows) seen.add(t.id)
    return { id: c.id, label: str(c.label) || c.id, rows, total: Math.max(num(c.total), rows.length) }
  })
  return {
    ...config,
    columns,
    ticket_count: num(raw.ticket_count),
    as_of: str(raw.as_of) || str(raw.updated_at),
    projects: projectsOf(raw.projects),
    releases: releasesOf(raw.releases),
  }
}

export function linksOf(v: unknown): Links {
  const raw = obj(v) ?? {}
  return { mentions: summariesOf(raw.mentions), mentioned_by: summariesOf(raw.mentioned_by) }
}

/** A `kanban:change` payload, or null when it is neither a ticket event nor `config.updated`. */
export function changeOf(v: unknown): ChangeEvent | null {
  const raw = obj(v)
  if (!raw || typeof raw.event !== 'string') return null
  if (raw.event === CONFIG_EVENT) return { event: CONFIG_EVENT, at: str(raw.at), ticket: null, from: null }
  const ticketRaw = obj(raw.ticket)
  const ticket = ticketRaw && summaryOf(ticketRaw)
  if (!ticket) return null
  // The status change of this very write (its activity entry is stamped with the event's time).
  const activity = obj(raw.activity)
  const status =
    activity && str(activity.at) === str(raw.at)
      ? arr(activity.changes)
          .map(obj)
          .find((c) => c?.field === 'status')
      : undefined
  return { event: raw.event, at: str(raw.at), ticket, from: status ? strOrNull(status.from) : null }
}

// ── Calls ───────────────────────────────────────────────────────────────────

export const CALL_TIMEOUT_MS = 30_000
/** Rows of each column in the first board read, and per page after. */
export const FIRST_PAGE = 50
export const NEXT_PAGE = 100

/**
 * Calls `kanban::<fn>` with an explicit timeout, and settles at that timeout
 * on its own too: the UI state behind a call must recover even when the host
 * does not enforce it. A late answer is dropped.
 */
function call(fn: string, payload: JsonObject = {}): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new IiiError('timeout', "The engine didn't answer in time")), CALL_TIMEOUT_MS)
    iii.call(`kanban::${fn}`, payload, { timeoutMs: CALL_TIMEOUT_MS }).then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (err: unknown) => {
        clearTimeout(timer)
        reject(err)
      },
    )
  })
}

/** A write's ticket, or a failure in words when the worker answered something else. */
async function written(fn: string, payload: JsonObject): Promise<Ticket> {
  const ticket = ticketOf(await call(fn, { ...payload, actor: 'user' }))
  if (!ticket) throw new Error("The kanban worker didn't return the ticket")
  return ticket
}

export const kanban = {
  config: () => call('config::info').then(configOf),
  /** The board, `limit` rows per column (none = every row). */
  board: (limit?: number) => call('board::get', limit ? { limit } : {}).then(boardOf),
  page: (status: string, offset: number, limit = NEXT_PAGE) =>
    call('ticket::list', { status, offset, limit }).then((v) => {
      const raw = obj(v) ?? {}
      const rows = summariesOf(raw.tickets).filter((t) => t.status === status && !t.deleted_at)
      return { rows, total: Math.max(num(raw.total), offset + rows.length) }
    }),
  ticket: async (id: string) => {
    const ticket = ticketOf(await call('ticket::get', { id }))
    if (!ticket) throw new Error(`No ticket matches ${id}`)
    return ticket
  },
  links: (id: string) => call('ticket::links', { id }).then(linksOf),
  agents: () => call('agent::list').then(agentsOf),
  projects: () => call('project::list', { include_archived: true }).then((v) => projectsOf(obj(v)?.projects)),
  releases: () => call('release::list').then((v) => releasesOf(obj(v)?.releases)),
  create: (payload: JsonObject) => written('ticket::create', payload),
  update: (id: string, patch: JsonObject) => written('ticket::update', { id, ...patch }),
  /** `index` absent = the end of the column. */
  move: (id: string, status: string, index?: number) =>
    written('ticket::move', index === undefined ? { id, status } : { id, status, index }),
  remove: (id: string) => written('ticket::delete', { id }),
  restore: (id: string) => written('ticket::restore', { id }),
  comment: async (ticketId: string, body: string, parentId: string | null) => {
    const raw = obj(await call('comment::create', { ticket_id: ticketId, body, parent_id: parentId, author: 'user' }))
    const ticket = ticketOf(raw?.ticket)
    if (!ticket) throw new Error("The kanban worker didn't return the ticket")
    return ticket
  },
}

/** Binds `kanban:change` (bind first, read after `ready`); a failed binding is reported and left unbound. */
export function listen(handler: (event: ChangeEvent) => void): Unsubscribe & { ready: Promise<boolean> } {
  const off = iii.on(CHANGE_TRIGGER, { ticket: 'summary' }, (payload) => {
    const event = changeOf(payload)
    if (event) handler(event)
  })
  const ready = off.ready.then(
    () => true,
    (err: unknown) => {
      iii.reportError(err)
      return false
    },
  )
  return Object.assign(() => off(), { ready })
}
