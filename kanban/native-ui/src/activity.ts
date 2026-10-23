/**
 * The ticket's activity as the screen lists it (design §6.3): the console
 * UI's sentences (ui/src/ticket.tsx, ui/src/subtask-rules.ts) as plain text,
 * consecutive subtask additions folded into one row, and the comments
 * threaded: each root comment in its place, its replies right after it.
 */

import type { Activity, Change, Column, Comment, Ticket } from './api'
import { ago, statusLabel } from './format'

export type ActivityFilter = 'all' | 'comments' | 'changes'

/** One item of the activity in the ticket's List. `kind` picks the template: 0 an event, 1 a comment (or a reply). */
export interface TimelineRow {
  key: string
  kind: 0 | 1
  /** A reply, drawn under its root and indented. */
  isReply: boolean
  /** The author or actor, by name. */
  who: string
  /** An event's sentence, after its actor's name; a comment's body (Markdown). */
  text: string
  time: string
  icon: string
  hue: string
  /** A comment's id and the root it belongs to (replies answer the root). */
  comment: string
  root: string
  canReply: boolean
}

const text = (v: unknown): string | null => (typeof v === 'string' && v ? v : null)
const list = (v: unknown): string[] =>
  Array.isArray(v) ? [...new Set(v.filter((s): s is string => typeof s === 'string'))] : []

function labelSentence(change: Change): string {
  const from = list(change.from)
  const to = list(change.to)
  const parts = [
    { verb: 'added', labels: to.filter((l) => !from.includes(l)) },
    { verb: 'removed', labels: from.filter((l) => !to.includes(l)) },
  ].filter((p) => p.labels.length > 0)
  if (!parts.length) return 'changed the labels'
  return parts
    .map((p) => `${p.verb} the ${p.labels.length === 1 ? 'label' : 'labels'} ${p.labels.join(', ')}`)
    .join(', ')
}

function parentSentence(change: Change): string {
  const [from, to] = [text(change.from), text(change.to)]
  if (from && to) return `made this a subtask of ${to} instead of ${from}`
  if (to) return `made this a subtask of ${to}`
  if (from) return `detached this from ${from}`
  return 'changed the parent'
}

function planSentence(change: Change): string {
  const [from, to] = [text(change.from), text(change.to)]
  if (from && to) return `moved this from ${from} to ${to}`
  if (to) return `added this to ${to}`
  if (from) return `removed this from ${from}`
  return `changed the ${change.field}`
}

const FIELDS: Record<string, string> = {
  title: 'the title',
  description: 'the description',
  priority: 'the priority',
  labels: 'the labels',
  pills: 'the pills',
}

function changeSentence(change: Change, columns: readonly Column[], nameOf: (id: string) => string): string {
  switch (change.field) {
    case 'status':
      return `moved this from ${statusLabel(columns, String(change.from))} to ${statusLabel(columns, String(change.to))}`
    case 'assignee':
      return change.to ? `assigned this to ${nameOf(String(change.to))}` : 'unassigned this'
    case 'labels':
      return labelSentence(change)
    case 'parent':
      return parentSentence(change)
    case 'project':
    case 'release':
      return planSentence(change)
    default:
      return `changed ${FIELDS[change.field] ?? change.field}`
  }
}

function subtaskSentence(entry: Activity): string {
  const change = entry.changes.find((c) => c.field === 'subtask')
  const key = text(change?.to) ?? text(change?.from)
  const verb = entry.type === 'subtask.removed' ? 'removed' : 'added'
  return key ? `${verb} subtask ${key}` : `${verb} a subtask`
}

/** What an entry says, after its actor's name. */
export function sentence(entry: Activity, columns: readonly Column[], nameOf: (id: string) => string): string {
  switch (entry.type) {
    case 'ticket.created':
      return `created this in ${statusLabel(columns, String(entry.changes[0]?.to ?? ''))}`
    case 'ticket.moved':
      return entry.changes[0] ? changeSentence(entry.changes[0], columns, nameOf) : 'moved this'
    case 'ticket.updated':
      return entry.changes.length
        ? entry.changes.map((c) => changeSentence(c, columns, nameOf)).join(', ')
        : 'updated this'
    case 'subtask.added':
    case 'subtask.removed':
      return subtaskSentence(entry)
    case 'ticket.deleted':
      return 'deleted this ticket'
    case 'ticket.restored':
      return 'restored this ticket'
    default:
      return entry.type
  }
}

