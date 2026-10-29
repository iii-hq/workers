/* In-session memory for what a view holds that the person typed or set —
   a search query, a commit message — so it survives the view unmounting:
   another sidebar view taking its place, or the whole page remounting when
   the console shows another workspace tab. It lives only in this page's
   module scope, never in the worker's saved pane state: these texts can
   hold secrets, and they are worth keeping across a switch, not a reload.

   Keyed by pane (`pane-scope.ts`), and by folder where the state belongs to
   one repository, so neither leaks into another pane or worktree. */

export interface PaneMemory<T> {
  read(key: string): T | undefined
  write(key: string, value: T): void
  forget(key: string): void
}

/** Most keys remembered per memory; the least recently written go first. */
const KEYS_KEPT = 64

export function createPaneMemory<T>(limit = KEYS_KEPT): PaneMemory<T> {
  const entries = new Map<string, T>()
  return {
    read: (key) => entries.get(key),
    write(key, value) {
      entries.delete(key)
      entries.set(key, value)
      if (entries.size > limit) {
        const oldest = entries.keys().next().value
        if (oldest !== undefined) entries.delete(oldest)
      }
    },
    forget(key) {
      entries.delete(key)
    },
  }
}

/** The key for state that belongs to one folder of one pane. */
export function paneFolderKey(paneKey: string, root: string | null): string {
  return `${paneKey}\u0000${root ?? ''}`
}
