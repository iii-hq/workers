/* Thin typed wrappers over the worker's own `coder::*` functions — the
   explorer page acts by invoking them through the tab's bus client
   (`host.iii.trigger`). Shapes mirror workers/shell/src/code/functions
   (the goldens under shell/tests/golden/schemas pin them); only the
   fields the page reads are declared. */

import type { Host } from '@iii-dev/console-ui'

export interface CoderInfo {
  /** Canonical absolute allowed roots; index 0 is the primary root. */
  base_paths: string[]
  primary_root: string
}

export interface WorkspaceValidateResponse {
  path: string
}

export interface TreeTruncation {
  /** "per_folder_limit" | "max_depth" | "default_exclude" | "max_nodes". */
  reason: string
  shown: number
  total?: number | null
  hint: string
}

export interface TreeNode {
  name: string
  kind: 'file' | 'dir' | 'symlink' | 'other'
  size: number
  mtime: number
  non_accessible?: boolean
  children?: TreeNode[] | null
  truncated?: TreeTruncation | null
}

export interface TreeResponse {
  /** Canonical absolute path of the requested folder (= root node). */
  path: string
  root: TreeNode
}

export interface ReadFileResponse {
  path?: string | null
  content?: string | null
  is_utf8?: boolean | null
  more_lines?: boolean | null
  lines_returned?: number | null
  total_lines?: number | null
  size?: number | null
  /** Unix permission bits, lower 9 bits of st_mode. */
  mode?: number | null
  mtime?: number | null
  /** Opaque exact-content identity for conflict-safe whole-file saves. */
  revision?: string | null
}

export interface BatchReadFileResult extends ReadFileResponse {
  path: string
  success: boolean
}

export interface CreateFileResult {
  path: string
  success: boolean
  bytes_written: number
  /** Opaque identity of the exact bytes written. */
  revision?: string | null
  error?: { code: string; message: string } | null
}

export interface ContentMatch {
  path: string
  line: number
  column: number
  text: string
  before?: string[] | null
  after?: string[] | null
}

export interface SearchResponse {
  content_matches: ContentMatch[]
  /** `kind` distinguishes folder name-matches from file ones. */
  path_matches: { path: string; kind?: 'file' | 'dir' }[]
  truncated: boolean
}

export function coderInfo(host: Host): Promise<CoderInfo> {
  return host.iii.trigger<CoderInfo>('coder::info', {})
}

/** Resolve a user/chat-selected directory to the worker's canonical path.
    This keeps `/tmp` and `/private/tmp` aliases identical to the watcher. */
export function workspaceValidate(
  host: Host,
  path: string,
): Promise<WorkspaceValidateResponse> {
  return host.iii.trigger<WorkspaceValidateResponse>(
    'shell::workspace::validate',
    { path },
  )
}

/** How deep the first listing of a root goes. Shallow on purpose: the
    explorer fetches a folder's children when it is expanded, so a
    workspace of any size opens with one small response. */
export const TREE_INITIAL_DEPTH = 3
/** Depth of a lazy folder fetch: the folder's children plus one level
    below, so the next expansion is usually instant. */
export const TREE_EXPAND_DEPTH = 2
export const TREE_PER_FOLDER_LIMIT = 500

export function coderTree(
  host: Host,
  path: string,
  includeHidden: boolean,
  maxDepth: number = TREE_INITIAL_DEPTH,
): Promise<TreeResponse> {
  return host.iii.trigger<TreeResponse>('coder::tree', {
    path,
    max_depth: maxDepth,
    // The worker default (50) fills with dot entries in home-shaped
    // folders — byte order sorts them first. 500 matches the editor
    // worker's browse call.
    per_folder_limit: TREE_PER_FOLDER_LIMIT,
    use_default_excludes: true,
    include_hidden: includeHidden,
  })
}

export interface ReadFileOptions {
  /** Per-call full-read budget (bytes); the worker clamps it to its
      max_read_bytes. Omit for the worker default (128 KiB). */
  maxOutputBytes?: number
}

export function coderReadFile(
  host: Host,
  path: string,
  options: ReadFileOptions = {},
): Promise<ReadFileResponse> {
  return host.iii.trigger<ReadFileResponse>('coder::read-file', {
    path,
    ...(options.maxOutputBytes !== undefined
      ? { max_output_bytes: options.maxOutputBytes }
      : {}),
  })
}

/** A line window of a file — the read-only view of a file too large for
    the editor budget. */
