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
 * The worker keeps its own bounded log of the same events, so a page that was
 * not open while an agent worked can still read what it did; this model folds
 * that log and the live stream into one feed.
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
  /** The harness session the write happened in, when it happened inside one. */
  sessionId?: string
  turnId?: string
  /** The workspace root the change was recorded against. An agent can be
   *  working in a different folder than the editor is pointed at, and a row
   *  whose root differs is the reason the file it names is not in the tree. */
  root?: string
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
    sessionId: event.session_id,
    turnId: event.turn_id,
    root: event.root,
  }
  return [next, ...log.filter((entry) => entry.path !== event.path)].slice(0, MAX_ENTRIES)
}

/**
 * Rows from the worker's recorded log.
 *
 * These are real changes with real provenance — they simply happened before
 * this page subscribed. They have no client clock, because the time they
 * carry is the worker's, not this page's; `earlier` is the honest age.
 * Anything already in the feed wins: a live entry has a timestamp this page
 * actually observed.
 */
export function fromRecords(
  log: ChangeEntry[],
  records: {
    path: string
    cause: string
    kind: string
    added: number
    removed: number
    patch: string
    truncated: boolean
    session_id?: string
    turn_id?: string
    root?: string
  }[],
): ChangeEntry[] {
  const known = new Set(log.map((entry) => entry.path))
  const restored = records
    .filter((record) => !known.has(record.path))
    .map<ChangeEntry>((record) => ({
      path: record.path,
      cause: record.cause,
      kind: record.kind,
      added: record.added,
      removed: record.removed,
      patch: record.patch,
      truncated: record.truncated,
      at: null,
      count: 1,
      sessionId: record.session_id,
      turnId: record.turn_id,
      root: record.root,
    }))
  return [...log, ...restored].slice(0, MAX_ENTRIES)
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

/**
 * A run of changes that belong together: one turn's work on several files.
 *
 * An agent turn that touches six files is one thing that happened, not six.
 * Read as a flat list it buries whatever else changed; read as a group it says
 * "this turn rewrote these six files, net +40 -12" at a glance, and the files
 * are still one click away.
 */
export interface ChangeGroup {
  /** Stable per run, so React keys survive a re-render. */
  key: string
  sessionId?: string
  turnId?: string
  entries: ChangeEntry[]
  added: number
  removed: number
  /** Newest change in the run, for the group's age. */
  at: number | null
  /** Root the run was recorded against, so a run that happened somewhere
   *  other than the open workspace can say so. */
  root?: string
}

/**
 * Fold the feed into runs.
 *
 * Grouping is by *adjacency*, not by collecting every entry of a turn wherever
 * it sits: the feed is a timeline, and a turn that writes, waits, then writes
 * again really did happen twice. Pulling those together would claim an order
 * that did not happen. Entries with no turn — a local edit, a working-tree
 * seed — group by their cause instead, so hand edits do not merge into an
 * agent's run.
 */
export function groupByTurn(log: ChangeEntry[]): ChangeGroup[] {
  const groups: ChangeGroup[] = []
  // Run identity and React key are different things: a turn that writes twice
  // with something in between is two runs, so the key needs an index while the
  // adjacency test must compare the bare run id.
  let openRun: string | null = null
  for (const entry of log) {
    const run = entry.turnId ?? entry.sessionId ?? `cause:${entry.cause}`
    const open = groups[groups.length - 1]
    if (open && openRun === run) {
      open.entries.push(entry)
      open.added += entry.added
      open.removed += entry.removed
      continue
    }
    openRun = run
    groups.push({
      key: `${run}#${groups.length}`,
      sessionId: entry.sessionId,
      turnId: entry.turnId,
      entries: [entry],
      added: entry.added,
      removed: entry.removed,
      at: entry.at,
      root: entry.root,
    })
  }
  return groups
}

/** How a run of changes names itself: the agent's session, or the surface. */
export function groupLabel(group: ChangeGroup): string {
  if (group.sessionId) return `agent ${group.sessionId.replace(/^s_/, '').slice(0, 6)}`
  return causeLabel(group.entries[0]?.cause ?? '')
}

/**
 * Whether a row names a file outside the open workspace.
 *
 * The observer makes a path relative to the workspace root and leaves it alone
 * when it does not sit under one, so an absolute path *is* the signal: the
 * agent was working somewhere the editor is not pointed at. The event's `root`
 * cannot be used for this — it reports the root the observer resolved, which
 * is the open workspace even when the file has nothing to do with it.
 */
export function isOutsideWorkspace(entry: ChangeEntry): boolean {
  return entry.path.startsWith('/')
}

/** The folder an outside change happened in, named for a header. */
export function outsideFolder(entry: ChangeEntry): string {
  const { dir } = splitPath(entry.path)
  const trimmed = dir.replace(/\/$/, '')
  return splitPath(trimmed).name || trimmed || '/'
}

/** `dir/` and `name` split, so a narrow row can keep the name and drop the path. */
export function splitPath(path: string): { dir: string; name: string } {
  const cut = path.lastIndexOf('/')
  if (cut === -1) return { dir: '', name: path }
  return { dir: path.slice(0, cut + 1), name: path.slice(cut + 1) }
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
