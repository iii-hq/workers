/**
 * Typed calls into the editor worker.
 *
 * The page is a *view* over the worker's workspace, not the owner of it. Which
 * files are open, which folders are expanded and which root is active all live
 * in the worker (backed by the `state` worker), so an agent sees the same
 * workspace this page does and a reload does not lose it.
 *
 * Everything here is `editor::*`. Browsing delegates to shell inside the
 * worker rather than from the browser, so the jail decision stays in one place.
 */

import type { Host } from '@iii-dev/console-ui'

export interface Buffer {
  path: string
  mtime: number
  language: string
}

export interface WorkspaceView {
  root: string
  buffers: Buffer[]
  expanded: string[]
}

/**
 * shell's vocabulary, verbatim: a directory is `dir`, not `folder`.
 * Getting this wrong classes every directory as a file, and clicking one
 * tries to open it.
 */
export type NodeKind = 'file' | 'dir' | 'symlink' | 'other'

export interface TreeNode {
  name: string
  kind: NodeKind
  size: number
  mtime: number
  children?: TreeNode[]
}

export interface TreeResult {
  root: string
  path: string
  tree: { path: string; root: TreeNode }
  expanded: string[]
}

export interface StatusEntry {
  path: string
  index: string
  worktree: string
  staged: boolean
  renamed_from: string | null
}

export interface StatusReport {
  branch: string | null
  upstream: string | null
  ahead: number
  behind: number
  entries: StatusEntry[]
  clean: boolean
}

export interface OpenResult {
  path: string
  content: string
  language: string
  size: number
  mtime: number
  truncated: boolean
}

export interface SaveResult {
  path: string
  saved: boolean
  conflict: boolean
  mtime: number
  disk_mtime: number | null
  conflict_patch: string | null
  added: number
  removed: number
  created: boolean
}

export interface DiffResult {
  patch: string
  added: number
  removed: number
  identical: boolean
  truncated: boolean
}

export interface FindMatch {
  path: string
  score: number
  positions: number[]
}

export interface SearchHit {
  line: number
  text: string
}

export interface SearchFile {
  path: string
  hits: SearchHit[]
}

export interface SearchResult {
  files: SearchFile[]
  total: number
  truncated: boolean
}

export type SyncAction = 'fetch' | 'pull' | 'push'

export interface GitActionResult {
  ok?: boolean
  committed?: boolean
  summary: string
  ahead?: number
  behind?: number
}

export interface FindResult {
  matches: FindMatch[]
  scanned: number
  truncated: boolean
  from_git: boolean
}

export function createApi(host: Host) {
  const call = <T>(fn: string, payload: Record<string, unknown>, timeoutMs = 20_000) =>
    host.iii.trigger<T>(fn, payload, { timeoutMs })

  return {
    workspace: () => call<WorkspaceView>('editor::workspace::get', {}),
    openWorkspace: (root: string) => call<WorkspaceView>('editor::workspace::open', { root }, 30_000),
    // Expansion is part of the shared workspace, so toggling a folder is a
    // call rather than local state: it survives a reload and both surfaces
    // agree on it.
    tree: (opts: { path?: string; maxDepth?: number; expand?: string[]; collapse?: string[] } = {}) =>
      call<TreeResult>(
        'editor::tree',
        {
          ...(opts.path ? { path: opts.path } : {}),
          max_depth: opts.maxDepth ?? 4,
          ...(opts.expand ? { expand: opts.expand } : {}),
          ...(opts.collapse ? { collapse: opts.collapse } : {}),
        },
        30_000,
      ),
    closeBuffer: (path: string) =>
      call<{ closed: boolean; root: string; buffers: Buffer[] }>('editor::buffers::close', {
        path,
      }),
    open: (path: string) => call<OpenResult>('editor::open', { path }, 30_000),
    save: (path: string, content: string, expectedMtime: number | null) =>
      call<SaveResult>(
        'editor::save',
        { path, content, ...(expectedMtime === null ? {} : { expected_mtime: expectedMtime }) },
        30_000,
      ),
    find: (query: string, limit: number) => call<FindResult>('editor::find', { query, limit }),
    diff: (before: string, after: string, path?: string) =>
      call<DiffResult>('editor::diff', { before, after, ...(path ? { path } : {}) }),
    status: () => call<StatusReport>('editor::git::status', {}),
    hunks: (path: string) =>
      call<{
        path: string
        added: number
        removed: number
        untracked: boolean
        patch: string
        // Context is requested explicitly: the worker defaults to zero so
        // that `hunks` stays gutter-accurate, but this view renders the patch
        // for a person to read.
      }>('editor::git::hunks', { path, against: 'head', context_lines: 3 }),
    show: (path: string) =>
      call<{ path: string; rev: string; content: string; exists: boolean }>('editor::git::show', {
        path,
      }),
    search: (pattern: string, ignoreCase: boolean) =>
      call<SearchResult>('editor::search', { pattern, ignore_case: ignoreCase }, 60_000),
    commit: (message: string) => call<GitActionResult>('editor::git::commit', { message }, 60_000),
    sync: (action: SyncAction) => call<GitActionResult>('editor::git::sync', { action }, 120_000),
    stash: (action: 'push' | 'pop') => call<GitActionResult>('editor::git::stash', { action }, 60_000),
  }
}

export type Api = ReturnType<typeof createApi>

/**
 * Readable text for a rejected bus call.
 *
 * The engine rejects with an object (`{code, message}`), and `String(err)` on
 * that renders the literal "[object Object]" — the message inside it is the
 * part the user needs.
 */
export function errorText(err: unknown): string {
  if (typeof err === 'string') return err
  if (err instanceof Error) return err.message
  if (typeof err === 'object' && err !== null) {
    const rec = err as Record<string, unknown>
    if (typeof rec.message === 'string') return rec.message
    try {
      return JSON.stringify(err)
    } catch {
      return 'unknown error'
    }
  }
  return String(err)
}

/** True when the failure is "this directory is not a git repository". */
export function isNotARepo(message: string): boolean {
  return message.includes('not a git repository')
}

export interface FlatNode {
  path: string
  name: string
  kind: NodeKind
  depth: number
}

/**
 * Flatten the tree into visible rows, honouring `expanded`.
 *
 * A collapsed folder contributes its own row and nothing beneath it — that is
 * what keeps the tree navigable in a large repo instead of a thousand-row dump.
 */
export function isDir(node: { kind: NodeKind; children?: unknown }): boolean {
  return node.kind === 'dir' || node.children !== undefined
}

export function visibleRows(root: TreeNode, expanded: Set<string>): FlatNode[] {
  const out: FlatNode[] = []
  const walk = (node: TreeNode, prefix: string, depth: number) => {
    const children = [...(node.children ?? [])].sort((a, b) => {
      const dirA = isDir(a)
      if (dirA !== isDir(b)) return dirA ? -1 : 1
      return a.name.localeCompare(b.name)
    })
    for (const child of children) {
      const path = prefix ? `${prefix}/${child.name}` : child.name
      out.push({ path, name: child.name, kind: child.kind, depth })
      if (isDir(child) && expanded.has(path)) walk(child, path, depth + 1)
    }
  }
  walk(root, '', 0)
  return out
}

/** Status vocabulary reduced to the one letter a gutter shows. */
export function statusMark(entry: StatusEntry): string {
  const label = entry.worktree !== 'unchanged' ? entry.worktree : entry.index
  if (label === 'untracked') return '?'
  if (label === 'conflicted') return '!'
  return label.charAt(0).toUpperCase()
}