export function coderReadWindow(
  host: Host,
  path: string,
  lineFrom: number,
  lineTo: number,
): Promise<ReadFileResponse> {
  return host.iii.trigger<ReadFileResponse>('coder::read-file', {
    path,
    line_from: lineFrom,
    line_to: lineTo,
  })
}

/** Metadata only: size, mode, mtime, total_lines, is_utf8. */
export function coderStat(host: Host, path: string): Promise<ReadFileResponse> {
  return host.iii.trigger<ReadFileResponse>('coder::read-file', { path, stat: true })
}

/** Best-effort batch snapshot used to capture pre-change baselines for a
    live review burst. Individual failures (deleted/binary/oversized)
    remain isolated to their entry. */
export async function coderReadFiles(
  host: Host,
  paths: readonly string[],
): Promise<BatchReadFileResult[]> {
  if (paths.length === 0) return []
  const out = await host.iii.trigger<{ results?: BatchReadFileResult[] }>(
    'coder::read-file',
    { paths },
  )
  return out.results ?? []
}

/** Existence/metadata probe for a watcher burst. Unlike full reads this
    remains reliable for binary and oversized files. */
export async function coderStatFiles(
  host: Host,
  paths: readonly string[],
): Promise<BatchReadFileResult[]> {
  if (paths.length === 0) return []
  const out = await host.iii.trigger<{ results?: BatchReadFileResult[] }>(
    'coder::read-file',
    { paths: paths.map((path) => ({ path, stat: true })) },
  )
  return out.results ?? []
}

/** Exact bytes, base64-encoded — the image-preview read. The override
    lifts the 128 KiB text budget to a preview-sized ceiling (the base64
    string lives in the per-tab cache plus a data: URL — an unbounded
    read would hold two copies of an arbitrarily large file); the worker
    clamps further to its max_read_bytes cap, and oversized files still
    fail loud (C218). */
export function coderReadFileBase64(
  host: Host,
  path: string,
): Promise<ReadFileResponse> {
  return host.iii.trigger<ReadFileResponse>('coder::read-file', {
    path,
    encoding: 'base64',
    max_output_bytes: 16_000_000,
  })
}

/** Create a brand-new empty file, parents included; an existing file at
    the path is an error rather than silently emptied. */
export async function coderCreateNewFile(
  host: Host,
  path: string,
): Promise<CreateFileResult> {
  const out = await host.iii.trigger<{ results: CreateFileResult[] }>(
    'coder::create-file',
    { files: [{ path, content: '', parents: true }] },
  )
  const result = out.results?.[0]
  if (!result) throw new Error('coder::create-file returned no result')
  if (!result.success) {
    throw new Error(result.error?.message ?? `could not create ${path}`)
  }
  return result
}

/** Create a folder, parents included, through the shell's fs surface. */
export async function shellCreateFolder(
  host: Host,
  path: string,
): Promise<void> {
  await host.iii.trigger('shell::fs::mkdir', { path, parents: true })
}

/** Whole-file save. `mode` (from a prior read) keeps permission bits —
    create-file would otherwise reset them to the 0644 default. */
export async function coderWriteFile(
  host: Host,
  path: string,
  content: string,
  mode?: number | null,
  expectedRevision?: string | null,
): Promise<CreateFileResult> {
  const out = await host.iii.trigger<{ results: CreateFileResult[] }>(
    'coder::create-file',
    {
      files: [
        {
          path,
          content,
          overwrite: true,
          ...(mode != null ? { mode: `0${mode.toString(8)}` } : {}),
          ...(expectedRevision != null
            ? { expected_revision: expectedRevision }
            : {}),
        },
      ],
    },
  )
  const result = out.results?.[0]
  if (!result) throw new Error('coder::create-file returned no result')
  return result
}

export interface SearchParams {
  query: string
  regex: boolean
  ignoreCase: boolean
  path: string
  /** Default true; false asks for path matches only (quick open). */
  searchContent?: boolean
  /** Default true. */
  searchPaths?: boolean
  /** Root-relative globs the paths must match, e.g. every TypeScript file. */
  includeGlobs?: readonly string[]
  excludeGlobs?: readonly string[]
  maxMatches?: number
  /** Skip files `.gitignore` rules hide, like an editor's search. */
  respectGitignore?: boolean
  /** Rank path matches by fuzzy score, best first (quick open). */
  fuzzyPaths?: boolean
  contextLinesBefore?: number
  contextLinesAfter?: number
}

