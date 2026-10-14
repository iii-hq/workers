import type { Column, ProjectView, ReleaseView, TicketSummary } from './shared'

/**
 * Pure rules of the Releases and Projects pages and of "Create from a label…":
 * sorting, grouping, label families, the next release, dates. No React, so
 * they are checkable without a browser.
 * Check: `node plan.check.ts` from kanban/ui.
 */

/** The server compares names and labels with ASCII case ignored; so do we. */
export const asciiLower = (text: string) => text.replace(/[A-Z]/g, (char) => char.toLowerCase())

/** `Phase 2` before `Phase 10`. */
export const naturalCompare = (a: string, b: string) => a.localeCompare(b, 'en', { numeric: true, sensitivity: 'base' })

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`

/** An entity by id, else by name (trimmed, ASCII case ignored), the way the worker resolves a ref. */
export function resolveEntity<T extends { id: string; name: string }>(
  list: readonly T[],
  ref: string | null | undefined,
): T | undefined {
  const text = ref?.trim()
  if (!text) return undefined
  const lower = asciiLower(text)
  return list.find((item) => item.id === text) ?? list.find((item) => asciiLower(item.name.trim()) === lower)
}

/* ── releases ─────────────────────────────────────────────────────────── */

/** Target date first (no date last), then natural name: the worker's order. */
export function compareReleases(
  a: { name: string; target_at: string | null },
  b: { name: string; target_at: string | null },
): number {
  if (a.target_at !== b.target_at) {
    if (!a.target_at) return 1
    if (!b.target_at) return -1
    return a.target_at < b.target_at ? -1 : 1
  }
  return naturalCompare(a.name, b.name)
}

export type ReleaseGroups = { active: ReleaseView[]; planned: ReleaseView[]; released: ReleaseView[] }

/** The sidebar groups: Active and Planned by target, Released newest first. */
export function groupReleases(releases: readonly ReleaseView[]): ReleaseGroups {
  const sorted = [...releases].sort(compareReleases)
  const released = sorted
    .filter((release) => release.status === 'released')
    .sort((a, b) => {
      const [x, y] = [a.released_at ?? '', b.released_at ?? '']
      return x === y ? compareReleases(a, b) : x < y ? 1 : -1
    })
  return {
    active: sorted.filter((release) => release.status === 'active'),
    planned: sorted.filter((release) => release.status === 'planned'),
    released,
  }
}

/** Where not-done tickets can go when `currentId` ships: any other release not yet released, in order. */
export function shipTargets<
  T extends { id: string; name: string; target_at: string | null; released_at: string | null },
>(releases: readonly T[], currentId: string): T[] {
  return [...releases].sort(compareReleases).filter((release) => release.id !== currentId && !release.released_at)
}

/**
 * The next release after `currentId` in the worker's order (dated ones first); when it is the last one, the first
 * of the others.
 */
export function nextRelease<
  T extends { id: string; name: string; target_at: string | null; released_at: string | null },
>(releases: readonly T[], currentId: string): T | null {
  const sorted = [...releases].sort(compareReleases)
  const at = sorted.findIndex((release) => release.id === currentId)
  const targets = shipTargets(releases, currentId)
  return targets.find((release) => sorted.indexOf(release) > at) ?? targets[0] ?? null
}

/** "Next by target date" is only true of a dated release that comes after the current one's own date. */
export const isNextByDate = (target: { target_at: string | null }, current: { target_at: string | null }) =>
  Boolean(target.target_at && current.target_at && target.target_at > current.target_at)

/**
 * `Active`, `Released Oct 12`, `Released Oct 12 · 1 reopened`: the status word, the day it shipped, and the
 * tickets that came back after the release.
 */
export function releaseStatusText(
  release: Pick<ReleaseView, 'status' | 'reopened'> & { released_at?: string | null },
  today: Date = new Date(),
): string {
  const word = release.status.charAt(0).toUpperCase() + release.status.slice(1)
  if (release.status !== 'released') return word
  const day = release.released_at ? formatStamp(release.released_at, today) : ''
  const shipped = day ? `${word} ${day}` : word
  return release.reopened > 0 ? `${shipped} · ${release.reopened} reopened` : shipped
}

/** Live tickets of an entity that are not in the done column. */
export const notDone = (entity: { total: number; done: number }) => entity.total - entity.done

/** Not-done tickets first when sorting by progress: most advanced column first, then by key. */
export function byProgress(columns: readonly Column[]) {
  const rank = (status: string) => columns.findIndex((column) => column.id === status)
  return (a: TicketSummary, b: TicketSummary) => rank(b.status) - rank(a.status) || naturalCompare(a.key, b.key)
}

/* ── labels ───────────────────────────────────────────────────────────── */

export type LabelCount = { label: string; count: number }

/** Live tickets per label, one entry per label whatever its case (the first spelling wins), by natural name. */
export function labelCounts(tickets: readonly TicketSummary[]): LabelCount[] {
  const seen = new Map<string, LabelCount>()
  for (const ticket of tickets) {
    if (ticket.deleted_at) continue
    const own = new Set<string>()
    for (const label of ticket.labels ?? []) {
      const key = asciiLower(label)
      if (own.has(key)) continue
      own.add(key)
      const entry = seen.get(key)
      if (entry) entry.count += 1
      else seen.set(key, { label, count: 1 })
    }
  }
  return [...seen.values()].sort((a, b) => naturalCompare(a.label, b.label))
}

export function ticketsWithLabel(tickets: readonly TicketSummary[], label: string): TicketSummary[] {
  const key = asciiLower(label)
  return tickets.filter((ticket) => !ticket.deleted_at && (ticket.labels ?? []).some((own) => asciiLower(own) === key))
}

/** Labels beside the one an entity came from, most used first. */
export function topLabels(tickets: readonly TicketSummary[], exclude: string | null, limit = 8): LabelCount[] {
  const skip = exclude ? asciiLower(exclude) : null
  return labelCounts(tickets)
    .filter((entry) => asciiLower(entry.label) !== skip)
    .sort((a, b) => b.count - a.count || naturalCompare(a.label, b.label))
    .slice(0, limit)
}

const FAMILY = /^(.*[-_ ])(\d+)$/

/**
 * The labels that share `label`'s stem and end in a number after a separator (`phase-0` … `phase-6`), in natural
 * order, the label included. A number glued to a word (`ipv4`, `es2020`, `h264`) is part of its name, not a
 * sequence. Empty when it has no siblings: a family needs at least two.
 */
export function labelFamily(label: string, labels: readonly string[]): string[] {
  const own = FAMILY.exec(label)
  if (!own) return []
  const stem = asciiLower(own[1])
  const family = labels.filter((other) => {
    const match = FAMILY.exec(other)
    return match !== null && asciiLower(match[1]) === stem
  })
  return family.length > 1 ? family.sort(naturalCompare) : []
}

/** A first name for what a label becomes: `phase-2` → `Phase 2`, `iii-cloud` → `iii cloud`. */
export function suggestName(label: string): string {
  const words = label.replace(/[-_]+/g, ' ').trim()
  return /\d$/.test(label) ? words.charAt(0).toUpperCase() + words.slice(1) : words
}

/** Tickets that carry the label but are in no release: what "Add" in the drift nudge would put in. */
export function driftTickets(
  release: { source_label: string | null },
  tickets: readonly TicketSummary[],
): TicketSummary[] {
  if (!release.source_label) return []
  return ticketsWithLabel(tickets, release.source_label).filter((ticket) => !ticket.release_id)
}

export type Buckets = { added: string[]; already_here: string[]; in_other: string[]; deleted: string[] }

/** What a run puts in: the new ones, plus the ones from another entity when "Move them here" is on. */
export const willAdd = (buckets: Buckets, replace: boolean) =>
  buckets.added.length + (replace ? buckets.in_other.length : 0)

/** `26 Backlog · 28 Done`: where a set of tickets stands, in column order. */
export function columnSummary(tickets: readonly Pick<TicketSummary, 'status'>[], columns: readonly Column[]): string {
  return columns
    .map((column) => ({ column, n: tickets.filter((ticket) => ticket.status === column.id).length }))
    .filter(({ n }) => n > 0)
    .map(({ column, n }) => `${n} ${column.label}`)
    .join(' · ')
}

/** `Phase 0 and Phase 1`, `A, B and 2 more`. */
export function nameList(names: readonly string[], shown = 2): string {
  if (names.length <= shown) return names.join(' and ')
  return `${names.slice(0, shown).join(', ')} and ${names.length - shown} more`
}

/* ── projects ─────────────────────────────────────────────────────────── */

/** Active projects first, each group by natural name. */
export function groupProjects(projects: readonly ProjectView[]): { active: ProjectView[]; archived: ProjectView[] } {
  const sorted = [...projects].sort((a, b) => naturalCompare(a.name, b.name))
  return {
    active: sorted.filter((project) => project.status !== 'archived'),
    archived: sorted.filter((project) => project.status === 'archived'),
  }
}

/* ── dates ────────────────────────────────────────────────────────────── */

/** A real calendar day written `YYYY-MM-DD`. */
export function isIsoDay(text: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
  if (!match) return false
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])]
  const date = new Date(Date.UTC(year, month - 1, day))
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
}

const dayNumber = (year: number, month: number, day: number) => Date.UTC(year, month - 1, day) / 86_400_000

/** `in 16 days`, `today`, `3 days ago`: how far a target day is from `today` (a local date). */
export function relativeDay(iso: string, today: Date = new Date()): string {
  const [year, month, day] = iso.split('-').map(Number)
  const diff = Math.round(
    dayNumber(year, month, day) - dayNumber(today.getFullYear(), today.getMonth() + 1, today.getDate()),
  )
  if (diff === 0) return 'today'
  if (diff === 1) return 'tomorrow'
  if (diff === -1) return 'yesterday'
  return diff > 0 ? `in ${diff} days` : `${-diff} days ago`
}

/** `Oct 30` or `Oct 30, 2026`: a stored day, written the same everywhere. */
export function formatDay(iso: string, withYear = true): string {
  const [year, month, day] = iso.split('-').map(Number)
  return new Date(Date.UTC(year, month - 1, day)).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: withYear ? 'numeric' : undefined,
    timeZone: 'UTC',
  })
}

/** The year only when it is not this one: `Oct 30` now, `Oct 30, 2027` next year. */
export const formatTarget = (iso: string, today: Date = new Date()) =>
  formatDay(iso, !iso.startsWith(String(today.getFullYear())))

/** `Oct 14`: when a timestamp happened, by the reader's calendar; the year only when it is not this one. */
export function formatStamp(iso: string, today: Date = new Date()): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  return date.toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() === today.getFullYear() ? undefined : 'numeric',
  })
}

/**
 * What a trigger binding is filtered to, for the chat row. Every filter given must match, so they read as one line;
 * `release_id` filters `kanban:change` only.
 */
export function triggerScope(triggerType: string, config: Record<string, unknown>): string {
  const given = (name: string) => (typeof config[name] === 'string' ? (config[name] as string).trim() : '')
  const parts = [
    given('ticket_id'),
    given('parent_id') && `${given('parent_id')} and its subtasks`,
    triggerType === 'kanban:change' && given('release_id') && `tickets in ${given('release_id')}`,
  ].filter(Boolean)
  return parts.length > 0 ? parts.join(' · ') : 'any ticket'
}
