/* Warm the editor cache for the file the pointer rests on, so the click
   that follows opens it from memory instead of waiting on the worker.

   The explorer reports every file row the pointer crosses; the prefetcher
   waits a beat (a sweep down the tree must not read every file it passes),
   then reads the file through the same call the editor uses and stores the
   same cache entry the editor would, so `EditorPane` starts in its ready
   state. Images are left to the pane (they stream through a byte reader
   with progress), and anything over `maxBytes` is skipped: a hover must
   never pull a megabyte it may not need.

   Entries it created and the user never opened are the prefetcher's to
   evict (oldest first, past `keep`); an entry the page opened in a tab is
   the page's. A disk change drops clean entries page-side, prefetched ones
   included, so a hover never shows stale content. */

import type { Host } from '@iii-dev/console-ui'
import { coderReadFile } from './coder'
import type { EditorCache, EditorCacheEntry } from './EditorPane'
import { imageMimeFromPath } from './file-kinds'

export interface FilePrefetcherOptions {
  host: Host
  cache: EditorCache
  /** The absolute path to read, with a token that must still match once the
      read lands (the root generation); null when the file must not be read
      (no root, a protected path). */
  resolve: (rel: string) => { abs: string; token: unknown } | null
  /** Whether a tab holds the file: its entry then belongs to the page. */
  isOpen: (rel: string) => boolean
  /** The pointer must rest this long before a read starts. */
  delayMs?: number
  /** Files larger than this are left for the click. */
  maxBytes?: number
  /** How many unopened prefetched entries to keep around. */
  keep?: number
  /** Test seam. */
  now?: () => number
}

export interface FilePrefetcher {
  /** The pointer is on `rel`: read it after the delay unless it moves on. */
  prefetch: (rel: string) => void
  /** The pointer left the tree: nothing pending reads. */
  cancel: () => void
  /** Forget everything (a root switch). */
  reset: () => void
}

export const PREFETCH_DELAY_MS = 90
export const PREFETCH_MAX_BYTES = 512 * 1024
export const PREFETCH_KEEP = 24

export function createFilePrefetcher({
  host,
  cache,
  resolve,
  isOpen,
  delayMs = PREFETCH_DELAY_MS,
  maxBytes = PREFETCH_MAX_BYTES,
  keep = PREFETCH_KEEP,
  now = () => Date.now(),
}: FilePrefetcherOptions): FilePrefetcher {
  let timer: ReturnType<typeof setTimeout> | null = null
  let pending: string | null = null
  const inflight = new Set<string>()
  // Paths a read refused (too large, unreadable): not asked again this root.
  const refused = new Set<string>()
  // Entries this prefetcher put in the cache, by insertion time.
  const owned = new Map<string, number>()

  const evict = () => {
    for (const [rel] of owned) {
      if (owned.size <= keep) return
      owned.delete(rel)
      if (isOpen(rel)) continue
      const entry = cache.get(rel)
      // Only a clean, untouched body is ours to drop.
      if (entry && entry.draft === entry.savedContent) cache.delete(rel)
    }
  }

  const read = (rel: string) => {
    if (cache.has(rel) || inflight.has(rel) || refused.has(rel)) return
    if (imageMimeFromPath(rel) !== null) return
    const target = resolve(rel)
    if (target === null) return
    inflight.add(rel)
    coderReadFile(host, target.abs, { maxOutputBytes: maxBytes })
      .then((out) => {
        if (resolve(rel)?.token !== target.token) return
        // Over budget or not text: the click reads it the editor's own way.
        if (out.is_utf8 === false || out.more_lines) {
          refused.add(rel)
          return
        }
        if (cache.has(rel)) return
        const content = out.content ?? ''
        const entry: EditorCacheEntry = {
          savedContent: content,
          draft: content,
          revision: out.revision ?? undefined,
          readOnly: null,
          mode: out.mode ?? null,
          size: out.size ?? null,
        }
        cache.set(rel, entry)
        owned.set(rel, now())
        evict()
      })
      .catch(() => {
        refused.add(rel)
      })
      .finally(() => {
        inflight.delete(rel)
      })
  }

  const cancel = () => {
    if (timer !== null) clearTimeout(timer)
    timer = null
    pending = null
  }

  return {
    prefetch: (rel) => {
      if (rel === pending) return
      cancel()
      if (cache.has(rel) || refused.has(rel)) return
      pending = rel
      timer = setTimeout(() => {
        timer = null
        pending = null
        read(rel)
      }, delayMs)
    },
    cancel,
    reset: () => {
      cancel()
      inflight.clear()
      refused.clear()
      owned.clear()
    },
  }
}
