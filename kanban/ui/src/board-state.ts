import type { BoardColumn, BoardData, GlyphTone, ProjectView, ReleaseView, TicketSummary } from './shared'

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

/* ── swimlanes ────────────────────────────────────────────────────────── */

export type GroupBy = 'none' | 'release' | 'project' | 'parent'

export const GROUPS: { id: GroupBy; label: string }[] = [
  { id: 'none', label: 'None' },
  { id: 'release', label: 'Release' },
  { id: 'project', label: 'Project' },
  { id: 'parent', label: 'Parent' },
]

/** A stored choice, or `none` when it is missing or from a future version. */
export function coerceGroup(value: unknown): GroupBy {
  return GROUPS.some((group) => group.id === value) ? (value as GroupBy) : 'none'
}

export type LaneFacet = 'release' | 'project' | 'parent'

export const REFUSE_RELEASED = "Released releases don't take tickets"
export const REFUSE_ARCHIVED = "Archived projects don't take tickets"
export const REFUSE_OWN_PARENT = "A ticket can't be its own parent"
export const REFUSE_HAS_SUBTASKS = "A ticket with subtasks can't become one"

export type Swimlane = {
  /** Stable across renders and sessions: the key of its collapsed state. */
  id: string
  facet: LaneFacet
  /** Release or project id, parent ticket id; `null` for the "No …" lane and the released group. */
  value: string | null
  kind: 'entity' | 'none' | 'group'
  /** Heading: the release or project name, the parent's title, "No release". */
  name: string
  /** The same lane inside a sentence: the parent's key instead of its title. */
  short: string
  state: 'planned' | 'active' | 'released' | 'archived' | null
  /** `YYYY-MM-DD`, releases only. */
  target: string | null
  color: GlyphTone | null
  parent: TicketSummary | null
  /** Why a drop here is refused (the lane is a released release, an archived project), if it is. */
  refuse: string | null
  /** Tickets that pass the filters, per status, in the order of the column. */
  cells: Map<string, TicketSummary[]>
  /** Tickets that pass the filters. */
  count: number
  /** Every live ticket of the lane, whatever the filters say, and how many of them are in the done status. */
  total: number
  done: number
  /** `group` only: the released releases, collapsed under one heading. */
  lanes: Swimlane[]
}

export type SwimlaneInput = {
  by: Exclude<GroupBy, 'none'>
  columns: BoardColumn[]
  /** Ids that pass the filters. */
  shownIds: ReadonlySet<string>
  releases: readonly ReleaseView[]
  projects: readonly ProjectView[]
  doneStatus: string
  /** Keep the swimlanes that have no ticket in the current filter. */
  showEmpty: boolean
}

const natural = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' })

/** Released ⇔ `released_at`; active ⇔ some live ticket outside the first column; planned otherwise (as the worker derives it). */
export function releaseState(
  release: { released_at: string | null },
  tickets: readonly TicketSummary[],
  firstStatus: string | undefined,
): 'planned' | 'active' | 'released' {
  if (release.released_at) return 'released'
  return tickets.some((ticket) => ticket.status !== firstStatus) ? 'active' : 'planned'
}

/** The id of the facet a ticket sits in (`null` = none). */
export function laneValueOf(ticket: TicketSummary, facet: LaneFacet): string | null {
  return (facet === 'release' ? ticket.release_id : facet === 'project' ? ticket.project_id : ticket.parent_id) ?? null
}

const keyNumber = (key: string) => Number(key.split('-').pop())

/** The ids a swimlane can stand for: the releases and projects the board lists, the tickets that have a child on it. */
export function knownLaneIds(
  by: LaneFacet,
  tickets: readonly TicketSummary[],
  releases: readonly ReleaseView[],
  projects: readonly ProjectView[],
): Set<string> {
  if (by === 'release') return new Set(releases.map((r) => r.id))
  if (by === 'project') return new Set(projects.map((p) => p.id))
  const ids = new Set(tickets.map((ticket) => ticket.id))
  return new Set(tickets.flatMap((ticket) => (ticket.parent_id && ids.has(ticket.parent_id) ? [ticket.parent_id] : [])))
}

/** The `value` of the swimlane a ticket is drawn in; one the board does not know (a deleted parent) is `''`: "No …". */
const laneKey = (ticket: TicketSummary, by: LaneFacet, known: ReadonlySet<string>) => {
  const value = laneValueOf(ticket, by)
  return value && known.has(value) ? value : ''
}

