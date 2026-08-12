/* The guest-target file tree: `shell::fs::ls` has no recursive form, so
   directory listings load lazily — the guest root on selection, then one
   `ls` per directory the FilesTab reports expanded (its debounced
   expansion snapshot is the only expansion signal the tree model
   exposes). Listings cache per target; a failed `ls` caches an empty
   listing (matching the host tree's silent degrade). */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type FlatTree, joinPath } from './coder'
import { buildGuestTree, type FsEntry, GUEST_ROOT, guestLs } from './sandbox'

export function useGuestTree(
  host: Host,
  sandboxId: string | null,
  expanded: readonly string[],
): FlatTree | null {
  const cacheRef = useRef<Map<string, FsEntry[]>>(new Map())
  const seqRef = useRef(0)
  const [tree, setTree] = useState<FlatTree | null>(null)

  const load = useCallback(
    async (sid: string, dirs: string[]) => {
      const seq = seqRef.current
      await Promise.all(
        dirs.map(async (dir) => {
          let entries: FsEntry[] = []
          try {
            entries = await guestLs(host, sid, joinPath(GUEST_ROOT, dir))
          } catch {
            // Cache the empty listing so the miss is not refetched on
            // every expansion snapshot.
          }
          if (seqRef.current === seq) cacheRef.current.set(dir, entries)
        }),
      )
      if (seqRef.current === seq) setTree(buildGuestTree(cacheRef.current))
    },
    [host],
  )

  useEffect(() => {
    seqRef.current++
    cacheRef.current = new Map()
    setTree(null)
    if (sandboxId === null) return
    void load(sandboxId, [''])
  }, [sandboxId, load])

  useEffect(() => {
    if (sandboxId === null) return
    const missing = expanded.filter((dir) => !cacheRef.current.has(dir))
    if (missing.length > 0) void load(sandboxId, missing)
  }, [sandboxId, expanded, load])

  return tree
}
