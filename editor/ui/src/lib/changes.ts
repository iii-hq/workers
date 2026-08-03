/**
 * The recent-changes feed.
 *
 * The worker already emits `editor::changed` for every file change however it
 * was made — an agent writing through `shell`, a save through this worker, or
 * an edit made by anything else the shell observer sees. The page was throwing
 * those events away after flashing a tree row, so "what just changed, and what
 * did it do" had no answer short of reading the tree and guessing.
 *
 * This is the model behind that answer, and it is deliberately pure: the
 * collapse rule and the ordering are the parts that are easy to get wrong, and
 * they should be testable without an engine or a browser.
 *
 * Nothing here is persisted. The feed is what happened while you were looking;
 * the durable record of a change is the file and its git history.
 */

import type { ChangedEvent } from './events'

/** One row of the feed: the latest change to one path. */
export interface ChangeEntry {
  path: string
  /** Function id that produced the change, e.g. `shell::fs::write`. */
  cause: string
  kind: string
  added: number
  removed: number
  /** Inline patch from the event, already truncated by the worker. Empty when
   *  there was no previous content to compare against. */
  patch: string
  truncated: boolean
  /** Client clock when the event arrived, or `null` for an entry seeded from
   *  the working tree — that change happened before the page was watching and
   *  claiming a time for it would be a lie. */
  at: number | null
  /** How many events collapsed into this row. A save loop writes one file many
   *  times; showing that as forty rows buries every other change. */
  count: number
}

/** Newest first, one row per path, bounded. */
export const MAX_ENTRIES = 50

/**
 * Fold one event into the feed.
 *
 * Same path collapses: the row keeps its place at the front, takes the newest
 * event's facts, and counts the repeat. A file touched twice is one thing that
 * changed twice, not two things.
 */
export function recordChange(log: ChangeEntry[], event: ChangedEvent, now: number): ChangeEntry[] {
  const previous = log.find((entry) => entry.path === event.path)
  const next: ChangeEntry = {
    path: event.path,
    cause: event.cause,
    kind: event.kind,
    added: event.added,
    removed: event.removed,
    patch: event.patch,
    truncated: event.truncated,
    at: now,
    count: (previous?.count ?? 0) + 1,
  }
  return [next, ...log.filter((entry) => entry.path !== event.path)].slice(0, MAX_ENTRIES)
}

/**
 * Rows for the working tree, for changes that predate the page.
 *
 * Seeding only fills gaps: a path already in the feed has a real event behind
 * it, which is strictly better information than a status code.
 */
export function seedFromStatus(
  log: ChangeEntry[],
  entries: { path: string; index: string; worktree: string }[],
): ChangeEntry[] {
  const known = new Set(log.map((entry) => entry.path))
  const seeded = entries
    .filter((entry) => !known.has(entry.path))
    .map<ChangeEntry>((entry) => ({
      path: entry.path,
      cause: 'git',
      kind: statusKind(entry),
      added: 0,
      removed: 0,
      patch: '',
      truncated: false,
      at: null,
      count: 0,
    }))
  return [...log, ...seeded].slice(0, MAX_ENTRIES)
}

/** The change kind a git status pair describes. */
function statusKind(entry: { index: string; worktree: string }): string {
  const codes = `${entry.index}${entry.worktree}`
  if (codes.includes('?')) return 'created'
  if (codes.includes('A')) return 'created'
  if (codes.includes('D')) return 'deleted'
  if (codes.includes('R')) return 'moved'
  return 'modified'
}

/**
 * Where a change came from, named by surface rather than by actor.
 *
 * The event carries a function id, which says which worker performed the
 * write — not who asked for it. An agent turn and a person can both reach
 * `editor::save`, so labelling one of them "agent" would be a guess presented
 * as a fact. The surface is what the payload actually knows.
 */
export function causeLabel(cause: string): string {
  const worker = String(cause || '').split('::')[0]
  if (!worker) return 'unknown'
  if (worker === 'git') return 'working tree'
  return worker
}

/** Compact relative age: the feed is about recency, not timestamps. */
export function relativeAge(at: number | null, now: number): string {
  if (at === null) return 'earlier'
  const seconds = Math.max(0, Math.round((now - at) / 1000))
  if (seconds < 5) return 'now'
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m`
  return `${Math.round(minutes / 60)}h`
}
