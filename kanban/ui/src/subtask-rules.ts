import type { Activity, Brief, Change, Column, ProjectView, ReleaseView, Ticket, TicketSummary } from './shared'

/*
 * Pure rules for subtasks, projects and releases on the ticket screen: no React and no console-ui, so
 * `subtasks.check.ts` runs them under plain node.
 */

export type Option = { value: string; label: string; description?: string; disabled?: boolean }
export type OptionGroup = { label: string; options: Option[] }

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const natural = new Intl.Collator('en', { numeric: true, sensitivity: 'base' })

export const plural = (count: number, one: string) => `${count} ${count === 1 ? one : `${one}s`}`

/** `2026-10-30` → `Oct 30`; anything else is no date. Read as text, so no timezone can move the day. */
export function shortDate(day: string | null | undefined): string | null {
  const match = day?.match(/^(\d{4})-(\d{2})-(\d{2})/)
  const month = match ? MONTHS[Number(match[2]) - 1] : undefined
  return match && month ? `${month} ${Number(match[3])}` : null
}

/** The number in `KAN-182`; keys without one sort last. */
export function keyNumber(key: string): number {
  const digits = key.match(/(\d+)$/)
  return digits ? Number(digits[1]) : Number.POSITIVE_INFINITY
}

const byKey = <T extends { key: string }>(a: T, b: T) => keyNumber(a.key) - keyNumber(b.key)

/* ── facet options ───────────────────────────────────────────────────── */

/** Active projects by name. One already on the ticket stays listed even when archived, or the selector would call it "No project". */
export function projectOptions(projects: ProjectView[], currentId: string | null): Option[] {
  const options = projects
    .filter((project) => !project.archived_at)
    .sort((a, b) => natural.compare(a.name, b.name))
    .map((project) => ({ value: project.id, label: project.name, description: plural(project.total, 'ticket') }))
  if (currentId && !options.some((option) => option.value === currentId)) {
    const current = projects.find((project) => project.id === currentId)
    options.push({
      value: currentId,
      label: current ? `${current.name} (archived)` : 'Unknown project',
      description: current ? plural(current.total, 'ticket') : '',
    })
  }
  return options
}

/** Releases in the order the worker sorts them, the released ones under their own heading. */
export function releaseGroups(releases: ReleaseView[], currentId: string | null): OptionGroup[] {
  const option = (release: ReleaseView): Option => ({
    value: release.id,
    label: release.name,
    description: [shortDate(release.target_at), plural(release.total, 'ticket')].filter(Boolean).join(' · '),
  })
  const planned = releases.filter((release) => !release.released_at).map(option)
  if (currentId && !releases.some((release) => release.id === currentId)) {
    planned.push({ value: currentId, label: 'Unknown release', description: '' })
  }
  return [
    { label: '', options: planned },
    { label: 'Released', options: releases.filter((release) => release.released_at).map(option) },
  ]
}

/**
 * The tickets this one can become a subtask of: live, not itself, not a subtask already. The current parent
 * stays listed even when deleted. The key is the label so the trigger reads `KAN-57`; the title is searchable.
 */
export function parentOptions(tickets: TicketSummary[], selfId: string | null, current: Brief | null): Option[] {
  const options = tickets
    .filter((ticket) => !ticket.deleted_at && ticket.id !== selfId && !ticket.parent_id)
    .sort(byKey)
    .map((ticket) => ({ value: ticket.key, label: ticket.key, description: ticket.title }))
  if (current && !options.some((option) => option.value === current.key)) {
    options.unshift({
      value: current.key,
      label: current.deleted_at ? `${current.key} (deleted)` : current.key,
      description: current.title,
    })
  }
  return options
}

/** Why the Parent selector is off, or null: one level only, so a ticket with subtasks can't be one. */
export function parentBlockedReason(ticket: Pick<Ticket, 'children'>): string | null {
  const count = ticket.children.length
  return count > 0 ? `Has ${plural(count, 'subtask')}, so it can't be a subtask.` : null
}

/** The id of the parent's project or release when this ticket has none: "Its parent is in Phase 2 · Add to Phase 2". */
export function parentFacet(
  ticket: Pick<Ticket, 'parent' | 'project_id' | 'release_id'>,
  kind: 'project' | 'release',
): string | null {
  const own = kind === 'project' ? ticket.project_id : ticket.release_id
  const theirs = kind === 'project' ? ticket.parent?.project_id : ticket.parent?.release_id
  return !own && theirs ? theirs : null
}

/**
 * "Its parent is in Phase 2 · Add to Phase 2": the parent's project or release when this ticket has none and the
 * target takes new tickets (an archived project and a released release don't). Null when there is nothing to offer.
 */
