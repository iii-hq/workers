import type { FlatTree } from './coder'
import type { GitChange } from './git'

const INVISIBLE_SEPARATOR = '\u2063'

export interface ChangeTreeRow {
  treePath: string
  change: GitChange
}

/** Give file/directory path replacements distinct internal ids while
    leaving their visible labels identical. */
export function changeTreeRows(changes: readonly GitChange[]): ChangeTreeRow[] {
  const files = changes.filter((change) => change.path !== '' && !change.path.endsWith('/'))
  const directoryCollisions = new Set(
    files
      .filter((candidate) => files.some((other) => other.path.startsWith(`${candidate.path}/`)))
      .map((change) => change.path),
  )
  const used = new Set<string>()
  return files.map((change) => {
    let treePath = directoryCollisions.has(change.path) ? `${change.path}${INVISIBLE_SEPARATOR}` : change.path
    while (used.has(treePath)) treePath += INVISIBLE_SEPARATOR
    used.add(treePath)
    return { treePath, change }
  })
}

/** Parent directories for changed files, shallowest first. The review
    tree expands these once when a fresh change set arrives so every
    changed row is immediately visible in its real hierarchy. */
export function changedParentDirs(changes: readonly GitChange[]): string[] {
  const dirs = new Set<string>()
  for (const change of changes) {
    const segments = change.path.split('/')
    for (let i = 1; i < segments.length; i++) {
      dirs.add(segments.slice(0, i).join('/'))
    }
  }
  return [...dirs]
}

/** Merge review-only rows into the filesystem tree. This matters for
    deleted files (and their now-empty parent directories): the live
    review still has a diff even though coder::tree cannot list them. */
export function withReviewChanges(tree: FlatTree | null, changes: readonly GitChange[]): FlatTree | null {
  if (tree === null || changes.length === 0) return tree

  const paths = [...tree.paths]
  const kinds = new Map(tree.kinds)
  const seen = new Set(paths)

  for (const change of changes) {
    if (change.path === '' || change.path.endsWith('/')) continue
    const segments = change.path.split('/')
    // A file-to-directory replacement can legitimately leave Git with a
    // deleted `foo` while disk now contains `foo/`. The tree cannot hold
    // both ids, so the live directory wins; the Git tab still exposes the
    // historical deletion record.
    if (seen.has(`${change.path}/`) || kinds.get(change.path) === 'dir') continue
    let blocked = false
    for (let i = 1; i < segments.length; i++) {
      const dir = segments.slice(0, i).join('/')
      const treePath = `${dir}/`
      if (seen.has(dir) && kinds.get(dir) !== 'dir') {
        blocked = true
        break
      }
      if (!seen.has(treePath)) {
        seen.add(treePath)
        paths.push(treePath)
      }
      if (!kinds.has(dir)) kinds.set(dir, 'dir')
    }
    if (blocked) continue
    if (!seen.has(change.path)) {
      seen.add(change.path)
      paths.push(change.path)
    }
    kinds.set(change.path, 'file')
  }

  return { paths, kinds, truncations: tree.truncations }
}