/** The id of the swimlane a ticket is drawn in. */
export const laneIdOf = (ticket: TicketSummary, by: LaneFacet, known: ReadonlySet<string>) =>
  `${by}:${laneKey(ticket, by, known)}`

/**
 * What dropping a card in another swimlane writes: its release, project or parent (`value` null = none), `name`
 * for the sentence. Nothing when the card is already drawn in that swimlane, even if the id it names is not
 * the lane's (a deleted parent sits in "No parent": a drop there must not clear it).
 */
export function laneChange(
  ticket: TicketSummary,
  lane: Swimlane,
  known: ReadonlySet<string>,
): { key: LaneFacet; value: string | null; name: string } | undefined {
  if (lane.kind === 'group' || laneIdOf(ticket, lane.facet, known) === lane.id) return undefined
  return { key: lane.facet, value: lane.value, name: lane.short }
}

/**
 * Why `ticket`, drawn in the swimlane `from`, cannot be dropped in `lane` (null = it can): the swimlane refuses
 * tickets, or the worker would refuse the parent (its own, or a ticket with subtasks: one level deep). A card
 * can always stay in the swimlane it is in, so it can still move to another status there.
 */
export function dropRefusal(lane: Swimlane, ticket: TicketSummary, from: string, hasSubtasks: boolean): string | null {
  if (lane.id === from || lane.lanes.some((member) => member.id === from)) return null
  if (lane.refuse) return lane.refuse
  if (lane.facet !== 'parent' || lane.kind !== 'entity') return null
  if (lane.value === ticket.id) return REFUSE_OWN_PARENT
  return hasSubtasks ? REFUSE_HAS_SUBTASKS : null
}

/**
 * The board as swimlanes. Every live ticket lands in exactly one lane: a
 * ticket that names a release, project or parent the board does not know (a
 * deleted parent, a release created a second ago) sits in the "No …" lane
 * instead of vanishing. Lanes with no ticket in the current filter are
 * dropped unless `showEmpty`.
 *
 * Release: the worker's order (target date, then name), "No release", then the
 * released ones under one `group` lane. Project: active by name, "No project",
 * then the archived. Parent: tickets that have live children, by number, then
 * "No parent" (the parents themselves are in it).
 */
export function buildSwimlanes(input: SwimlaneInput): Swimlane[] {
  const { by, columns, shownIds, releases, projects, doneStatus, showEmpty } = input
  const all = columns.flatMap((column) => column.tickets)
  const byId = new Map(all.map((ticket) => [ticket.id, ticket]))
  const known = knownLaneIds(by, all, releases, projects)

  const members = new Map<string, TicketSummary[]>()
  const lane = (
    value: string | null,
    id: string,
    over: Partial<Swimlane> & Pick<Swimlane, 'kind' | 'name'>,
  ): Swimlane => ({
    id,
    facet: by,
    value,
    short: over.name,
    state: null,
    target: null,
    color: null,
    parent: null,
    refuse: null,
    cells: new Map(),
    count: 0,
    total: 0,
    done: 0,
    lanes: [],
    ...over,
  })
  const fill = (target: Swimlane, tickets: readonly TicketSummary[]) => {
    for (const column of columns) {
      for (const ticket of column.tickets) {
        if (!tickets.includes(ticket)) continue
        target.total += 1
        if (ticket.status === doneStatus) target.done += 1
        if (!shownIds.has(ticket.id)) continue
        target.count += 1
        target.cells.set(column.id, [...(target.cells.get(column.id) ?? []), ticket])
      }
    }
    return target
  }

  for (const ticket of all) {
    const key = laneKey(ticket, by, known)
    members.set(key, [...(members.get(key) ?? []), ticket])
  }
  const of = (key: string) => members.get(key) ?? []
  const keep = (l: Swimlane) => showEmpty || l.count > 0
  const first = columns[0]?.id

  const none = (name: string) => fill(lane(null, `${by}:`, { kind: 'none', name }), of(''))

  if (by === 'release') {
    const made = releases.map((release) =>
      fill(
        lane(release.id, `release:${release.id}`, {
          kind: 'entity',
          name: release.name,
          state: releaseState(release, of(release.id), first),
          target: release.target_at,
          refuse: release.released_at ? REFUSE_RELEASED : null,
        }),
        of(release.id),
      ),
    )
    const open = made.filter((l) => l.state !== 'released' && keep(l))
    const released = made.filter((l) => l.state === 'released' && keep(l))
    const out = [...open]
    const nothing = none('No release')
    if (keep(nothing)) out.push(nothing)
    if (released.length > 0) {
      const group = lane(null, 'release:released', {
        kind: 'group',
        name: `Released · ${released.length}`,
        refuse: REFUSE_RELEASED,
        lanes: released,
      })
      group.count = released.reduce((n, l) => n + l.count, 0)
      group.total = released.reduce((n, l) => n + l.total, 0)
      group.done = released.reduce((n, l) => n + l.done, 0)
      out.push(group)
    }
    return out
  }

  if (by === 'project') {
    const made = [...projects]
      .sort((a, b) => Number(Boolean(a.archived_at)) - Number(Boolean(b.archived_at)) || natural(a.name, b.name))
      .map((project) =>
        fill(
          lane(project.id, `project:${project.id}`, {
            kind: 'entity',
            name: project.name,
            state: project.archived_at ? 'archived' : null,
            color: project.color,
            refuse: project.archived_at ? REFUSE_ARCHIVED : null,
          }),
          of(project.id),
        ),
      )
      .filter(keep)
    const nothing = none('No project')
    const at = made.findIndex((l) => l.state === 'archived')
    const out = [...made]
    if (keep(nothing)) out.splice(at < 0 ? out.length : at, 0, nothing)
    return out
  }

  const made = [...known]
    .map((id) => byId.get(id) as TicketSummary)
    .sort((a, b) => keyNumber(a.key) - keyNumber(b.key) || natural(a.key, b.key))
    .map((parent) =>
      fill(
        lane(parent.id, `parent:${parent.id}`, { kind: 'entity', name: parent.title, short: parent.key, parent }),
        of(parent.id),
      ),
    )
    .filter(keep)
  const nothing = none('No parent')
  return keep(nothing) ? [...made, nothing] : made
}