export function parentOffer(
  ticket: Pick<Ticket, 'parent' | 'project_id' | 'release_id'>,
  kind: 'project' | 'release',
  projects: ProjectView[],
  releases: ReleaseView[],
): { id: string; name: string } | null {
  const id = parentFacet(ticket, kind)
  const entity =
    kind === 'project'
      ? projects.find((project) => project.id === id && !project.archived_at)
      : releases.find((release) => release.id === id && !release.released_at)
  return id && entity ? { id, name: entity.name } : null
}

/* ── the New ticket dialog ───────────────────────────────────────────── */

/** What the board prefills when New ticket is opened from a swimlane cell; a project or release by id or name, a parent by id or key. */
export type NewTicketDefaults = {
  status?: string
  project?: string | null
  release?: string | null
  parent?: string | null
  /** The swimlane's own short name ("Phase 5", "KAN-30"), for "Opened from the … swimlane". */
  lane?: string
}

export type Facets = { project: string | null; release: string | null; parent: string | null }
export const NO_FACETS: Facets = { project: null, release: null, parent: null }

export type Plan = { projects: ProjectView[]; releases: ReleaseView[]; tickets: TicketSummary[] }

const sameText = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase()

/** The defaults as the selectors hold them (ids, the parent's key). What no longer exists, or refuses new tickets, is dropped. */
export function resolveDefaults(plan: Plan, defaults: NewTicketDefaults | undefined): Facets {
  const project = defaults?.project
  const release = defaults?.release
  const parent = defaults?.parent
  return {
    project:
      (project && plan.projects.find((p) => !p.archived_at && (p.id === project || sameText(p.name, project)))?.id) ||
      null,
    release:
      (release && plan.releases.find((r) => !r.released_at && (r.id === release || sameText(r.name, release)))?.id) ||
      null,
    parent:
      (parent &&
        plan.tickets.find((t) => !t.deleted_at && !t.parent_id && (t.id === parent || sameText(t.key, parent)))?.key) ||
      null,
  }
}

/** "Phase 3" in "Opened from the Phase 3 swimlane": the lane's own name, else the release, the project, the parent. */
export function laneName(plan: Plan, facets: Facets, lane?: string | null): string | null {
  return (
    lane?.trim() ||
    (plan.releases.find((r) => r.id === facets.release)?.name ??
      plan.projects.find((p) => p.id === facets.project)?.name ??
      facets.parent)
  )
}

/* ── add an existing ticket ──────────────────────────────────────────── */

/** Ticket id → why it can't become a subtask of `selfId` (null when it can). */
export function attachReasons(tickets: TicketSummary[], selfId: string): Map<string, string | null> {
  const keys = new Map(tickets.map((ticket) => [ticket.id, ticket.key]))
  const parents = new Set(
    tickets.filter((ticket) => ticket.parent_id && !ticket.deleted_at).map((ticket) => ticket.parent_id),
  )
  return new Map(
    tickets.map((ticket) => {
      const reason =
        ticket.id === selfId
          ? 'this ticket'
          : ticket.deleted_at
            ? 'deleted'
            : ticket.parent_id
              ? `already a subtask of ${keys.get(ticket.parent_id) ?? 'another ticket'}`
              : parents.has(ticket.id)
                ? 'has its own subtasks'
                : null
      return [ticket.id, reason]
    }),
  )
}

/** Tickets that can join first, the ones that can't after them, each group by number. */
export function attachOrder(tickets: TicketSummary[], reasons: Map<string, string | null>): TicketSummary[] {
  const blocked = (ticket: TicketSummary) => (reasons.get(ticket.id) ? 1 : 0)
  return [...tickets].sort((a, b) => blocked(a) - blocked(b) || byKey(a, b))
}

/* ── subtasks of a ticket ────────────────────────────────────────────── */

export const openChildren = (children: Brief[], doneStatus: string): Brief[] =>
  children.filter((child) => !child.deleted_at && child.status !== doneStatus)

export function progressPercent(progress: { done: number; total: number }): number {
  return progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : 0
}

/** The column just before `id`, where a parent returns to when it was closed too early. */
export function columnBefore(columns: Column[], id: string): Column | null {
  const index = columns.findIndex((column) => column.id === id)
  return index > 0 ? (columns[index - 1] ?? null) : null
}

/** A ticket in the done column with subtasks still open: the open ones and where "Move back" goes. */
export function doneWarning(
  ticket: Pick<Ticket, 'status' | 'children' | 'deleted_at'>,
  doneStatus: string,
  columns: Column[],
): { open: Brief[]; back: Column | null } | null {
  if (ticket.deleted_at || ticket.status !== doneStatus) return null
  const open = openChildren(ticket.children, doneStatus)
  return open.length > 0 ? { open, back: columnBefore(columns, doneStatus) } : null
}