/** The glyph of an event: what happened, not that something did. */
export function eventIcon(entry: Activity): string {
  switch (entry.type) {
    case 'ticket.created':
      return 'circle-plus'
    case 'ticket.moved':
      return 'arrow-right'
    case 'ticket.deleted':
      return 'trash'
    case 'ticket.restored':
      return 'rotate-ccw'
    case 'subtask.added':
    case 'subtask.removed':
      return 'list-tree'
    default: {
      const only = (...fields: string[]) =>
        entry.changes.length > 0 && entry.changes.every((c) => fields.includes(c.field))
      return only('labels') ? 'tag' : only('parent') ? 'list-tree' : only('project', 'release') ? 'flag' : 'pencil'
    }
  }
}

const addedKey = (entry: Activity): string | null =>
  entry.type === 'subtask.added' ? text(entry.changes.find((c) => c.field === 'subtask')?.to) : null

export interface Who {
  name: (id: string) => string
  hue: (id: string) => string
}

/**
 * The rows of the activity, oldest first, for `filter`: events (several
 * subtasks added in a row by one actor are one row), and the comments
 * threaded, a reply right after its root (a reply whose root is gone stands
 * on its own).
 */
export function timeline(
  ticket: Ticket,
  filter: ActivityFilter,
  columns: readonly Column[],
  who: Who,
  now: number,
  canReply: boolean,
): TimelineRow[] {
  const comments = new Map(ticket.comments.map((c) => [c.id, c]))
  const replies = new Map<string, Comment[]>()
  for (const c of ticket.comments) {
    if (c.parent_id && comments.has(c.parent_id)) {
      const thread = replies.get(c.parent_id) ?? []
      thread.push(c)
      replies.set(c.parent_id, thread)
    }
  }
  const rows: TimelineRow[] = []
  const placed = new Set<string>()
  const commentRow = (c: Comment, reply: boolean): TimelineRow => ({
    key: `c:${c.id}`,
    kind: 1,
    isReply: reply,
    who: who.name(c.author),
    text: c.body,
    time: ago(c.created_at, now),
    icon: c.author === 'user' ? 'user' : 'bot',
    hue: who.hue(c.author),
    comment: c.id,
    root: reply && c.parent_id ? c.parent_id : c.id,
    canReply,
  })
  const pushComment = (c: Comment) => {
    if (placed.has(c.id)) return
    placed.add(c.id)
    const isReply = !!c.parent_id && comments.has(c.parent_id)
    rows.push(commentRow(c, isReply))
    if (!isReply) for (const r of replies.get(c.id) ?? []) if (!placed.has(r.id)) pushComment(r)
  }
  const entries = [...ticket.activity].sort((a, b) => a.seq - b.seq)
  let adds: { entry: Activity; keys: string[] } | null = null
  const flush = () => {
    if (!adds) return
    const { entry, keys } = adds
    const many = keys.length > 1
    rows.push({
      key: `a:${entry.id}`,
      kind: 0,
      isReply: false,
      who: who.name(entry.actor),
      text: `${who.name(entry.actor)} ${many ? `added ${keys.length} subtasks ${keys[0]} … ${keys[keys.length - 1]}` : `added subtask ${keys[0]}`}`,
      time: ago(entry.at, now),
      icon: 'list-tree',
      hue: 'none',
      comment: '',
      root: '',
      canReply: false,
    })
    adds = null
  }
  for (const entry of entries) {
    if (entry.type === 'comment.created') {
      flush()
      if (filter === 'changes') continue
      const c = entry.comment_id ? comments.get(entry.comment_id) : undefined
      // A reply is placed with its root; one whose root is gone comes in its own place.
      if (c && (!c.parent_id || !comments.has(c.parent_id))) pushComment(c)
      continue
    }
    if (filter === 'comments') continue
    const key = addedKey(entry)
    if (key && adds && adds.entry.actor === entry.actor) {
      adds.keys.push(key)
      continue
    }
    flush()
    if (key) {
      adds = { entry, keys: [key] }
      continue
    }
    rows.push({
      key: `a:${entry.id}`,
      kind: 0,
      isReply: false,
      who: who.name(entry.actor),
      text: `${who.name(entry.actor)} ${sentence(entry, columns, who.name)}`,
      time: ago(entry.at, now),
      icon: eventIcon(entry),
      hue: 'none',
      comment: '',
      root: '',
      canReply: false,
    })
  }
  flush()
  // Comments the activity does not list (an older worker, a bounded read): at the end, oldest first.
  if (filter !== 'changes') {
    for (const c of [...ticket.comments].sort((a, b) => (a.created_at < b.created_at ? -1 : 1))) {
      if (!c.parent_id || !comments.has(c.parent_id)) pushComment(c)
    }
  }
  return rows
}

/** How many entries each filter shows: comments, and everything else. */
export function counts(ticket: Ticket): { comments: number; changes: number } {
  const comments = ticket.comments.length
  const changes = ticket.activity.filter((a) => a.type !== 'comment.created').length
  return { comments, changes }
}
