import type { BoardColumn, BoardData, TicketSummary } from './shared'

/* Pure board state: no React, no console-ui, so `board-state.check.ts` runs it under plain node. */

/** Ticket id → when it was deleted (ms). A copy of the ticket older than that is stale and never comes back. */
export type Tombstones = Map<string, number>

export type WriteKind = {
  /** `ticket.moved` (event or reply): `order` is the position in a lane the worker just renumbered. */
  moved?: boolean
  /** `ticket.restored`: the ticket leaves the deleted set. */
  restored?: boolean
  /** Replay over a board read computed after this write: its `deleted_count` already reflects it. */
  counted?: boolean
}

/** A ticket the worker returned, kept until the board read that was in flight can be replayed over. */
export type Write = { summary: TicketSummary; kind: WriteKind }

const renumber = (tickets: TicketSummary[]) => tickets.map((ticket, index) => ({ ...ticket, order: index + 1 }))

/**
 * Put one ticket where the worker has it. `order` is the worker's number, not
 * a position: a delete or a status change by update leaves a gap in the lane
 * and nobody renumbers it, so a new card is placed before the first card with
 * a greater number and a card that stays in its lane keeps its slot.
 *
 * A move is the exception: the worker renumbers the target lane 1..n (and
 * compacts the lane the card left), so there `order - 1` is the index and the
 * same renumbering is mirrored here. Optimistic moves use it too.
 */
export function upsertSummary(columns: BoardColumn[], summary: TicketSummary, moved = false): BoardColumn[] {
  const from = columns.find((column) => column.tickets.some((ticket) => ticket.id === summary.id))
  const into = summary.deleted_at ? undefined : columns.find((column) => column.id === summary.status)
  if (!moved && from && from === into) {
    const tickets = from.tickets.map((ticket) => (ticket.id === summary.id ? summary : ticket))
    return columns.map((column) => (column === from ? { ...column, tickets } : column))
  }
  return columns.map((column) => {
    if (column !== from && column !== into) return column
    let tickets = column.tickets.filter((ticket) => ticket.id !== summary.id)
    if (column === into) {
      const next = tickets.findIndex((ticket) => ticket.order > summary.order)
      const position = moved
        ? Math.max(0, Math.min(summary.order - 1, tickets.length))
        : next < 0
          ? tickets.length
          : next
      tickets = [...tickets.slice(0, position), summary, ...tickets.slice(position)]
    }
    return { ...column, tickets: moved ? renumber(tickets) : tickets }
  })
}

const time = (iso: string) => Date.parse(iso)

/**
 * Apply a ticket the worker returned (event, reply of a write, bulk result)
 * to the board, unless the board already has that version or a newer one, or
 * the ticket was deleted after it was written. Returns `data` itself when
 * nothing changes. Also keeps `deleted_count` and `ticket_count` in step, and
 * the tombstones.
 */
export function applyWrite(
  data: BoardData,
  summary: TicketSummary,
  tombstones: Tombstones,
  kind: WriteKind = {},
): BoardData {
  const at = time(summary.updated_at)
  const shown = data.columns.flatMap((column) => column.tickets).find((ticket) => ticket.id === summary.id)
  if (shown && time(shown.updated_at) >= at) return data
  const deletedAt = tombstones.get(summary.id)
  if (deletedAt !== undefined && at <= deletedAt) return data
  if (summary.deleted_at) tombstones.set(summary.id, Math.max(at, time(summary.deleted_at) || 0))
  else tombstones.delete(summary.id)
  const columns = upsertSummary(data.columns, summary, kind.moved)
  let deleted = data.deleted_count
  if (summary.deleted_at) {
    if (shown) deleted += 1
  } else if (!shown && !kind.counted && (kind.restored || deletedAt !== undefined)) {
    // A ticket in a status that is not a column is never `shown`, so the read cannot tell whether it counted this restore.
    deleted = Math.max(0, deleted - 1)
  }
  return {
    ...data,
    columns,
    deleted_count: deleted,
    ticket_count: columns.reduce((total, column) => total + column.tickets.length, 0),
  }
}

/**
 * A board read from the worker may predate a deletion this tab saw: drop the
 * tickets that were deleted after the version it holds (and count them as
 * deleted), and forget the tombstone of one restored since.
 */
export function dropResurrected(board: BoardData, tombstones: Tombstones): BoardData {
  let dropped = 0
  const columns = board.columns.map((column) => {
    const tickets = column.tickets.filter((ticket) => {
      const deletedAt = tombstones.get(ticket.id)
      if (deletedAt === undefined) return true
      if (time(ticket.updated_at) > deletedAt) {
        tombstones.delete(ticket.id)
        return true
      }
      dropped += 1
      return false
    })
    return tickets.length === column.tickets.length ? column : { ...column, tickets }
  })
  return dropped === 0
    ? board
    : { ...board, columns, deleted_count: board.deleted_count + dropped, ticket_count: board.ticket_count - dropped }
}

/**
 * A board read that was in flight while `journal` (the writes this tab saw
 * meanwhile) arrived: drop what it resurrects, then replay the writes over it.
 * A write stamped no later than the read was computed is already in the read's
 * `deleted_count`. ponytail: `updated_at` of the read is taken just after the
 * store is read, so a write landing in that sub-millisecond gap is counted
 * twice; add a version to `kanban::board::get` if it ever shows.
 */
export function applyRead(board: BoardData, journal: Write[], tombstones: Tombstones): BoardData {
  const readAt = time(board.updated_at)
  let next = dropResurrected(board, tombstones)
  for (const { summary, kind } of journal) {
    next = applyWrite(next, summary, tombstones, { ...kind, counted: time(summary.updated_at) <= readAt })
  }
  return next
}
