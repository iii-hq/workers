/* The explorer's flat tree as a value with pure transitions: lazy
   subtree merges and watcher-driven patches replace whole-snapshot
   refetches, so a workspace of thousands of files costs one small
   request per opened folder and one map update per change burst — not a
   multi-megabyte listing every 400 ms. */

import type { FlatTree, TreeNode, TreeTruncation } from './coder'
import { ancestorDirs, stripDirSlash } from './paths'

export const EMPTY_TREE: FlatTree = {
  paths: [],
  kinds: new Map(),
  truncations: [],
  loaded: new Set(),
}

export function emptyTree(): FlatTree {
  return { paths: [], kinds: new Map(), truncations: [], loaded: new Set() }
}

/** The change vocabulary of `shell::changed`, root-relative. */
export interface TreeChange {
  rel: string
  kind: 'created' | 'modified' | 'deleted' | string
  dir: boolean
}

/** Whether a folder's own children are known. A folder the snapshot cut
    off (`max_depth`, `max_nodes`, a default-exclude stub) is not, and
    expanding it should fetch. */
export function isDirLoaded(tree: FlatTree, dir: string): boolean {
  if (dir === '') return true
  return tree.loaded.has(stripDirSlash(dir))
}

/** Splice a freshly listed folder in: everything the tree held under it
    is replaced by `sub` (whose paths are relative to `dir`). */
export function mergeSubtree(tree: FlatTree, dir: string, sub: FlatTree): FlatTree {
  const base = stripDirSlash(dir)
  const prefix = base === '' ? '' : `${base}/`
  const paths: string[] = []
  const kinds = new Map<string, TreeNode['kind']>()
  const loaded = new Set<string>()
  for (const p of tree.paths) {
    if (prefix !== '' && stripDirSlash(p).startsWith(prefix)) continue
    if (prefix === '') continue
    paths.push(p)
  }
  for (const [p, k] of tree.kinds) {
    if (prefix === '' || !p.startsWith(prefix)) kinds.set(p, k)
  }
  for (const d of tree.loaded) {
    if (prefix === '' || !(d === base || d.startsWith(prefix))) loaded.add(d)
  }
  if (base !== '') {
    if (!kinds.has(base)) kinds.set(base, 'dir')
    if (!paths.includes(`${base}/`)) paths.push(`${base}/`)
    loaded.add(base)
  }
  for (const p of sub.paths) paths.push(prefix + p)
  for (const [p, k] of sub.kinds) kinds.set(prefix + p, k)
  for (const d of sub.loaded) loaded.add(prefix + d)
  const truncations = base === '' ? sub.truncations : [...tree.truncations, ...sub.truncations]
  return { paths, kinds, truncations, loaded }
}

function addPath(
  paths: string[],
  kinds: Map<string, TreeNode['kind']>,
  seen: Set<string>,
  rel: string,
  dir: boolean,
): boolean {
  let changed = false
  for (const ancestor of ancestorDirs(rel)) {
    const marker = `${ancestor}/`
    if (!seen.has(marker)) {
      seen.add(marker)
      paths.push(marker)
      changed = true
    }
    if (kinds.get(ancestor) !== 'dir') {
      kinds.set(ancestor, 'dir')
      changed = true
    }
  }
  const marker = dir ? `${rel}/` : rel
  if (!seen.has(marker)) {
    seen.add(marker)
    paths.push(marker)
    changed = true
  }
  const kind = dir ? 'dir' : 'file'
  if (kinds.get(rel) !== kind) {
    kinds.set(rel, kind)
    changed = true
  }
  return changed
}

/** Whether the deepest known ancestor of `rel` has a listed child set —
    the root always does. */
function parentListed(
  kinds: ReadonlyMap<string, TreeNode['kind']>,
  loaded: ReadonlySet<string>,
  rel: string,
): boolean {
  const ancestors = ancestorDirs(rel)
  for (let i = ancestors.length - 1; i >= 0; i--) {
    const dir = ancestors[i]
    if (kinds.has(dir)) return kinds.get(dir) === 'dir' && loaded.has(dir)
  }
  return true
}

/** Apply a burst of watcher events. Creations add the path (and any
    missing ancestors); deletions drop it and, for a folder, everything
    beneath it. Modifications are metadata-only and leave the shape alone.
    Returns the same tree when nothing changed, so React state stays put. */
export function applyTreeChanges(tree: FlatTree, changes: readonly TreeChange[]): FlatTree {
  if (changes.length === 0) return tree
  const paths = [...tree.paths]
  const kinds = new Map(tree.kinds)
  const loaded = new Set(tree.loaded)
  const seen = new Set(paths)
  let changed = false
  let removedAny = false
  for (const change of changes) {
    const rel = stripDirSlash(change.rel)
    if (rel === '') continue
    if (change.kind === 'deleted') {
      const fileMarker = rel
      const dirMarker = `${rel}/`
      const prefix = `${rel}/`
      const wasKnown = seen.has(fileMarker) || seen.has(dirMarker)
      if (!wasKnown) continue
      seen.delete(fileMarker)
      seen.delete(dirMarker)
      kinds.delete(rel)
      loaded.delete(rel)
      for (const key of [...kinds.keys()]) {
        if (key.startsWith(prefix)) {
          kinds.delete(key)
          loaded.delete(key)
          seen.delete(key)
          seen.delete(`${key}/`)
        }
      }
      removedAny = true
      changed = true
      continue
    }
    // created / modified / unknown kinds: make sure the path exists with
    // the right kind (a modify of an unseen path is a create we missed).
    // A path under a folder whose listing is unknown stays out: that
    // folder is fetched whole when it is expanded, and a lone child
    // would only misrepresent it meanwhile.
    if (!parentListed(kinds, loaded, rel)) continue
    const existingKind = kinds.get(rel)
    if (existingKind !== undefined && existingKind !== 'dir' && existingKind !== 'file') continue
    if (existingKind === 'dir' && !change.dir) {
      // A file replaced a directory: drop the old subtree first.
      const prefix = `${rel}/`
      seen.delete(prefix)
      for (const key of [...kinds.keys()]) {
        if (key.startsWith(prefix)) {
          kinds.delete(key)
          loaded.delete(key)
          seen.delete(key)
          seen.delete(`${key}/`)
        }
      }
      removedAny = true
      changed = true
    } else if (existingKind === 'file' && change.dir) {
      seen.delete(rel)
      removedAny = true
      changed = true
    }
    if (addPath(paths, kinds, seen, rel, change.dir)) changed = true
    if (change.dir && existingKind !== 'dir') loaded.add(rel)
  }
  if (!changed) return tree
  const nextPaths = removedAny ? paths.filter((p) => seen.has(p)) : paths
  return { paths: nextPaths, kinds, truncations: tree.truncations, loaded }
}

/** Folders whose listing the watcher says changed shape (a new or
    removed entry directly inside), for callers that keep per-folder
    listings elsewhere. */
export function changedDirsOf(changes: readonly TreeChange[]): Set<string> {
  const out = new Set<string>()
  for (const change of changes) {
    const rel = stripDirSlash(change.rel)
    const idx = rel.lastIndexOf('/')
    out.add(idx === -1 ? '' : rel.slice(0, idx))
  }
  return out
}

/** Truncation notes worth a word in the UI: everything but the
    default-exclude stubs, which are expected noise. */
export function visibleTruncations(truncations: readonly TreeTruncation[]): TreeTruncation[] {
  return truncations.filter((t) => t.reason !== 'default_exclude')
}
