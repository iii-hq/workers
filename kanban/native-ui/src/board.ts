/**
 * Pure board state (design §7, §8.2, §8.4): the lanes as the app has them,
 * each the first rows of a worker column (all of them once `rows` reaches
 * `total`), and how a write, a read and a page change them. No `iii` here, so
 * the rules run in plain Node too. Ported from the console UI's
 * ui/src/board-state.ts, with partial lanes and their totals.
 */

import type { BoardRead, Summary } from './api'

export interface Lane {
  id: string
  label: string
  /** The first rows of the column, in the worker's order. */
  rows: Summary[]
  /** Every live ticket of the column. */
  total: number
}

/** What the app knows besides the lanes: the newest version applied per ticket, and when tickets were deleted. */
export interface Memo {
  versions: Map<string, number>
  tombstones: Map<string, number>
  placed: Placed
}

/**
 * Tickets the app counts in a lane without drawing them (their place is past
 * the rows of a partial lane): id → lane id. The next write of such a ticket
 * leaves that lane, whatever its event says, so a count is never taken twice.
 */
export type Placed = Map<string, string>

export const newMemo = (): Memo => ({ versions: new Map(), tombstones: new Map(), placed: new Map() })

export interface WriteKind {
  /** `ticket.moved`: `order` is the ticket's position in a column the worker just renumbered. */
  moved?: boolean
  /** The ticket entered the board: `ticket.created`, `ticket.restored`. */
  entered?: boolean
  /** The status before this write, when the event's activity says so. */
  from?: string | null
}

const time = (iso: string) => {
  const t = Date.parse(iso)
  return Number.isNaN(t) ? 0 : t
}

export const partial = (lane: Lane) => lane.rows.length < lane.total

const renumber = (rows: Summary[]) => rows.map((row, i) => (row.order === i + 1 ? row : { ...row, order: i + 1 }))

export function laneOf(lanes: readonly Lane[], id: string): Lane | undefined {
  return lanes.find((lane) => lane.rows.some((row) => row.id === id))
}

export function findRow(lanes: readonly Lane[], id: string): Summary | undefined {
  for (const lane of lanes) {
    const row = lane.rows.find((r) => r.id === id)
    if (row) return row
  }
  return undefined
}

/**
 * Put one ticket where the worker has it. A moved ticket's `order` is its
 * position (the worker renumbers both columns 1..n, and so do we); any other
 * write keeps the worker's number, so the ticket goes before the first row
 * with a greater one. A ticket whose place is past the rows of a partial lane
 * is not drawn: it comes with the next page, and only the total changes
 * (`placed` remembers where it was counted).
 */
export function upsert(lanes: readonly Lane[], s: Summary, kind: WriteKind = {}, placed?: Placed): Lane[] {
  const from = laneOf(lanes, s.id)
  const into = s.deleted_at ? undefined : lanes.find((lane) => lane.id === s.status)
  if (from && from === into && !kind.moved) {
    return lanes.map((lane) =>
      lane === from ? { ...lane, rows: lane.rows.map((row) => (row.id === s.id ? s : row)) } : lane,
    )
  }
  // The column the ticket left: the lane that drew it, the lane that counted it, else what the write says.
  const left = from?.id ?? placed?.get(s.id) ?? (kind.entered ? null : (kind.from ?? s.status))
  const changesLane = left !== (into?.id ?? null)
  let drawn = false
  const next = lanes.map((lane) => {
    if (lane !== from && lane !== into && lane.id !== left) return lane
    let rows = lane === from ? lane.rows.filter((row) => row.id !== s.id) : lane.rows
    let total = lane.total
    if (changesLane && lane.id === left) total = Math.max(0, total - 1)
    if (lane === into) {
      if (changesLane) total += 1
      const wasPartial = rows.length < lane.total - (lane === from ? 1 : 0)
      let at: number
      if (kind.moved) at = Math.max(0, s.order - 1)
      else {
        const next = rows.findIndex((row) => row.order > s.order)
        at = next < 0 ? rows.length : next
        if (next < 0 && wasPartial) at = Number.POSITIVE_INFINITY
      }
      if (at <= rows.length) {
        rows = [...rows.slice(0, at), s, ...rows.slice(at)]
        drawn = true
      }
    }
    // A move renumbers its column and compacts the one it left (move_ticket), drawn or not.
    if (kind.moved && (lane === into || lane === from || lane.id === left)) rows = renumber(rows)
    return { ...lane, rows, total: Math.max(total, rows.length) }
  })
  if (into && !drawn) placed?.set(s.id, into.id)
  else placed?.delete(s.id)
  return next
}

/**
 * Apply a ticket the worker returned (an event, a write's reply) unless the
 * app already has that version or a newer one, or the ticket was deleted
 * after it was written.
 */
export function applyWrite(lanes: readonly Lane[], s: Summary, memo: Memo, kind: WriteKind = {}): Lane[] {
  const at = time(s.updated_at)
  const known = memo.versions.get(s.id)
  if (known !== undefined && known >= at) return lanes as Lane[]
  const deletedAt = memo.tombstones.get(s.id)
  if (deletedAt !== undefined && at <= deletedAt) return lanes as Lane[]
  memo.versions.set(s.id, at)
  if (s.deleted_at) memo.tombstones.set(s.id, Math.max(at, time(s.deleted_at)))
  else memo.tombstones.delete(s.id)
  return upsert(lanes, s, kind, memo.placed)
}

