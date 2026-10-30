/* Which files a person works in, per folder, kept in this browser: every
   open counts, with the time of the last one. Go to file (⌘P) ranks by it,
   so the files you live in are one or two letters away. */

import type { FileUse } from './quick-open'

const KEY_PREFIX = 'iii.ide.file-use:'
/** Files remembered per folder; the least used go first. */
const KEPT = 400

function key(root: string): string {
  return `${KEY_PREFIX}${root}`
}

export function loadFileUse(root: string): Record<string, FileUse> {
  try {
    const raw = localStorage.getItem(key(root))
    const parsed: unknown = raw ? JSON.parse(raw) : null
    if (!parsed || typeof parsed !== 'object') return {}
    const out: Record<string, FileUse> = {}
    for (const [rel, value] of Object.entries(parsed as Record<string, unknown>)) {
      const use = value as Partial<FileUse> | null
      if (use && typeof use.count === 'number' && typeof use.last === 'number') out[rel] = { count: use.count, last: use.last }
    }
    return out
  } catch {
    return {}
  }
}

/** Count one open of `rel` in `root`. */
export function recordFileUse(root: string, rel: string, now = Date.now()): void {
  const uses = loadFileUse(root)
  const current = uses[rel]
  uses[rel] = { count: (current?.count ?? 0) + 1, last: now }
  const entries = Object.entries(uses)
  const kept =
    entries.length > KEPT
      ? Object.fromEntries(entries.sort((a, b) => b[1].last - a[1].last).slice(0, KEPT))
      : uses
  try {
    localStorage.setItem(key(root), JSON.stringify(kept))
  } catch {
    // Storage full or private mode: ranking falls back to the fuzzy order.
  }
}
