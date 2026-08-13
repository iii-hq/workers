import type { EditorCacheEntry } from './EditorPane'

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

/** Reconcile transient row-level dirty signals with the durable page cache.
    A review row unmount reports clean, but its cached draft still owns the
    edit until the user explicitly saves, cancels, or discards it. */
export function currentReviewDirtyPaths(
  reviewPaths: Iterable<string>,
  cache: ReadonlyMap<string, EditorCacheEntry>,
  reportedDirtyPaths: Iterable<string>,
): ReadonlySet<string> {
  const dirty = new Set(reportedDirtyPaths)
  for (const path of reviewPaths) {
    const entry = cache.get(path)
    if (entry !== undefined && entry.draft !== entry.savedContent) dirty.add(path)
  }
  return dirty
}
