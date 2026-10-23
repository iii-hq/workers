/**
 * The ticket screen's rules for subtasks, parents, projects and releases
 * (design §6.1, §6.5), the console UI's ui/src/subtask-rules.ts as the phone
 * needs them. Pure: no `iii`.
 */

import type { Brief, Column, Project, Release, Summary, Ticket } from './api'

/** Live children that are neither done nor canceled. */
export const openChildren = (children: readonly Brief[], done: string, canceled: string | null): Brief[] =>
  children.filter((c) => !c.deleted_at && c.status !== done && c.status !== canceled)

/** The nearest column before `id` that is not `skip`: where a parent closed too early goes back to. */
export function columnBefore(columns: readonly Column[], id: string, skip: string | null): Column | null {
  const index = columns.findIndex((c) => c.id === id)
  for (let at = index - 1; at >= 0; at -= 1) if (columns[at]?.id !== skip) return columns[at] ?? null
  return null
}

/** A ticket in the done column with subtasks still open: the open ones and where "Move back" goes. */
export function doneWarning(
  ticket: Pick<Ticket, 'status' | 'children' | 'deleted_at'>,
  done: string,
  canceled: string | null,
  columns: readonly Column[],
): { open: Brief[]; back: Column | null } | null {
  if (ticket.deleted_at || ticket.status !== done) return null
  const open = openChildren(ticket.children, done, canceled)
  // Never back into the canceled column: that would close the parent instead of reopening it.
  return open.length ? { open, back: columnBefore(columns, done, canceled) } : null
}

/** Children in the order the screen lists them: the open ones first, each group by number. */
export function childOrder(children: readonly Brief[], done: string, canceled: string | null): Brief[] {
  const closed = (c: Brief) => (c.status === done || c.status === canceled ? 1 : 0)
  return [...children].sort((a, b) => closed(a) - closed(b) || keyNumber(a.key) - keyNumber(b.key))
}

/** The number in `KAN-182`; keys without one sort last. */
export function keyNumber(key: string): number {
  const digits = key.match(/(\d+)$/)
  return digits ? Number(digits[1]) : Number.POSITIVE_INFINITY
}

/**
 * The fields a new subtask starts with: the parent, the first column, and the
 * parent's project and release when they still take tickets (copied once,
 * never synced; an archived project and a released release are left out).
 */
export function subtaskDefaults(
  ticket: Pick<Ticket, 'key' | 'project_id' | 'release_id'>,
  projects: readonly Project[],
  releases: readonly Release[],
  first: string | undefined,
): { parent: string; status?: string; project?: string; release?: string } {
  const project = projects.find((p) => p.id === ticket.project_id)
  const release = releases.find((r) => r.id === ticket.release_id)
  return {
    parent: ticket.key,
    ...(first ? { status: first } : {}),
    ...(ticket.project_id && project && !project.archived ? { project: ticket.project_id } : {}),
    ...(ticket.release_id && release && !release.released ? { release: ticket.release_id } : {}),
  }
}

/**
 * Why a ticket cannot be picked (null = it can), for `mode`: `parent` (the
 * tickets this one can become a subtask of) or `attach` (the tickets that can
 * become its subtasks). One level only: a parent has no parent, and a ticket
 * with subtasks cannot become one.
 */
export function pickRefusal(
  candidate: Summary,
  self: string,
  mode: 'parent' | 'attach',
  parents: ReadonlySet<string>,
  keys: ReadonlyMap<string, string>,
): string | null {
  if (candidate.id === self) return 'this ticket'
  if (candidate.deleted_at) return 'deleted'
  if (mode === 'parent')
    return candidate.parent_id ? `a subtask of ${keys.get(candidate.parent_id) ?? 'another'}` : null
  if (candidate.parent_id === self) return 'already a subtask'
  if (candidate.parent_id) return `already a subtask of ${keys.get(candidate.parent_id) ?? 'another ticket'}`
  return parents.has(candidate.id) ? 'has its own subtasks' : null
}
