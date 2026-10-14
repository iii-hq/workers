import type { TicketSummary } from './shared'

/**
 * Board filters, shared by the board and the list layouts. Everything here
 * is pure so the facet counts and saved views stay testable without React.
 * Check: `node filters.check.ts` (or `pnpm check`) from kanban/ui.
 */

export type UpdatedWindow = 'any' | 'visit' | 'today' | 'week'

export type Filters = {
  /** Key (`KAN-12`, `12`, `#12`) or title substring. */
  q: string
  labels: string[]
  /** `any`: the ticket has at least one of `labels`; `all`: every one. */
  labelMode: 'any' | 'all'
  /** Agent profile ids; `''` matches unassigned tickets. */
  assignees: string[]
  priorities: string[]
  updated: UpdatedWindow
}

export type FilterKey = 'q' | 'labels' | 'assignees' | 'priorities' | 'updated'

/**
 * `lastVisit` is the ISO time the board was last opened in this browser (an
 * unparseable value counts as no visit). `own` maps a ticket id to the
 * `updated_at` of the newest write this browser tab made to it: a ticket whose
 * `updated_at` is not past that write is the user's own edit, not a change.
 */
export type MatchContext = { lastVisit: string | null; now: number; own?: ReadonlyMap<string, string> }

export type SavedView = { id: string; name: string; filters: Filters }

export const EMPTY_FILTERS: Filters = {
  q: '',
  labels: [],
  labelMode: 'any',
  assignees: [],
  priorities: [],
  updated: 'any',
}

export const ALL_VIEW: SavedView = { id: 'all', name: 'All tickets', filters: EMPTY_FILTERS }

export const UPDATED_WINDOWS: { id: UpdatedWindow; label: string; short: string }[] = [
  { id: 'any', label: 'Any time', short: '' },
  { id: 'visit', label: 'Since your last visit', short: 'since last visit' },
  { id: 'today', label: 'Today', short: 'today' },
  { id: 'week', label: 'Last 7 days', short: 'last 7 days' },
]

const DAY = 24 * 60 * 60 * 1000

/** A stored last visit, or null when it is missing or not a date. */
export function validVisit(raw: string | null | undefined): string | null {
  return raw && !Number.isNaN(Date.parse(raw)) ? raw : null
}

/** The cutoff in ms for a window, or null when it filters nothing. */
export function updatedSince(window: UpdatedWindow, ctx: MatchContext): number | null {
  switch (window) {
    case 'visit': {
      const visit = validVisit(ctx.lastVisit)
      return visit ? Date.parse(visit) : null
    }
    case 'today': {
      const start = new Date(ctx.now)
      start.setHours(0, 0, 0, 0)
      return start.getTime()
    }
    case 'week':
      return ctx.now - 7 * DAY
    default:
      return null
  }
}

/** The ticket is exactly what this tab last wrote; a later edit by someone else is not. */
function isOwnWrite(ticket: TicketSummary, ctx: MatchContext): boolean {
  const wrote = ctx.own?.get(ticket.id)
  return wrote !== undefined && Date.parse(wrote) >= Date.parse(ticket.updated_at)
}

/** Changed by someone else after the last visit (false on a first visit). */
export function isChangedSinceVisit(ticket: TicketSummary, ctx: MatchContext): boolean {
  const since = updatedSince('visit', ctx)
  return since !== null && Date.parse(ticket.updated_at) > since && !isOwnWrite(ticket, ctx)
}

/** Updated after `since` (`visit` is strict and skips the user's own edits; the other windows include the cutoff). */
function updatedAfter(ticket: TicketSummary, window: UpdatedWindow, since: number, ctx: MatchContext): boolean {
  const at = Date.parse(ticket.updated_at)
  return window === 'visit' ? at > since && !isOwnWrite(ticket, ctx) : at >= since
}

/** Lower case without accents, so "acentuacao" finds "acentuação". */
function fold(text: string): string {
  return text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
}

