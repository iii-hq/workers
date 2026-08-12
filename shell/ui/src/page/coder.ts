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
  total_lines?: number | null
  size?: number | null
  /** Unix permission bits, lower 9 bits of st_mode. */
  mode?: number | null
  mtime?: number | null
}

export interface CreateFileResult {
  path: string
  success: boolean
  bytes_written: number
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

export function coderTree(
  host: Host,
  path: string,
  includeHidden: boolean,
): Promise<TreeResponse> {
  return host.iii.trigger<TreeResponse>('coder::tree', {
    path,
    // Deep enough for real repos; the worker's per-folder limit and
    // output budget still bound the response (truncated stubs carry
    // hints the Files tab surfaces).
    max_depth: 25,
    // The worker default (50) fills with dot entries in home-shaped
    // folders — byte order sorts them first. 500 matches the editor
    // worker's browse call.
    per_folder_limit: 500,
    use_default_excludes: true,
    include_hidden: includeHidden,
  })
}

export function coderReadFile(
  host: Host,
  path: string,
): Promise<ReadFileResponse> {
  return host.iii.trigger<ReadFileResponse>('coder::read-file', { path })
}

/** Exact bytes, base64-encoded — the image-preview read. The override
    lifts the 128 KiB text budget; the worker clamps it to its
    max_read_bytes cap, so oversized files still fail loud (C218). */
export function coderReadFileBase64(
  host: Host,
  path: string,
): Promise<ReadFileResponse> {
  return host.iii.trigger<ReadFileResponse>('coder::read-file', {
    path,
    encoding: 'base64',
    max_output_bytes: 100_000_000,
  })
}

/** Whole-file save. `mode` (from a prior read) keeps permission bits —
    create-file would otherwise reset them to the 0644 default. */
export async function coderWriteFile(
  host: Host,
  path: string,
  content: string,
  mode?: number | null,
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
}

export function coderSearch(
  host: Host,
  { query, regex, ignoreCase, path }: SearchParams,
): Promise<SearchResponse> {
  return host.iii.trigger<SearchResponse>('coder::search', {
    query,
    regex,
    ignore_case: ignoreCase,
    path,
    search_content: true,
    search_paths: true,
  })
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
}

export function flattenTree(root: TreeNode): FlatTree {
  const paths: string[] = []
  const kinds = new Map<string, TreeNode['kind']>()
  const truncations: TreeTruncation[] = []

  const walk = (node: TreeNode, prefix: string) => {
    if (node.truncated) truncations.push(node.truncated)
    for (const child of node.children ?? []) {
      const childPath = prefix === '' ? child.name : `${prefix}/${child.name}`
      paths.push(child.kind === 'dir' ? `${childPath}/` : childPath)
      kinds.set(childPath, child.kind)
      walk(child, childPath)
    }
  }
  // The ROOT node's own name is never joined — child path = prefix + name.
  walk(root, '')

  return { paths, kinds, truncations }
}
