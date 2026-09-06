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