function matchesQuery(ticket: TicketSummary, raw: string): boolean {
  const q = fold(raw.trim())
  if (!q) return true
  const key = ticket.key.toLowerCase()
  if (key.includes(q) || fold(ticket.title).includes(q)) return true
  const digits = q.replace(/^#/, '')
  const number = key.split('-').pop() ?? ''
  return /^\d+$/.test(digits) && number.startsWith(digits)
}

/** The match mode that applies: with a single label `all` and `any` agree. */
function labelMode(f: Filters): 'any' | 'all' {
  return new Set(f.labels).size > 1 ? f.labelMode : 'any'
}

/**
 * Whether `ticket` passes every filter except `skip` (used for facet counts).
 * Choosing one more label narrows the result in `all` mode, so its facet
 * counts keep the label filter; in `any` mode it widens, so the facet skips it.
 */
export function matches(ticket: TicketSummary, f: Filters, ctx: MatchContext, skip?: FilterKey): boolean {
  if (skip !== 'q' && !matchesQuery(ticket, f.q)) return false
  if (f.labels.length > 0 && (skip !== 'labels' || labelMode(f) === 'all')) {
    const has = (label: string) => ticket.labels.includes(label)
    if (labelMode(f) === 'all' ? !f.labels.every(has) : !f.labels.some(has)) return false
  }
  if (skip !== 'assignees' && f.assignees.length > 0 && !f.assignees.includes(ticket.assignee ?? '')) return false
  if (skip !== 'priorities' && f.priorities.length > 0 && !f.priorities.includes(ticket.priority)) return false
  if (skip !== 'updated') {
    const since = updatedSince(f.updated, ctx)
    if (since !== null && !updatedAfter(ticket, f.updated, since, ctx)) return false
  }
  return true
}

export function applyFilters(tickets: TicketSummary[], f: Filters, ctx: MatchContext): TicketSummary[] {
  return tickets.filter((ticket) => matches(ticket, f, ctx))
}

/**
 * How many tickets each option would keep, given every other active filter:
 * the number shown next to each choice in a filter menu. `updated` counts by
 * window id; `assignees` uses `''` for unassigned.
 */
export function facetCounts(
  tickets: TicketSummary[],
  f: Filters,
  ctx: MatchContext,
  key: 'labels' | 'assignees' | 'priorities' | 'updated',
): Record<string, number> {
  // No prototype: a label named `constructor` or `__proto__` is just a key.
  const counts: Record<string, number> = Object.create(null)
  const bump = (value: string) => {
    counts[value] = (counts[value] ?? 0) + 1
  }
  for (const ticket of tickets) {
    if (!matches(ticket, f, ctx, key)) continue
    if (key === 'labels') new Set(ticket.labels).forEach(bump)
    else if (key === 'assignees') bump(ticket.assignee ?? '')
    else if (key === 'priorities') bump(ticket.priority)
    else {
      for (const window of UPDATED_WINDOWS) {
        const since = updatedSince(window.id, ctx)
        if (since === null ? window.id === 'any' : updatedAfter(ticket, window.id, since, ctx)) bump(window.id)
      }
    }
  }
  return counts
}

/** Every label on the board, most used first. */
export function allLabels(tickets: TicketSummary[]): string[] {
  const counts = new Map<string, number>()
  for (const ticket of tickets) for (const label of ticket.labels) counts.set(label, (counts.get(label) ?? 0) + 1)
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([label]) => label)
}

export function isEmpty(f: Filters): boolean {
  return sameFilters(f, EMPTY_FILTERS)
}

function normalized(f: Filters): string {
  const sorted = (values: string[]) => [...new Set(values)].sort()
  return JSON.stringify([
    fold(f.q.trim()),
    sorted(f.labels),
    labelMode(f),
    sorted(f.assignees),
    sorted(f.priorities),
    f.updated,
  ])
}

/** Equal as filters: order or repeats in a selection, case and an unused match mode do not count. */
export function sameFilters(a: Filters, b: Filters): boolean {
  return normalized(a) === normalized(b)
}

/** Read stored filters defensively: localStorage may hold an older shape. */
export function coerceFilters(value: unknown): Filters {
  const v = (value && typeof value === 'object' ? value : {}) as Partial<Record<keyof Filters, unknown>>
  const strings = (x: unknown) => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string') : [])
  const windows = UPDATED_WINDOWS.map((w) => w.id) as string[]
  return {
    q: typeof v.q === 'string' ? v.q : '',
    labels: strings(v.labels),
    labelMode: v.labelMode === 'all' ? 'all' : 'any',
    assignees: strings(v.assignees),
    priorities: strings(v.priorities),
    updated: windows.includes(v.updated as string) ? (v.updated as UpdatedWindow) : 'any',
  }
}

export function coerceViews(value: unknown): SavedView[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<string>()
  return value
    .filter(
      (v) =>
        v && typeof v === 'object' && typeof v.id === 'string' && typeof v.name === 'string' && v.id !== ALL_VIEW.id,
    )
    .filter((v) => !seen.has(v.id) && seen.add(v.id))
    .map((v) => ({ id: v.id, name: v.name, filters: coerceFilters(v.filters) }))
}
