/**
 * The board filters (design §8.2, §8.6), the console UI's ui/src/filters.ts
 * without the saved views: the same shape (so a stored value reads the same),
 * the same matching and facet counts. The text filter is a plain substring of
 * the key or the title, never a regex (SDK README › Pitfalls).
 */

import type { Summary } from './api'

export type UpdatedWindow = 'any' | 'visit' | 'today' | 'week'

export interface Filters {
  /** Key (`KAN-12`, `12`, `#12`) or title substring. */
  q: string
  labels: string[]
  /** `any`: at least one of `labels`; `all`: every one. */
  labelMode: 'any' | 'all'
  /** Agent profile ids; `''` = unassigned. */
  assignees: string[]
  priorities: string[]
  updated: UpdatedWindow
  /** Project ids; `''` = no project. */
  projects: string[]
  /** Release ids; `''` = no release. */
  releases: string[]
}

export type FacetKey = 'labels' | 'assignees' | 'priorities' | 'updated' | 'projects' | 'releases'
type FilterKey = 'q' | FacetKey

/** `lastVisit`: when the board was last seen here; `own`: ticket id → the `updated_at` of this app's last write to it. */
export interface MatchContext {
  lastVisit: string | null
  now: number
  own?: ReadonlyMap<string, string>
}

export const EMPTY_FILTERS: Filters = {
  q: '',
  labels: [],
  labelMode: 'any',
  assignees: [],
  priorities: [],
  updated: 'any',
  projects: [],
  releases: [],
}

export const UPDATED_WINDOWS: { id: UpdatedWindow; label: string }[] = [
  { id: 'any', label: 'Any time' },
  { id: 'visit', label: 'Since last visit' },
  { id: 'today', label: 'Today' },
  { id: 'week', label: '7 days' },
]

const DAY = 24 * 60 * 60 * 1000

/** A stored last visit, or null when it is missing or not a date. */
export function validVisit(raw: unknown): string | null {
  return typeof raw === 'string' && raw && !Number.isNaN(Date.parse(raw)) ? raw : null
}

/** The cutoff in ms of a window, or null when it filters nothing. */
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

/** The ticket is exactly what this app last wrote; a later edit by someone else is not. */
function isOwnWrite(ticket: Summary, ctx: MatchContext): boolean {
  const wrote = ctx.own?.get(ticket.id)
  return wrote !== undefined && Date.parse(wrote) >= Date.parse(ticket.updated_at)
}

/** Changed by someone else after the last visit (false on a first visit). */
export function isChangedSinceVisit(ticket: Summary, ctx: MatchContext): boolean {
  const since = updatedSince('visit', ctx)
  return since !== null && Date.parse(ticket.updated_at) > since && !isOwnWrite(ticket, ctx)
}

function updatedAfter(ticket: Summary, window: UpdatedWindow, since: number, ctx: MatchContext): boolean {
  const at = Date.parse(ticket.updated_at)
  return window === 'visit' ? at > since && !isOwnWrite(ticket, ctx) : at >= since
}

/** Lower case without accents: "acentuacao" finds "acentuação". */
export function fold(text: string): string {
  return text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
}

const DIGITS = /^\d+$/

function matchesQuery(ticket: Summary, raw: string): boolean {
  if (!raw) return true
  const q = fold(raw.trim())
  if (!q) return true
  const key = ticket.key.toLowerCase()
  if (key.includes(q) || fold(ticket.title).includes(q)) return true
  const digits = q.startsWith('#') ? q.slice(1) : q
  const number = key.split('-').pop() ?? ''
  return DIGITS.test(digits) && number.startsWith(digits)
}

function labelMode(f: Filters): 'any' | 'all' {
  return new Set(f.labels).size > 1 ? f.labelMode : 'any'
}

/** Whether `ticket` passes every filter but `skip` (the facet counts skip their own). */
export function matches(ticket: Summary, f: Filters, ctx: MatchContext, skip?: FilterKey): boolean {
  if (skip !== 'q' && !matchesQuery(ticket, f.q)) return false
  if (f.labels.length > 0 && (skip !== 'labels' || labelMode(f) === 'all')) {
    const has = (label: string) => ticket.labels.includes(label)
    if (labelMode(f) === 'all' ? !f.labels.every(has) : !f.labels.some(has)) return false
  }
  if (skip !== 'assignees' && f.assignees.length > 0 && !f.assignees.includes(ticket.assignee ?? '')) return false
  if (skip !== 'priorities' && f.priorities.length > 0 && !f.priorities.includes(ticket.priority)) return false
  if (skip !== 'projects' && f.projects.length > 0 && !f.projects.includes(ticket.project_id ?? '')) return false
  if (skip !== 'releases' && f.releases.length > 0 && !f.releases.includes(ticket.release_id ?? '')) return false
  if (skip !== 'updated') {
    const since = updatedSince(f.updated, ctx)
    if (since !== null && !updatedAfter(ticket, f.updated, since, ctx)) return false
  }
  return true
}

/** How many tickets each option would keep, given the other filters: the count beside each chip. */
export function facetCounts(
  tickets: readonly Summary[],
  f: Filters,
  ctx: MatchContext,
  key: FacetKey,
): Map<string, number> {
  const counts = new Map<string, number>()
  const bump = (value: string) => counts.set(value, (counts.get(value) ?? 0) + 1)
  for (const ticket of tickets) {
    if (!matches(ticket, f, ctx, key)) continue
    if (key === 'labels') new Set(ticket.labels).forEach(bump)
    else if (key === 'assignees') bump(ticket.assignee ?? '')
    else if (key === 'priorities') bump(ticket.priority)
    else if (key === 'projects') bump(ticket.project_id ?? '')
    else if (key === 'releases') bump(ticket.release_id ?? '')
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
export function allLabels(tickets: readonly Summary[]): string[] {
  const counts = new Map<string, number>()
  for (const ticket of tickets) for (const label of ticket.labels) counts.set(label, (counts.get(label) ?? 0) + 1)
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)).map(([l]) => l)
}

export function isEmpty(f: Filters): boolean {
  return (
    !f.q.trim() &&
    f.updated === 'any' &&
    !f.labels.length &&
    !f.assignees.length &&
    !f.priorities.length &&
    !f.projects.length &&
    !f.releases.length
  )
}

/** How many filters are on (the text counts as one). */
export function activeCount(f: Filters): number {
  return (
    (f.q.trim() ? 1 : 0) +
    (f.updated === 'any' ? 0 : 1) +
    f.labels.length +
    f.assignees.length +
    f.priorities.length +
    f.projects.length +
    f.releases.length
  )
}

/** Read stored filters defensively: storage may hold an older shape or garbage. */
export function coerceFilters(value: unknown): Filters {
  const v = (value && typeof value === 'object' ? value : {}) as Partial<Record<keyof Filters, unknown>>
  const strings = (x: unknown) => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string') : [])
  return {
    q: typeof v.q === 'string' ? v.q : '',
    labels: strings(v.labels),
    labelMode: v.labelMode === 'all' ? 'all' : 'any',
    assignees: strings(v.assignees),
    priorities: strings(v.priorities),
    updated: UPDATED_WINDOWS.some((w) => w.id === v.updated) ? (v.updated as UpdatedWindow) : 'any',
    projects: strings(v.projects),
    releases: strings(v.releases),
  }
}

/** `values` with `value` added, or taken out when it is there. */
export function toggled(values: readonly string[], value: string): string[] {
  return values.includes(value) ? values.filter((v) => v !== value) : [...values, value]
}