export interface Write {
  summary: Summary
  kind: WriteKind
}

/**
 * A board read: its lanes, without the tickets this app saw deleted after the
 * version the read holds, then the writes that arrived while it was in flight
 * replayed over it (the ones stamped after the newest write the read holds;
 * the rest it has). Its counts are the worker's: nothing is placed any more.
 */
export function applyRead(read: BoardRead, journal: readonly Write[], memo: Memo): Lane[] {
  memo.placed.clear()
  let lanes: Lane[] = read.columns.map((column) => {
    const rows = column.rows.filter((row) => {
      const deletedAt = memo.tombstones.get(row.id)
      return deletedAt === undefined || time(row.updated_at) > deletedAt
    })
    return { id: column.id, label: column.label, rows, total: column.total - (column.rows.length - rows.length) }
  })
  for (const lane of lanes) for (const row of lane.rows) memo.versions.set(row.id, time(row.updated_at))
  const readAt = time(read.as_of)
  for (const { summary, kind } of journal) {
    if (time(summary.updated_at) <= readAt) continue
    // The read replaced the lanes: a write this app already applied must be applied again over it.
    memo.versions.delete(summary.id)
    lanes = applyWrite(lanes, summary, memo, kind)
  }
  return lanes
}

/**
 * The next page of lane `status`: the rows after the ones the app has. A row
 * the app already has, or knows a newer version of (it moved meanwhile), is
 * skipped. `total` replaces the lane's unless a write changed the lane while
 * the page was out (`touched`): then the app's count is the newer one. A page
 * that adds nothing while the lane is still partial means the app's rows are
 * out of step with the column: the caller reads the board again.
 */
export function appendPage(
  lanes: readonly Lane[],
  status: string,
  rows: readonly Summary[],
  total: number,
  memo: Memo,
  touched: boolean,
): Lane[] {
  return lanes.map((lane) => {
    if (lane.id !== status) return lane
    const have = new Set(lanes.flatMap((l) => l.rows.map((row) => row.id)))
    const added = rows.filter((row) => {
      if (have.has(row.id)) return false
      const known = memo.versions.get(row.id)
      if (known !== undefined && known > time(row.updated_at)) return false
      const deletedAt = memo.tombstones.get(row.id)
      if (deletedAt !== undefined && time(row.updated_at) <= deletedAt) return false
      memo.versions.set(row.id, time(row.updated_at))
      memo.placed.delete(row.id)
      return true
    })
    const next = [...lane.rows, ...added]
    return { ...lane, rows: next, total: Math.max(touched ? lane.total : total, next.length) }
  })
}

/**
 * Where a card dropped at `index` among the visible cards goes in the column
 * as the app has it (the worker's order): before the visible card it was
 * dropped on, else right after the last visible one, else at the end. The
 * console UI's `fullIndex`.
 */
export function dropIndex(
  column: readonly Summary[],
  visible: readonly string[],
  index: number,
  dragged: string,
): number {
  const full = column.filter((row) => row.id !== dragged)
  const ids = visible.filter((id) => id !== dragged)
  const before = index < ids.length ? full.findIndex((row) => row.id === ids[index]) : -1
  if (before >= 0) return before
  const last = ids.length > 0 ? full.findIndex((row) => row.id === ids[ids.length - 1]) : -1
  return last >= 0 ? last + 1 : full.length
}

/**
 * The optimistic move: the card out of its lane and into `status` at `index`
 * (absent = the end of the column), both lanes renumbered like the worker
 * does. A card that lands past the rows of a partial lane only counts.
 */
export function placeMove(lanes: readonly Lane[], id: string, status: string, index?: number, placed?: Placed): Lane[] {
  const row = findRow(lanes, id)
  const into = lanes.find((lane) => lane.id === status)
  if (!row || !into) return lanes as Lane[]
  const end = index === undefined
  const at = end ? into.total - (row.status === status ? 1 : 0) : index
  return upsert(lanes, { ...row, status, order: at + 1 }, { moved: true }, placed)
}

/** Every row the app has, lane by lane. */
export const allRows = (lanes: readonly Lane[]): Summary[] => lanes.flatMap((lane) => lane.rows)

/** Every lane has all of its rows: the filters and the subtask counts see the whole board. */
export const isComplete = (lanes: readonly Lane[]): boolean => lanes.every((lane) => !partial(lane))

/** Parent id → its children on the board: done ones (in `done`), and all but the canceled. */
export function subtaskStats(
  rows: readonly Summary[],
  done: string,
  canceled: string | null,
): Map<string, { done: number; total: number }> {
  const stats = new Map<string, { done: number; total: number }>()
  for (const row of rows) {
    if (!row.parent_id) continue
    const s = stats.get(row.parent_id) ?? { done: 0, total: 0 }
    if (row.status !== canceled) s.total += 1
    if (row.status === done) s.done += 1
    stats.set(row.parent_id, s)
  }
  return stats
}
