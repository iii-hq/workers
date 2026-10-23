/* A folder's repository and worktree changes, pushed by the ide worker and
   shared: the IDE header's branch chip, the chat composer's and the Git
   window on one folder hold one binding of each kind, not one apiece.

   - `watchGit` binds `shell::git-changed`: HEAD, the index, refs and the
     other worktrees of the repository the folder is in, and the repository
     itself appearing or going (the worker watches a folder in none for a
     `.git`). `shell::changed` leaves `.git` out, so this is how a
     `git switch` typed in a terminal reaches the page.
   - `watchWorktree` hears the folder's files change (`shell::changed`, the
     changes git does not ignore), coalesced: what can make the worktree
     dirty or clean. A view that already hears the folder (the IDE page's
     own `shell::changed` binding) passes its events on with
     `publishWorktree`, and while it does no second binding is made.

   Every event bumps a generation that reads of the folder key their shared
   results on: a read started before the event never answers after it. */

import type { Host } from '@iii-dev/console-ui'

/** `shell::git-changed`'s payload (ide/src/git_events.rs). */
export interface GitChangedEvent {
  /** The watched folder, canonical. */
  path: string
  /** The worktree's top level; absent outside a repository. */
  worktree?: string
  git_dir?: string
  common_dir?: string
  /** What moved: `head`, `index`, `refs`, `worktrees`, `repository`. */
  changes: string[]
}

type Kind = 'git' | 'worktree'

interface Entry {
  listeners: Set<(event: GitChangedEvent) => void>
  /** Views passing the folder's worktree events on (worktree kind only). */
  publishers: number
  /** The engine binding, while one is wanted. */
  off: (() => void) | null
  /** A worktree burst waiting out its coalesce window. */
  timer: ReturnType<typeof setTimeout> | null
}

const TRIGGER: Record<Kind, string> = { git: 'shell::git-changed', worktree: 'shell::changed' }
// The `iii::` prefix keeps the per-event invocations span-suppressed.
const FUNCTION: Record<Kind, string> = {
  git: 'iii::shell-ui::git-changed',
  worktree: 'iii::shell-ui::worktree-changed',
}
/** Worktree events arrive one per path: a save, a build, an agent's batch of
    writes is one notice. */
export const WORKTREE_COALESCE_MS = 250

const registries = new WeakMap<Host, Map<string, Entry>>()
let bindings = 0
// One counter for every folder and kind: a generation never repeats, so a
// result keyed on one is never mistaken for a later one.
let generations = 0
const generationOf = new WeakMap<Host, Map<string, number>>()

const keyOf = (kind: Kind, dir: string) => `${kind}\u0000${dir}`

function registry(host: Host): Map<string, Entry> {
  let map = registries.get(host)
  if (map === undefined) {
    map = new Map()
    registries.set(host, map)
  }
  return map
}

/** The folder's generation for `kind`: it moves on every change heard. */
export function generation(host: Host, kind: Kind, dir: string): number {
  return generationOf.get(host)?.get(keyOf(kind, dir)) ?? 0
}

function bump(host: Host, kind: Kind, dir: string) {
  let map = generationOf.get(host)
  if (map === undefined) {
    map = new Map()
    generationOf.set(host, map)
  }
  map.set(keyOf(kind, dir), ++generations)
}

function deliver(host: Host, kind: Kind, dir: string, entry: Entry, event: GitChangedEvent) {
  bump(host, kind, dir)
  for (const listener of [...entry.listeners]) listener(event)
}

const worktreeEvent = (dir: string): GitChangedEvent => ({ path: dir, changes: ['worktree'] })

/** A worktree change heard: one notice per coalesce window. */
function noteWorktree(host: Host, dir: string, entry: Entry) {
  if (entry.timer !== null) return
  entry.timer = setTimeout(() => {
    entry.timer = null
    deliver(host, 'worktree', dir, entry, worktreeEvent(dir))
  }, WORKTREE_COALESCE_MS)
}

function bind(host: Host, kind: Kind, dir: string, entry: Entry): () => void {
  bindings += 1
  const functionId = `${FUNCTION[kind]}::${bindings}`
  const offHandler = host.iii.on<GitChangedEvent & { ignored?: boolean }>(functionId, (event) => {
    if (kind === 'git') {
      if (Array.isArray(event?.changes)) deliver(host, kind, dir, entry, event)
      return
    }
    // Ignored paths never change what `git status` reports.
    if (event?.ignored !== true) noteWorktree(host, dir, entry)
  })
  let offTrigger: () => void = () => {}
  try {
    offTrigger = host.iii.registerTrigger({
      type: TRIGGER[kind],
      function_id: `${functionId}::${host.iii.browserId}`,
      config: { path: dir },
    })
  } catch {
    // No ide worker to bind: the views keep their last read.
  }
  return () => {
    try {
      offTrigger()
    } finally {
      offHandler()
    }
  }
}

/** Bind while someone listens and nobody passes the events on already. */
function sync(host: Host, kind: Kind, dir: string) {
  const map = registry(host)
  const key = keyOf(kind, dir)
  const entry = map.get(key)
  if (entry === undefined) return
  const wanted = entry.listeners.size > 0 && entry.publishers === 0
  if (wanted && entry.off === null) entry.off = bind(host, kind, dir, entry)
  else if (!wanted && entry.off !== null) {
    entry.off()
    entry.off = null
  }
  if (entry.listeners.size === 0 && entry.publishers === 0) {
    if (entry.timer !== null) clearTimeout(entry.timer)
    map.delete(key)
  }
}

function entryFor(host: Host, kind: Kind, dir: string): Entry {
  const map = registry(host)
  const key = keyOf(kind, dir)
  let entry = map.get(key)
  if (entry === undefined) {
    entry = { listeners: new Set(), publishers: 0, off: null, timer: null }
    map.set(key, entry)
  }
  return entry
}

function listen(host: Host, kind: Kind, dir: string, listener: (event: GitChangedEvent) => void): () => void {
  const entry = entryFor(host, kind, dir)
  // One function per subscription: the same listener twice is two.
  const own = (event: GitChangedEvent) => listener(event)
  entry.listeners.add(own)
  sync(host, kind, dir)
  return () => {
    entry.listeners.delete(own)
    sync(host, kind, dir)
  }
}

/** Calls `listener` with each batch of changes to `dir`'s repository. */
export function watchGit(host: Host, dir: string, listener: (event: GitChangedEvent) => void): () => void {
  return listen(host, 'git', dir, listener)
}

/** Calls `listener` once per burst of changes to the files under `dir`. */
export function watchWorktree(host: Host, dir: string, listener: () => void): () => void {
  return listen(host, 'worktree', dir, () => listener())
}

/** For a view already bound to `dir`'s `shell::changed`: pass its changes
    on with `note`, and the watchers of `dir` make no binding of their own
    meanwhile. `off` when the view lets go of the folder. */
export function publishWorktree(host: Host, dir: string): { note: () => void; off: () => void } {
  const entry = entryFor(host, 'worktree', dir)
  entry.publishers += 1
  sync(host, 'worktree', dir)
  let done = false
  return {
    note: () => {
      if (!done) noteWorktree(host, dir, entry)
    },
    off: () => {
      if (done) return
      done = true
      entry.publishers -= 1
      sync(host, 'worktree', dir)
    },
  }
}
