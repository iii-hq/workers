/* The explorer's tree as one hook: a shallow first listing, per-folder
   fetches on expansion, and watcher patches applied in place. The page
   keeps a generation counter so a root switch retires every in-flight
   fetch. The hook is the tree's only writer: every change goes through
   `commit`, which updates a synchronous ref first and React state second,
   so a chain of folder fetches (a deep reveal) never reads a stale tree. */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  coderTree,
  type FlatTree,
  flattenTree,
  joinPath,
  TREE_EXPAND_DEPTH,
  TREE_INITIAL_DEPTH,
} from './coder'
import { ancestorDirs, stripDirSlash } from './paths'
import {
  applyTreeChanges,
  emptyTree,
  isDirLoaded,
  mergeSubtree,
  type TreeChange,
} from './tree-model'

export interface WorkspaceTree {
  /** null until the first listing of the current root lands. */
  tree: FlatTree | null
  /** Re-list the whole root (shallow); expanded folders refetch on demand. */
  refresh: () => void
  /** Make sure a folder's children are known; resolves once they are. */
  ensureDir: (dir: string) => Promise<void>
  /** Load every folder on the way to `rel` so it can be revealed. */
  ensurePath: (rel: string) => Promise<void>
  /** Apply a watcher burst; folders whose listing is unknown stay unknown. */
  applyChanges: (changes: readonly TreeChange[]) => void
  /** Re-list one folder (after a rename inside it, say). */
  reloadDir: (dir: string) => Promise<void>
  loadingDirs: ReadonlySet<string>
}

export function useWorkspaceTree(
  host: Host,
  root: string | null,
  showHidden: boolean,
  generationRef: React.MutableRefObject<number>,
): WorkspaceTree {
  const [tree, setTree] = useState<FlatTree | null>(null)
  const treeRef = useRef<FlatTree | null>(null)
  const [loadingDirs, setLoadingDirs] = useState<ReadonlySet<string>>(new Set())
  const inflightRef = useRef(new Map<string, Promise<void>>())

  const commit = useCallback((update: (prev: FlatTree | null) => FlatTree | null) => {
    const next = update(treeRef.current)
    if (next === treeRef.current) return
    treeRef.current = next
    setTree(next)
  }, [])

  const fetchDir = useCallback(
    (dir: string, depth: number): Promise<void> => {
      if (root === null) return Promise.resolve()
      const key = stripDirSlash(dir)
      const existing = inflightRef.current.get(key)
      if (existing) return existing
      const generation = generationRef.current
      const currentRoot = root
      setLoadingDirs((prev) => new Set(prev).add(key))
      const promise = coderTree(host, joinPath(currentRoot, key), showHidden, depth)
        .then((out) => {
          if (generationRef.current !== generation) return
          const sub = flattenTree(out.root)
          commit((prev) => {
            if (key === '') return sub
            // The folder went away meanwhile: nothing to splice under.
            if (prev !== null && !prev.kinds.has(key)) return prev
            return mergeSubtree(prev ?? emptyTree(), key, sub)
          })
        })
        .catch(() => {
          if (generationRef.current !== generation) return
          if (key === '') {
            commit(() => emptyTree())
            return
          }
          // An inaccessible folder: mark it listed-and-empty so expanding
          // it again does not refetch on every burst; a change under it
          // drops the mark and retries.
          commit((prev) =>
            prev === null || !prev.kinds.has(key) ? prev : mergeSubtree(prev, key, emptyTree()),
          )
        })
        .finally(() => {
          inflightRef.current.delete(key)
          setLoadingDirs((prev) => {
            if (!prev.has(key)) return prev
            const next = new Set(prev)
            next.delete(key)
            return next
          })
        })
      inflightRef.current.set(key, promise)
      return promise
    },
    [host, root, showHidden, generationRef, commit],
  )

  // A new root or hidden-filter flips the whole listing; every lazily
  // fetched folder was listed with the old filter and is stale.
  useEffect(() => {
    treeRef.current = null
    setTree(null)
    inflightRef.current.clear()
    setLoadingDirs(new Set())
    if (root === null) return
    void fetchDir('', TREE_INITIAL_DEPTH)
  }, [root, fetchDir])

  const refresh = useCallback(() => {
    if (root === null) return
    inflightRef.current.clear()
    void fetchDir('', TREE_INITIAL_DEPTH)
  }, [root, fetchDir])

  const ensureDir = useCallback(
    (dir: string): Promise<void> => {
      const key = stripDirSlash(dir)
      const current = treeRef.current
      if (current !== null && isDirLoaded(current, key)) return Promise.resolve()
      return fetchDir(key, TREE_EXPAND_DEPTH)
    },
    [fetchDir],
  )

  const ensurePath = useCallback(
    async (rel: string): Promise<void> => {
      for (const dir of ancestorDirs(rel)) {
        const current = treeRef.current
        // A file where a folder was expected: nothing below it to load.
        if (current?.kinds.has(dir) && current.kinds.get(dir) !== 'dir') return
        await ensureDir(dir)
      }
    },
    [ensureDir],
  )

  const applyChanges = useCallback(
    (changes: readonly TreeChange[]) => {
      commit((prev) => (prev === null ? prev : applyTreeChanges(prev, changes)))
    },
    [commit],
  )

  const reloadDir = useCallback(
    (dir: string): Promise<void> => {
      const key = stripDirSlash(dir)
      inflightRef.current.delete(key)
      return fetchDir(key, key === '' ? TREE_INITIAL_DEPTH : TREE_EXPAND_DEPTH)
    },
    [fetchDir],
  )

  return useMemo(
    () => ({ tree, refresh, ensureDir, ensurePath, applyChanges, reloadDir, loadingDirs }),
    [tree, refresh, ensureDir, ensurePath, applyChanges, reloadDir, loadingDirs],
  )
}
