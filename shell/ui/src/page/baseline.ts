import type { Host } from '@iii-dev/console-ui'
import {
  coderReadFiles,
  flattenTree,
  joinPath,
  relativeTo,
  type TreeNode,
  type TreeResponse,
} from './coder'

const SNAPSHOT_MAX_FILES = 500
const SNAPSHOT_BATCH_SIZE = 40

export interface WorkspaceBaseline {
  /** Captured tree view, including dotfiles independently of display. */
  kinds: ReadonlyMap<string, 'file' | 'dir' | 'symlink' | 'other'>
  /** UTF-8 whole-file bodies captured before Harness can execute tools. */
  contents: ReadonlyMap<string, string>
  /** False when coder::tree may have omitted reviewable descendants. */
  complete: boolean
  /** How much of the reviewable inventory the body snapshot could hold. A
      capped snapshot still classifies every path — only bodies are missing —
      so this stays separate from `complete`, which drives new-vs-existing. */
  coverage: WorkspaceBaselineCoverage
}

export interface WorkspaceBaselineCoverage {
  /** Reviewable files the inventory offered. */
  candidates: number
  /** Files whose body the snapshot actually holds. */
  captured: number
  /** True when the candidate count exceeded the per-turn body budget. */
  capped: boolean
}

export interface WorkspaceBaselinePathState {
  priorKind: 'file' | 'dir' | null
  /** Whether the inventory proves this classification rather than infers it. */
  exact: boolean
}

function baselineTree(host: Host, root: string): Promise<TreeResponse> {
  return host.iii.trigger<TreeResponse>('coder::tree', {
    path: root,
    max_depth: 25,
    per_folder_limit: 500,
    // Keep large known-noise subtrees out of the worker's global node budget.
    // Path-aware completeness below distinguishes harmless ignored stubs from
    // configurable excludes that still contain reviewable source.
    use_default_excludes: true,
    include_hidden: true,
  })
}

function inventoryCompleteForReview(
  root: TreeNode,
  includePath: (path: string) => boolean,
): boolean {
  let complete = true
  const walk = (node: TreeNode, path: string) => {
    if (
      node.truncated &&
      (node.truncated.reason !== 'default_exclude' || includePath(path))
    ) {
      complete = false
    }
    for (const child of node.children ?? []) {
      const childPath = path === '' ? child.name : `${path}/${child.name}`
      walk(child, childPath)
    }
  }
  walk(root, '')
  return complete
}

/** Classify a path against the captured inventory. An absent path is known
    new only when the inventory was complete. When it was truncated, omitted
    existing files and genuinely new files are indistinguishable, so classify
    conservatively and let the missing body fail closed downstream. */
export function classifyWorkspaceBaselinePath(
  baseline: Pick<WorkspaceBaseline, 'kinds' | 'complete'>,
  path: string,
): WorkspaceBaselinePathState {
  const kind = baseline.kinds.get(path)
  if (kind === 'dir') return { priorKind: 'dir', exact: true }
  if (kind !== undefined) return { priorKind: 'file', exact: true }
  return baseline.complete
    ? { priorKind: null, exact: true }
    : { priorKind: 'file', exact: false }
}

/** Reviewable files in the order the body budget should spend itself: most
    recently modified first. A turn edits the working set, not the
    alphabetically first 500 paths, so recency buys far more coverage than
    tree order on a large or shared root. Equal mtimes keep tree order. */
export function prioritizedBaselineCandidates(
  root: TreeNode,
  includePath: (path: string) => boolean,
): string[] {
  const candidates: { path: string; mtime: number; order: number }[] = []
  const walk = (node: TreeNode, prefix: string) => {
    for (const child of node.children ?? []) {
      const path = prefix === '' ? child.name : `${prefix}/${child.name}`
      if (child.kind === 'file' && includePath(path)) {
        candidates.push({ path, mtime: child.mtime, order: candidates.length })
      }
      walk(child, path)
    }
  }
  walk(root, '')
  return candidates
    .sort((left, right) =>
      left.mtime === right.mtime
        ? left.order - right.order
        : right.mtime - left.mtime,
    )
    .map((candidate) => candidate.path)
}

/**
 * Capture a turn baseline at Harness's awaited pre-turn boundary. The result
 * is built locally and published atomically, so tree refreshes cannot cancel a
 * partially populated snapshot. Worker default excludes protect the global
 * tree budget; path-aware completeness below prevents configurable reviewable
 * excludes from being mistaken for a complete inventory.
 */
export async function captureWorkspaceBaseline(
  host: Host,
  root: string,
  includePath: (path: string) => boolean,
): Promise<WorkspaceBaseline> {
  const treeResponse = await baselineTree(host, root)
  const tree = flattenTree(treeResponse.root)
  const candidates = prioritizedBaselineCandidates(treeResponse.root, includePath)
  const relPaths = candidates.slice(0, SNAPSHOT_MAX_FILES)
  const contents = new Map<string, string>()

  for (let start = 0; start < relPaths.length; start += SNAPSHOT_BATCH_SIZE) {
    const batch = relPaths.slice(start, start + SNAPSHOT_BATCH_SIZE)
    const results = await coderReadFiles(
      host,
      batch.map((path) => joinPath(root, path)),
    ).catch(() => [])
    for (const result of results) {
      if (!result.success || result.is_utf8 === false || result.more_lines === true) continue
      contents.set(relativeTo(root, result.path), result.content ?? '')
    }
  }

  return {
    kinds: tree.kinds,
    contents,
    // A default-exclude stub is harmless only when the same review predicate
    // rejects that subtree. Capacity, depth, I/O, and reviewable default
    // excludes remain fail-closed.
    complete: inventoryCompleteForReview(treeResponse.root, includePath),
    coverage: {
      candidates: candidates.length,
      captured: contents.size,
      capped: candidates.length > SNAPSHOT_MAX_FILES,
    },
  }
}
