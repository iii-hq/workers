import type { EditorCache, EditorCacheEntry } from './EditorPane'
import { createPaneMemory, paneFolderKey } from './pane-memory'
import { fileTabId, fileTarget, type TabsState } from './tabs'

/** Apply a watcher read only when the editor still mirrors its last saved
    baseline. A dirty buffer must retain both that baseline and its original
    optimistic revision so a later save detects an intervening disk write. */
export function refreshCleanEditorCacheEntry(
  entry: EditorCacheEntry,
  content: string,
  revision?: string,
): boolean {
  if (entry.draft !== entry.savedContent || entry.savedContent === content) return false
  entry.savedContent = content
  entry.draft = content
  entry.revision = revision ?? entry.revision
  return true
}

/* ── surviving a remount ──
   The console mounts only the workspace tab on screen: switching away and
   back mounts this page afresh. Two things must not be lost on the way:
   the edits not saved yet, and where the reader was in each file.

   Kept edits are keyed by pane AND folder: their paths are relative to the
   folder, and while the page is away the chat may move the pane to another
   worktree — whose `src/x.ts` must never wear the first one's draft. They
   wait under their folder until the pane is back on it. */

const keptEdits = createPaneMemory<EditorCache>()

/** Hold a pane's unsaved buffers for `root` while the page is unmounted.
    Clean buffers are dropped: they reload from disk, which may have moved on. */
export function keepUnsavedEdits(paneKey: string, root: string | null, cache: EditorCache): void {
  if (root === null) return
  const key = paneFolderKey(paneKey, root)
  const dirty: EditorCache = new Map()
  for (const [path, entry] of cache) {
    if (entry.draft !== entry.savedContent) dirty.set(path, { ...entry, image: null })
  }
  if (dirty.size > 0) keptEdits.write(key, dirty)
  else keptEdits.forget(key)
}

/** The buffers a pane left unsaved in `root` when it last unmounted, once. */
export function takeUnsavedEdits(paneKey: string, root: string): EditorCache {
  const key = paneFolderKey(paneKey, root)
  const kept = keptEdits.read(key)
  keptEdits.forget(key)
  return kept ?? new Map()
}

/** Fold the edits kept for a folder back into the pane coming onto it: the
    buffers rejoin `cache`, and a file whose tab the restored set lacks gets
    a pinned one at the end (a draft never comes back with no way to it).
    The active tab stays the restored one. */
export function rejoinKeptEdits(
  kept: EditorCache,
  cache: EditorCache,
  tabs: TabsState,
): { tabs: TabsState; dirty: ReadonlySet<string> } {
  if (kept.size === 0) return { tabs, dirty: new Set() }
  const added = []
  for (const [path, entry] of kept) {
    cache.set(path, entry)
    const id = fileTabId(path)
    if (!tabs.tabs.some((tab) => tab.id === id)) added.push({ id, target: fileTarget(path), pinned: true })
  }
  const next =
    added.length === 0 ? tabs : { tabs: [...tabs.tabs, ...added], active: tabs.active ?? added[0].id }
  return { tabs: next, dirty: new Set(kept.keys()) }
}

/** Most files whose place is remembered; the oldest go first. */
const VIEW_STATES_KEPT = 300
const viewStates = new Map<string, unknown>()

/** Where the reader was in a file (Monaco's view state), by absolute path. */
export function rememberedViewState(absPath: string): unknown {
  return viewStates.get(absPath)
}

export function rememberViewState(absPath: string, state: unknown): void {
  viewStates.delete(absPath)
  viewStates.set(absPath, state)
  if (viewStates.size > VIEW_STATES_KEPT) {
    const oldest = viewStates.keys().next().value
    if (oldest !== undefined) viewStates.delete(oldest)
  }
}