/** The lanes with the released group opened up: every lane a card can sit in. */
export function flattenLanes(lanes: readonly Swimlane[]): Swimlane[] {
  return lanes.flatMap((l) => (l.kind === 'group' ? [l, ...l.lanes] : [l]))
}

/** Subtasks on the board per parent id: how many, how many in the done status. */
export function subtaskStats(
  tickets: readonly TicketSummary[],
  doneStatus: string,
): Map<string, { done: number; total: number }> {
  const stats = new Map<string, { done: number; total: number }>()
  for (const ticket of tickets) {
    if (!ticket.parent_id) continue
    const s = stats.get(ticket.parent_id) ?? { done: 0, total: 0 }
    s.total += 1
    if (ticket.status === doneStatus) s.done += 1
    stats.set(ticket.parent_id, s)
  }
  return stats
}

/** Cards a swimlane cell draws before "Show N more"; a cell that would hide a single card draws it instead. */
export const CELL_LIMIT = 2

export function cellWindow(count: number, expanded: boolean): { shown: number; hidden: number } {
  if (expanded || count <= CELL_LIMIT + 1) return { shown: count, hidden: 0 }
  return { shown: CELL_LIMIT, hidden: count - CELL_LIMIT }
}

/**
 * Where a card dropped among `neighbours` (the ids drawn in the target cell,
 * in order) goes in the whole column, which also holds the tickets the cell
 * does not draw: before the neighbour at `index`, or after the last one. An
 * empty cell has no neighbour: the end of the column.
 */
export function fullIndex(
  column: readonly TicketSummary[],
  neighbours: readonly string[],
  index: number,
  draggedId: string,
): number {
  const full = column.filter((ticket) => ticket.id !== draggedId)
  const ids = neighbours.filter((id) => id !== draggedId)
  const before = index < ids.length ? full.findIndex((ticket) => ticket.id === ids[index]) : -1
  if (before >= 0) return before
  const last = ids.length > 0 ? full.findIndex((ticket) => ticket.id === ids[ids.length - 1]) : -1
  return last >= 0 ? last + 1 : full.length
}

/** `2026-10-30` as "Oct 30" (the year follows when it is not the current one). */
export function shortDay(day: string, now: number): string {
  const [year, month, date] = day.split('-').map(Number)
  const at = new Date(year, month - 1, date)
  if (Number.isNaN(at.getTime())) return day
  const options: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' }
  if (year !== new Date(now).getFullYear()) options.year = 'numeric'
  return at.toLocaleDateString('en', options)
}
