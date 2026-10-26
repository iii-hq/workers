/**
 * The items of the ticket screen's List (design §6): below the head (the
 * List's header), the description in pieces, the subtasks, the linked
 * tickets, the activity's heading with its filter, and the activity with long
 * comments folded. Each is an item, so the host composes only what is on
 * screen. Pure: no `iii`.
 */

import { type ActivityFilter, counts, timeline, type Who } from './activity'
import type { Brief, Column, Summary, Ticket } from './api'
import { folded, pieces } from './blocks'
import { STEP_ICON, statusLabel, statusStep } from './format'
import { type LinkedTicket, linkTicketKeys } from './links'
import { childOrder } from './rules'

/** Subtasks listed before "Show all". */
export const SUBTASKS_SHOWN = 8

export const FILTERS: ActivityFilter[] = ['all', 'comments', 'changes']

/**
 * One item. `kind` picks the template: 0 an event, 1 a comment, 2 a piece of
 * the description, 3 the rest (a heading with its count, progress or filter,
 * a note, a ticket row, the subtasks' buttons), whose parts show by their
 * `is…`/`has…` fields.
 */
export interface Item {
  key: string
  kind: 0 | 1 | 2 | 3
  text?: string
  // events and comments (activity.ts)
  who?: string
  time?: string
  icon?: string
  hue?: string
  comment?: string
  root?: string
  canReply?: boolean
  isReply?: boolean
  /** A long comment showing its start; `canFold`: one shown whole that can fold again. */
  folded?: boolean
  canFold?: boolean
  // kind 3
  isHeading?: boolean
  heading?: string
  hasCount?: boolean
  count?: string
  isActivity?: boolean
  options?: string[]
  selected?: number
  hasProgress?: boolean
  progress?: number
  isNote?: boolean
  note?: string
  isRow?: boolean
  rowIcon?: string
  rowTitle?: string
  rowSubtitle?: string
  /** The ticket key a row opens. */
  open?: string
  hasButtons?: boolean
  hasAll?: boolean
  allText?: string
  hasAdd?: boolean
}

/** A kind 3 item: every part it does not name is off (a missing field would read as visible). */
function other(key: string, fields: Partial<Item>): Item {
  return {
    key,
    kind: 3,
    isHeading: false,
    heading: '',
    hasCount: false,
    count: '',
    isActivity: false,
    hasProgress: false,
    isNote: false,
    note: '',
    isRow: false,
    hasButtons: false,
    hasAll: false,
    hasAdd: false,
    ...fields,
  }
}

export interface ItemsInput {
  ticket: Ticket
  filter: ActivityFilter
  columns: readonly Column[]
  done: string
  canceled: string | null
  who: Who
  now: number
  /** The tickets the text may link to. */
  linked: readonly LinkedTicket[]
  /** The linked tickets to list (without the parent and the subtasks, which have their places). */
  links: readonly { ticket: Summary; relation: string }[]
  /** Comments shown whole although long. */
  expanded: ReadonlySet<string>
  allSubtasks: boolean
  /** The ticket takes writes now (live, online, nothing in flight). */
  editable: boolean
}

export function ticketItems(input: ItemsInput): Item[] {
  const { ticket: t, linked } = input
  const glyph = (status: string) => STEP_ICON[statusStep(input.columns, status, input.done, input.canceled)]
  const items: Item[] = []

  if (t.description.trim()) {
    pieces(t.description).forEach((piece, i) => {
      items.push({ key: `@d${i}`, kind: 2, text: linkTicketKeys(piece, linked) })
    })
  } else items.push(other('@d-none', { isNote: true, note: 'No description yet.' }))

  if (!t.parent_id && t.children.length) {
    const ordered = childOrder(t.children, input.done, input.canceled)
    const shown = input.allSubtasks ? ordered : ordered.slice(0, SUBTASKS_SHOWN)
    const { done, total } = t.progress
    items.push(
      other('@sub', {
        isHeading: true,
        heading: 'Subtasks',
        hasCount: true,
        count: `${done} of ${total}`,
        hasProgress: true,
        progress: total ? done / total : 0,
      }),
    )
    for (const child of shown) items.push(row(`@s:${child.id}`, child, statusLabel(input.columns, child.status), glyph))
    const more = !input.allSubtasks && ordered.length > SUBTASKS_SHOWN
    items.push(
      other('@sub-more', {
        hasButtons: more || input.editable,
        hasAll: more,
        allText: `Show all ${ordered.length}`,
        hasAdd: input.editable,
      }),
    )
  }

  if (input.links.length) {
    items.push(other('@links', { isHeading: true, heading: 'Linked tickets' }))
    for (const { ticket: linkedTicket, relation } of input.links) {
      items.push(
        row(
          `@l:${linkedTicket.id}`,
          linkedTicket,
          `${statusLabel(input.columns, linkedTicket.status)} · ${relation}`,
          glyph,
        ),
      )
    }
  }

  const tally = counts(t)
  items.push(
    other('@activity', {
      isHeading: true,
      heading: 'Activity',
      isActivity: true,
      options: ['All', `Comments ${tally.comments}`, `Changes ${tally.changes}`],
      selected: Math.max(0, FILTERS.indexOf(input.filter)),
    }),
  )
  const rows = timeline(t, input.filter, input.columns, input.who, input.now, !t.deleted_at)
  if (!rows.length) {
    const note =
      input.filter === 'all'
        ? 'Nothing has happened yet.'
        : input.filter === 'comments'
          ? 'No comments yet.'
          : 'No changes yet.'
    items.push(other('@activity-none', { isNote: true, note }))
  }
  for (const r of rows) {
    if (r.kind === 0) {
      items.push(r)
      continue
    }
    const start = folded(r.text)
    const isFolded = start !== null && !input.expanded.has(r.comment)
    items.push({
      ...r,
      text: linkTicketKeys(isFolded ? (start as string) : r.text, linked),
      folded: isFolded,
      canFold: start !== null && !isFolded,
    })
  }
  return items
}

function row(key: string, t: Brief | Summary, subtitle: string, glyph: (status: string) => string): Item {
  return other(key, {
    isRow: true,
    rowIcon: glyph(t.status),
    rowTitle: `${t.key}  ${t.title}`,
    rowSubtitle: subtitle,
    open: t.key,
  })
}