export function coderSearch(
  host: Host,
  {
    query,
    regex,
    ignoreCase,
    path,
    searchContent = true,
    searchPaths = true,
    includeGlobs,
    excludeGlobs,
    maxMatches,
    respectGitignore,
    fuzzyPaths,
    contextLinesBefore,
    contextLinesAfter,
  }: SearchParams,
): Promise<SearchResponse> {
  return host.iii.trigger<SearchResponse>('coder::search', {
    query,
    regex,
    ignore_case: ignoreCase,
    path,
    search_content: searchContent,
    search_paths: searchPaths,
    ...(includeGlobs && includeGlobs.length > 0 ? { include_globs: includeGlobs } : {}),
    ...(excludeGlobs && excludeGlobs.length > 0 ? { exclude_globs: excludeGlobs } : {}),
    ...(maxMatches !== undefined ? { max_matches: maxMatches } : {}),
    ...(respectGitignore ? { respect_gitignore: true } : {}),
    ...(fuzzyPaths ? { fuzzy_paths: true } : {}),
    ...(contextLinesBefore ? { context_lines_before: contextLinesBefore } : {}),
    ...(contextLinesAfter ? { context_lines_after: contextLinesAfter } : {}),
  })
}

export interface DeleteResult {
  path: string
  success: boolean
  removed: boolean
  error?: { code: string; message: string } | null
}

/** Remove files or folders; folders need `recursive` when non-empty. */
export async function coderDelete(
  host: Host,
  paths: readonly string[],
  recursive: boolean,
): Promise<DeleteResult[]> {
  const out = await host.iii.trigger<{ results?: DeleteResult[] }>('coder::delete-file', {
    paths,
    recursive,
  })
  return out.results ?? []
}

export interface MoveResult {
  from: string
  to: string
  success: boolean
  error?: { code: string; message: string } | null
}

/** Rename or move one entry; refuses to overwrite an existing target. */
export async function coderMove(host: Host, from: string, to: string): Promise<MoveResult> {
  const out = await host.iii.trigger<{ results?: MoveResult[] }>('coder::move', {
    files: [{ from, to, overwrite: false, parents: true }],
  })
  const result = out.results?.[0]
  if (!result) throw new Error('coder::move returned no result')
  if (!result.success) throw new Error(result.error?.message ?? `could not move ${from}`)
  return result
}

/* ── path helpers ───────────────────────────────────────────────────── */

export function joinPath(root: string, rel: string): string {
  if (rel === '' || rel === '.') return root
  return root.endsWith('/') ? `${root}${rel}` : `${root}/${rel}`
}

/** Absolute → root-relative (the tree/search results are canonical
    absolute; the FileTree speaks root-relative). */
export function relativeTo(root: string, abs: string): string {
  const prefix = root.endsWith('/') ? root : `${root}/`
  if (abs === root) return ''
  return abs.startsWith(prefix) ? abs.slice(prefix.length) : abs
}

export interface FlatTree {
  /** Root-relative FileTree input paths. The tree's path model treats a
      bare path as a FILE — a directory appended bare would collide with
      its own children ("Path collides with an existing file") — so
      directories carry the explicit trailing-slash marker. */
  paths: string[]
  /** kind by slash-less root-relative path — the open-on-select gate. */
  kinds: Map<string, TreeNode['kind']>
  /** Truncation hints, for the "partial listing" note. */
  truncations: TreeTruncation[]
  /** Slash-less dirs whose children the snapshot actually listed. A dir
      absent here was cut off (depth, node budget, default exclude) and
      needs its own fetch when expanded. */
  loaded: Set<string>
}

/** Whether a listed node's children are known in full or in part. Depth
    and budget cuts leave the folder unlisted; a per-folder cap still
    listed a first page, which is all the explorer can show anyway. */
function childrenListed(node: TreeNode): boolean {
  if (node.children == null) return false
  const reason = node.truncated?.reason
  return reason !== 'max_depth' && reason !== 'max_nodes' && reason !== 'default_exclude'
}

export function flattenTree(root: TreeNode): FlatTree {
  const paths: string[] = []
  const kinds = new Map<string, TreeNode['kind']>()
  const truncations: TreeTruncation[] = []
  const loaded = new Set<string>()

  const walk = (node: TreeNode, prefix: string) => {
    if (node.truncated) truncations.push(node.truncated)
    for (const child of node.children ?? []) {
      const childPath = prefix === '' ? child.name : `${prefix}/${child.name}`
      paths.push(child.kind === 'dir' ? `${childPath}/` : childPath)
      kinds.set(childPath, child.kind)
      if (child.kind === 'dir' && childrenListed(child)) loaded.add(childPath)
      walk(child, childPath)
    }
  }
  // The ROOT node's own name is never joined — child path = prefix + name.
  walk(root, '')

  return { paths, kinds, truncations, loaded }
}