/** The neighbours of a subtask among its parent's, in the parent's order. */
export function siblings(children: Brief[], id: string): { previous: Brief | null; next: Brief | null } {
  const index = children.findIndex((child) => child.id === id)
  if (index < 0) return { previous: null, next: null }
  return { previous: children[index - 1] ?? null, next: children[index + 1] ?? null }
}

/**
 * The fields a new subtask starts with: the parent's project and release are copied once, never synced. An
 * archived project and a released release take no new tickets, so they are left out instead of failing the create.
 */
export function subtaskDefaults(
  ticket: Pick<Ticket, 'key' | 'project_id' | 'release_id'>,
  projects: ProjectView[],
  releases: ReleaseView[],
  firstColumn: string | undefined,
): Record<string, unknown> {
  const project = ticket.project_id ? projects.find((candidate) => candidate.id === ticket.project_id) : undefined
  const release = ticket.release_id ? releases.find((candidate) => candidate.id === ticket.release_id) : undefined
  return {
    parent: ticket.key,
    ...(firstColumn ? { status: firstColumn } : {}),
    ...(ticket.project_id && !project?.archived_at ? { project: ticket.project_id } : {}),
    ...(ticket.release_id && !release?.released_at ? { release: ticket.release_id } : {}),
  }
}

/** The project a create was refused for ("project X is archived; …"), or null for any other error. */
export function archivedProjectIn(message: string): string | null {
  return message.match(/\bproject (.+?) is archived\b/)?.[1] ?? null
}

/** The helper under the add field: where a new subtask starts and what it takes from this ticket. */
export function prefillHint(column: string, hasProject: boolean, hasRelease: boolean): string {
  const taken = hasProject && hasRelease ? 'project and release' : hasProject ? 'project' : 'release'
  return hasProject || hasRelease
    ? `New subtasks start in ${column} with this ticket's ${taken} — you can change them.`
    : `New subtasks start in ${column}.`
}

/* ── activity ────────────────────────────────────────────────────────── */

/** A sentence is text and ticket keys; the screen renders a key as a link. */
export type Part = string | { ticket: string }

const text = (value: unknown): string | null => (typeof value === 'string' && value ? value : null)

/** The parent side: "added subtask KAN-182". */
export function subtaskSentence(type: string, changes: Change[] | undefined): Part[] {
  const change = changes?.find((candidate) => candidate.field === 'subtask')
  const key = text(change?.to) ?? text(change?.from)
  const verb = type === 'subtask.removed' ? 'removed' : 'added'
  return key ? [`${verb} subtask `, { ticket: key }] : [`${verb} a subtask`]
}

/** The child side. */
export function parentSentence(change: Change): Part[] {
  const [from, to] = [text(change.from), text(change.to)]
  if (from && to) return ['made this a subtask of ', { ticket: to }, ' instead of ', { ticket: from }]
  if (to) return ['made this a subtask of ', { ticket: to }]
  if (from) return ['detached this from ', { ticket: from }]
  return ['changed the parent']
}

/** A project or release, by the readable names the worker records. */
export function planSentence(change: Change): string {
  const [from, to] = [text(change.from), text(change.to)]
  if (from && to) return `moved this from ${from} to ${to}`
  if (to) return `added this to ${to}`
  if (from) return `removed this from ${from}`
  return `changed the ${change.field}`
}

export type TimelineRow<T> = { kind: 'row'; item: T } | { kind: 'adds'; items: T[]; keys: string[] }

const addedKey = (entry: Pick<Activity, 'type' | 'changes'>): string | null =>
  entry.type === 'subtask.added' ? text(entry.changes?.find((change) => change.field === 'subtask')?.to) : null

/** Consecutive `subtask.added` entries by one actor become one row: "added 11 subtasks KAN-128 … KAN-167". */
export function groupSubtaskAdds<T extends { entry: Activity }>(items: T[]): TimelineRow<T>[] {
  const rows: TimelineRow<T>[] = []
  for (const item of items) {
    const key = addedKey(item.entry)
    const last = rows[rows.length - 1]
    const first = last && (last.kind === 'row' ? last.item : last.items[0])
    if (!key || !last || !first || !addedKey(first.entry) || first.entry.actor !== item.entry.actor) {
      rows.push({ kind: 'row', item })
    } else if (last.kind === 'adds') {
      last.items.push(item)
      last.keys.push(key)
    } else {
      rows[rows.length - 1] = {
        kind: 'adds',
        items: [last.item, item],
        keys: [addedKey(last.item.entry) as string, key],
      }
    }
  }
  return rows
}
