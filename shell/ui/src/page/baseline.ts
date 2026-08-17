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
  const relPaths = [...tree.kinds]
    .filter(([path, kind]) => kind === 'file' && includePath(path))
    .map(([path]) => path)
    .slice(0, SNAPSHOT_MAX_FILES)
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
  }
}
