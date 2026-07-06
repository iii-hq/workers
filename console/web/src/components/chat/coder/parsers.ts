/**
 * Zod schemas + envelope helpers for every `coder::*` function (v0.4.1 wire).
 *
 * Wire source (one Rust handler per function):
 *   workers/coder/src/functions/create_file.rs  -> CreateFileInput/Output
 *   workers/coder/src/functions/update_file.rs  -> UpdateFileInput/Output
 *   workers/coder/src/functions/delete_file.rs  -> DeleteFileInput/Output
 *   workers/coder/src/functions/move_file.rs    -> MoveFileInput/Output
 *   workers/coder/src/functions/read_file.rs    -> ReadFileInput/Output
 *   workers/coder/src/functions/search.rs       -> SearchInput/Output
 *   workers/coder/src/functions/tree.rs         -> TreeInput/Output
 *   workers/coder/src/functions/list_folder.rs  -> ListFolderInput/Output
 *   workers/coder/src/functions/info.rs         -> InfoInput/Output
 *   workers/coder/src/functions/apply_patch.rs  -> ApplyPatchInput/Output
 *   workers/coder/src/functions/context.rs      -> ContextInput/Output
 *   workers/coder/src/functions/worktree.rs     -> WorktreeAdd/RemoveInput/Output
 *
 * Schemas are non-strict so additive wire fields don't break the UI.
 * Every response-side Rust `Option<T>` carries `skip_serializing_if =
 * "Option::is_none"`, so unset fields are OMITTED (never `null`) — we
 * model them as `.nullish()` for forward tolerance either way.
 */
import { z } from 'zod'
import {
  safeParseRequest,
  safeParseResponse,
  unwrapEnvelope,
} from '@/components/chat/sandbox/parsers'

export { safeParseRequest, safeParseResponse, unwrapEnvelope }

/* ---------------- function ids ---------------- */

/** Explicit allowlist — never prefix-match (would be accidentally too broad). */
export const CODER_FUNCTION_IDS = [
  'coder::create-file',
  'coder::update-file',
  'coder::delete-file',
  'coder::move',
  'coder::read-file',
  'coder::search',
  'coder::tree',
  'coder::list-folder',
  'coder::info',
  'coder::apply-patch',
  'coder::context',
  'coder::worktree-add',
  'coder::worktree-remove',
] as const
export type CoderFunctionId = (typeof CODER_FUNCTION_IDS)[number]

const CODER_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  CODER_FUNCTION_IDS,
)

export function isCoderFunction(id: string): id is CoderFunctionId {
  return CODER_FUNCTION_ID_SET.has(id)
}

/** Mutators gate approval previews; everything else is read-only. */
export const CODER_MUTATE_FUNCTION_IDS = [
  'coder::create-file',
  'coder::update-file',
  'coder::delete-file',
  'coder::move',
  'coder::apply-patch',
] as const
export type CoderMutateFunctionId = (typeof CODER_MUTATE_FUNCTION_IDS)[number]

const CODER_MUTATE_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  CODER_MUTATE_FUNCTION_IDS,
)

export function isCoderMutateFunction(id: string): id is CoderMutateFunctionId {
  return CODER_MUTATE_FUNCTION_ID_SET.has(id)
}

/* ---------------- shared ---------------- */

/**
 * Per-entry structured error shared by every coder response. `code` is
 * stable for branching (C210 bad input, C211 not-found-or-denied,
 * C213 size/budget exceeded, C217 already-exists); `message` is
 * agent-oriented and names the corrective next call — show verbatim.
 */
export const wireErrorSchema = z.object({
  code: z.string(),
  message: z.string(),
})
export type WireError = z.infer<typeof wireErrorSchema>

/** `list_folder.rs::EntryKind` / `tree.rs::NodeKind` — drives icons. */
export const entryKindSchema = z.enum(['file', 'dir', 'symlink', 'other'])
export type EntryKind = z.infer<typeof entryKindSchema>

/**
 * Post-write check run — shared by create-file, update-file and
 * apply-patch responses. `error` set (e.g. a timeout) means the command
 * never produced a trustworthy exit code; treat it as its own state,
 * not as a failure exit.
 */
export const checkResultSchema = z.object({
  command: z.string(),
  /** Omitted when the check errored before exiting. */
  exit_code: z.number().nullish(),
  output: z.string(),
  truncated: z.boolean(),
  error: z.string().nullish(),
})
export type CheckResult = z.infer<typeof checkResultSchema>

/* ---------------- info ---------------- */

/** `info.rs::InfoInput` — pure discovery call, zero arguments. */
export const infoRequestSchema = z.object({})
export type InfoRequest = z.infer<typeof infoRequestSchema>

/** `info.rs::InfoOutput` — all 16 fields required, none nullable. */
export const infoResponseSchema = z.object({
  /** Canonical absolute allowed roots; index 0 is the primary root. */
  base_paths: z.array(z.string()),
  /** Convenience duplicate of `base_paths[0]`. */
  primary_root: z.string(),
  batch_read_budget_bytes: z.number(),
  max_output_bytes: z.number(),
  max_read_bytes: z.number(),
  max_write_bytes: z.number(),
  /** Noise filter only (bypassable) — distinct from non_accessible_globs. */
  default_exclude_globs: z.array(z.string()),
  /** Access-protected: listable but not readable/writable (C211). */
  non_accessible_globs: z.array(z.string()),
  list_default_page_size: z.number(),
  list_max_page_size: z.number(),
  search_default_max_line_bytes: z.number(),
  search_default_max_matches: z.number(),
  search_response_budget_bytes: z.number(),
  tree_default_depth: z.number(),
  tree_per_folder_limit: z.number(),
  version: z.string(),
})
export type InfoResponse = z.infer<typeof infoResponseSchema>

/* ---------------- create-file ---------------- */

export const createFileSpecSchema = z.object({
  path: z.string(),
  content: z.string(),
  /** Octal permission bits as a string, e.g. "0644". */
  mode: z.string().optional(),
  /** Create missing parent dirs (handler default true). */
  parents: z.boolean().optional(),
  /** When false (default), refuse existing paths with C217. */
  overwrite: z.boolean().optional(),
})
export type CreateFileSpec = z.infer<typeof createFileSpecSchema>

/**
 * No `.min(1)` on any request array (here or below): the goldens pin no
 * minItems. Empty batches (and `ops: []`) are runtime rejections —
 * top-level error or per-entry C210 — and those exchanges must still
 * render structurally instead of falling back to raw JSON.
 */
export const createFileRequestSchema = z.object({
  files: z.array(createFileSpecSchema),
})
export type CreateFileRequest = z.infer<typeof createFileRequestSchema>

export const createFileResultSchema = z.object({
  /** Canonical absolute; caller's input verbatim when resolution failed. */
  path: z.string(),
  success: z.boolean(),
  /** 0 on failure. */
  bytes_written: z.number(),
  /** Omitted on success. */
  error: wireErrorSchema.nullish(),
})
export type CreateFileResult = z.infer<typeof createFileResultSchema>

export const createFileResponseSchema = z.object({
  results: z.array(createFileResultSchema),
  /** Post-write check runs; absent on older wires. */
  checks: z.array(checkResultSchema).nullish(),
})
export type CreateFileResponse = z.infer<typeof createFileResponseSchema>

/* ---------------- update-file ---------------- */

export const updateOpInsertSchema = z.object({
  op: z.literal('insert'),
  /** Insert BEFORE this 1-based line; lines+1 appends to EOF. */
  at_line: z.number(),
  content: z.string(),
})

export const updateOpRemoveSchema = z.object({
  op: z.literal('remove'),
  from_line: z.number(),
  to_line: z.number(),
})

export const updateOpUpdateLinesSchema = z.object({
  op: z.literal('update_lines'),
  from_line: z.number(),
  to_line: z.number(),
  content: z.string(),
})

export const updateOpReplaceSchema = z.object({
  op: z.literal('replace'),
  pattern: z.string(),
  /** `$1`/`${name}` capture refs; literal `$` must be `$$`. */
  replacement: z.string(),
  ignore_case: z.boolean().optional(),
  /** When true `.` also matches \n (multi-line region replaces). */
  dot_matches_newline: z.boolean().optional(),
  /** Mismatch fails the file with C210; 0 asserts absence; null/omit = all. */
  expect_matches: z.number().nullish(),
})

export const updateOpSchema = z.discriminatedUnion('op', [
  updateOpInsertSchema,
  updateOpRemoveSchema,
  updateOpUpdateLinesSchema,
  updateOpReplaceSchema,
])
export type UpdateOp = z.infer<typeof updateOpSchema>

export const updateFileSpecSchema = z.object({
  path: z.string(),
  ops: z.array(updateOpSchema),
})
export type UpdateFileSpec = z.infer<typeof updateFileSpecSchema>

export const updateFileRequestSchema = z.object({
  files: z.array(updateFileSpecSchema),
})
export type UpdateFileRequest = z.infer<typeof updateFileRequestSchema>

/**
 * `update_file.rs::OpEcho` — bounded post-apply snapshot per op. Line ops
 * echo the region ±2 context lines; replace ops emit one echo per match
 * site (max 5, no context) sharing the same `op_index`.
 */
export const opEchoSchema = z.object({
  /** 0-based index into the request's ops array. */
  op_index: z.number(),
  /** 1-based first echoed line, AFTER all ops applied. */
  from_line: z.number(),
  lines: z.array(z.string()),
  /** Middle lines elided when the echoed region is large. */
  elided: z.number().nullish(),
  /** Replace-site echoes only: total replacements across the file. */
  total_replacements: z.number().nullish(),
})
export type OpEcho = z.infer<typeof opEchoSchema>

export const updateFileResultSchema = z.object({
  path: z.string(),
  /** Per-file atomic: all ops or none. */
  success: z.boolean(),
  /** Ops applied — only meaningful when success. */
  applied: z.number(),
  /** Final line count — only meaningful when success. */
  new_line_count: z.number(),
  /** Always present on the wire; empty array on failure. */
  echoes: z.array(opEchoSchema),
  /** True when the ~4 KiB echo budget cut echoes short — read-file to inspect. */
  echoes_truncated: z.boolean(),
  /** Omitted on success. */
  error: wireErrorSchema.nullish(),
})
export type UpdateFileResult = z.infer<typeof updateFileResultSchema>

export const updateFileResponseSchema = z.object({
  results: z.array(updateFileResultSchema),
  /** Post-write check runs; absent on older wires. */
  checks: z.array(checkResultSchema).nullish(),
})
export type UpdateFileResponse = z.infer<typeof updateFileResponseSchema>

/* ---------------- delete-file ---------------- */

export const deleteFileRequestSchema = z.object({
  paths: z.array(z.string()),
  /** Required for non-empty dirs; files and empty dirs ignore it. */
  recursive: z.boolean().optional(),
})
export type DeleteFileRequest = z.infer<typeof deleteFileRequestSchema>

export const deleteFileResultSchema = z.object({
  path: z.string(),
  /** Missing paths are idempotent SUCCESSES. */
  success: z.boolean(),
  /** False on idempotent missing-path success — "already absent". */
  removed: z.boolean(),
  /** Omitted on success. C210 = refusing to delete an allowed root. */
  error: wireErrorSchema.nullish(),
})
export type DeleteFileResult = z.infer<typeof deleteFileResultSchema>

export const deleteFileResponseSchema = z.object({
  results: z.array(deleteFileResultSchema),
})
export type DeleteFileResponse = z.infer<typeof deleteFileResponseSchema>

/* ---------------- move ---------------- */

export const moveFileSpecSchema = z.object({
  from: z.string(),
  to: z.string(),
  /** When false (default), refuse existing destinations with C217. */
  overwrite: z.boolean().optional(),
  /** Create missing parent dirs of the destination. */
  parents: z.boolean().optional(),
})
export type MoveFileSpec = z.infer<typeof moveFileSpecSchema>

export const moveFileRequestSchema = z.object({
  files: z.array(moveFileSpecSchema),
})
export type MoveFileRequest = z.infer<typeof moveFileRequestSchema>

export const moveFileResultSchema = z.object({
  from: z.string(),
  to: z.string(),
  success: z.boolean(),
  /** False for a no-op self-move — render as "unchanged", not moved. */
  moved: z.boolean(),
  /** Omitted on success. C210 messages may name a corrected target path. */
  error: wireErrorSchema.nullish(),
})
export type MoveFileResult = z.infer<typeof moveFileResultSchema>

export const moveFileResponseSchema = z.object({
  results: z.array(moveFileResultSchema),
})
export type MoveFileResponse = z.infer<typeof moveFileResponseSchema>

/* ---------------- read-file ---------------- */

/** Batch entry: bare path string = whole-file read, or per-entry options. */
export const readTargetSchema = z.union([
  z.string(),
  z.object({
    path: z.string(),
    line_from: z.number().nullish(),
    line_to: z.number().nullish(),
    /** Prefix lines with absolute `N→` numbers (feeds update-file ops). */
    numbered: z.boolean().optional(),
    /** Metadata-only probe; no batch budget consumed. */
    stat: z.boolean().optional(),
  }),
])
export type ReadTarget = z.infer<typeof readTargetSchema>

/**
 * `read_file.rs::ReadFileInput` — runtime enforces `path` XOR `paths`
 * (both or neither → C210); the schema itself has no required fields.
 */
export const readFileRequestSchema = z.object({
  path: z.string().nullish(),
  paths: z.array(readTargetSchema).nullish(),
  /** 1-based inclusive window start; switches to windowed streaming. */
  line_from: z.number().nullish(),
  /** 1-based inclusive window end; omit = read to EOF (byte-capped). */
  line_to: z.number().nullish(),
  numbered: z.boolean().optional(),
  /** Metadata probe — exclusive with window/numbered/max_output_bytes. */
  stat: z.boolean().optional(),
  /** Per-call override of the full-read context budget. */
  max_output_bytes: z.number().nullish(),
})
export type ReadFileRequest = z.infer<typeof readFileRequestSchema>

/** `read_file.rs::ReadEntryResult` — only `path` + `success` required. */
export const readEntryResultSchema = z.object({
  path: z.string(),
  success: z.boolean(),
  content: z.string().nullish(),
  is_utf8: z.boolean().nullish(),
  lines_returned: z.number().nullish(),
  /** Present only when the stream reached EOF — absent ≠ 0. */
  total_lines: z.number().nullish(),
  more_lines: z.boolean().nullish(),
  /** FILE size from metadata, not content size. */
  size: z.number().nullish(),
  /** Unix permission bits, lower 9 bits of st_mode. */
  mode: z.number().nullish(),
  mtime: z.number().nullish(),
  /** Only when success:false. C213 budget errors name the batch budget. */
  error: wireErrorSchema.nullish(),
})
export type ReadEntryResult = z.infer<typeof readEntryResultSchema>

/**
 * `read_file.rs::ReadFileOutput` — every field optional/omitted-when-unset.
 * Single-path mode populates the scalars and omits `results`; batch mode
 * populates only `results`.
 */
export const readFileResponseSchema = z.object({
  path: z.string().nullish(),
  content: z.string().nullish(),
  is_utf8: z.boolean().nullish(),
  lines_returned: z.number().nullish(),
  total_lines: z.number().nullish(),
  more_lines: z.boolean().nullish(),
  size: z.number().nullish(),
  mode: z.number().nullish(),
  mtime: z.number().nullish(),
  results: z.array(readEntryResultSchema).nullish(),
})
export type ReadFileResponse = z.infer<typeof readFileResponseSchema>

/* ---------------- search ---------------- */

export const searchRequestSchema = z.object({
  /** Regex when `regex: true`, else literal substring. */
  query: z.string(),
  regex: z.boolean().optional(),
  ignore_case: z.boolean().optional(),
  /** Folder scoping the walk; default "." = primary root. */
  path: z.string().optional(),
  include_globs: z.array(z.string()).optional(),
  exclude_globs: z.array(z.string()).optional(),
  use_default_excludes: z.boolean().optional(),
  search_content: z.boolean().optional(),
  search_paths: z.boolean().optional(),
  /** Max 10 — larger → C210. */
  context_lines_before: z.number().nullish(),
  context_lines_after: z.number().nullish(),
  max_matches: z.number().nullish(),
  max_line_bytes: z.number().nullish(),
})
export type SearchRequest = z.infer<typeof searchRequestSchema>

/** `search.rs::ContentMatch` — one per matching LINE (first match only). */
export const contentMatchSchema = z.object({
  path: z.string(),
  line: z.number(),
  column: z.number(),
  /** Truncated to max_line_bytes; never spans newlines. */
  text: z.string(),
  /** Omitted when empty — absence means zero context. */
  before: z.array(z.string()).nullish(),
  after: z.array(z.string()).nullish(),
})
export type ContentMatch = z.infer<typeof contentMatchSchema>

export const pathMatchSchema = z.object({
  path: z.string(),
})
export type PathMatch = z.infer<typeof pathMatchSchema>

export const searchResponseSchema = z.object({
  content_matches: z.array(contentMatchSchema),
  path_matches: z.array(pathMatchSchema),
  /** Hit max_matches or the response budget — refine, don't paginate. */
  truncated: z.boolean(),
})
export type SearchResponse = z.infer<typeof searchResponseSchema>

/* ---------------- tree ---------------- */

export const treeRequestSchema = z.object({
  path: z.string().optional(),
  /** Root node is depth 0; falls back to config tree_default_depth. */
  max_depth: z.number().nullish(),
  per_folder_limit: z.number().nullish(),
  /** False lists everything (excluded dirs otherwise become stubs). */
  use_default_excludes: z.boolean().optional(),
})
export type TreeRequest = z.infer<typeof treeRequestSchema>

/** `tree.rs::TruncationInfo` — why a folder's children were cut off. */
export const truncationInfoSchema = z.object({
  /** "per_folder_limit" | "max_depth" | "default_exclude". */
  reason: z.string(),
  /** Children actually returned. */
  shown: z.number(),
  /** Populated ONLY when reason == "per_folder_limit". */
  total: z.number().nullish(),
  /** Pre-written next-step guidance — render it. */
  hint: z.string(),
})
export type TruncationInfo = z.infer<typeof truncationInfoSchema>

/**
 * `tree.rs::TreeNode` (recursive). Wire caveats: `non_accessible` is
 * skipped when false (defaulted here); `children`/`truncated` omitted
 * when absent. The ROOT node's path is the top-level `path` — never
 * join root.name onto it; child path = parent path + "/" + name.
 */
export interface TreeNode {
  name: string
  kind: EntryKind
  size: number
  mtime: number
  non_accessible: boolean
  children?: TreeNode[] | null
  truncated?: TruncationInfo | null
}

export const treeNodeSchema: z.ZodType<TreeNode> = z.lazy(() =>
  z.object({
    name: z.string(),
    kind: entryKindSchema,
    size: z.number(),
    mtime: z.number(),
    non_accessible: z.boolean().default(false),
    children: z.array(treeNodeSchema).nullish(),
    truncated: truncationInfoSchema.nullish(),
  }),
)

export const treeResponseSchema = z.object({
  /** Canonical absolute path of the requested folder (= root node's path). */
  path: z.string(),
  root: treeNodeSchema,
})
export type TreeResponse = z.infer<typeof treeResponseSchema>

/* ---------------- list-folder ---------------- */

/** `list_folder.rs::ListFolderInput` — empty request lists primary root. */
export const listFolderRequestSchema = z.object({
  path: z.string().optional(),
  /** 1-based. */
  page: z.number().optional(),
  /** Falls back to list_default_page_size; capped by list_max_page_size. */
  page_size: z.number().nullish(),
})
export type ListFolderRequest = z.infer<typeof listFolderRequestSchema>

/** `list_folder.rs::DirEntry` — all fields always present on the wire. */
export const dirEntrySchema = z.object({
  /** Basename only — join onto the response `path` for the full path. */
  name: z.string(),
  kind: entryKindSchema,
  size: z.number(),
  mtime: z.number(),
  /** Listed but locked: coder::* ops on it return C211. */
  non_accessible: z.boolean(),
})
export type DirEntry = z.infer<typeof dirEntrySchema>

export const listFolderResponseSchema = z.object({
  path: z.string(),
  /** Sorted by name, this page only. */
  entries: z.array(dirEntrySchema),
  page: z.number(),
  /** Effective size used (after default fill / cap clamp). */
  page_size: z.number(),
  total: z.number(),
  has_more: z.boolean(),
})
export type ListFolderResponse = z.infer<typeof listFolderResponseSchema>

/* ---------------- apply-patch ---------------- */

/** V4A patch text: `*** Begin Patch` … `*** End Patch` with per-file
    Add/Delete/Update hunks. Parsed for rendering in ApplyPatchView. */
export const applyPatchRequestSchema = z.object({
  patch: z.string(),
})
export type ApplyPatchRequest = z.infer<typeof applyPatchRequestSchema>

export const patchResultKindSchema = z.enum([
  'added',
  'modified',
  'deleted',
  'moved',
])
export type PatchResultKind = z.infer<typeof patchResultKindSchema>

/** Bounded post-apply snapshot, like update-file's OpEcho (no op_index —
    apply-patch is one hunk per file). */
export const patchEchoSchema = z.object({
  /** 1-based first echoed line, AFTER the patch applied. */
  from_line: z.number(),
  lines: z.array(z.string()),
})
export type PatchEcho = z.infer<typeof patchEchoSchema>

export const applyPatchResultSchema = z.object({
  /** Canonical absolute; the destination path for moved files. */
  path: z.string(),
  kind: patchResultKindSchema,
  /** Absent for deleted files. */
  new_line_count: z.number().nullish(),
  echo: patchEchoSchema.nullish(),
})
export type ApplyPatchResult = z.infer<typeof applyPatchResultSchema>

export const applyPatchResponseSchema = z.object({
  /** One entry per file hunk, in patch order. */
  results: z.array(applyPatchResultSchema),
  /** Post-write check runs; absent on older wires. */
  checks: z.array(checkResultSchema).nullish(),
})
export type ApplyPatchResponse = z.infer<typeof applyPatchResponseSchema>

/* ---------------- context ---------------- */

/** `context.rs::ContextInput` — pure discovery call, zero arguments. */
export const contextRequestSchema = z.object({})
export type ContextRequest = z.infer<typeof contextRequestSchema>

export const contextGitSchema = z.object({
  branch: z.string(),
  /** Porcelain status lines — length is the dirty-entry count. */
  status: z.array(z.string()),
  status_truncated: z.boolean(),
  recent_commits: z.array(z.string()),
})
export type ContextGit = z.infer<typeof contextGitSchema>

export const instructionFileSchema = z.object({
  path: z.string(),
  content: z.string(),
  truncated: z.boolean(),
})
export type InstructionFile = z.infer<typeof instructionFileSchema>

export const contextResponseSchema = z.object({
  primary_root: z.string(),
  base_paths: z.array(z.string()),
  platform: z.object({
    os: z.string(),
    arch: z.string(),
  }),
  /** Absent when the primary root is not a git repository. */
  git: contextGitSchema.nullish(),
  instruction_files: z.array(instructionFileSchema),
})
export type ContextResponse = z.infer<typeof contextResponseSchema>

/* ---------------- worktree-add / worktree-remove ---------------- */

export const worktreeAddRequestSchema = z.object({
  name: z.string(),
})
export type WorktreeAddRequest = z.infer<typeof worktreeAddRequestSchema>

export const worktreeAddResponseSchema = z.object({
  path: z.string(),
  branch: z.string(),
})
export type WorktreeAddResponse = z.infer<typeof worktreeAddResponseSchema>

export const worktreeRemoveRequestSchema = z.object({
  name: z.string(),
})
export type WorktreeRemoveRequest = z.infer<typeof worktreeRemoveRequestSchema>

export const worktreeRemoveResponseSchema = z.object({
  /** False + dirty = refused: uncommitted work kept in place. */
  removed: z.boolean(),
  dirty: z.boolean(),
  path: z.string(),
  branch: z.string(),
  branch_deleted: z.boolean(),
})
export type WorktreeRemoveResponse = z.infer<
  typeof worktreeRemoveResponseSchema
>

/* ---------------- formatting helpers ---------------- */

/** Human-readable one-liner for a single update op (approval + done views). */
export function formatUpdateOp(op: UpdateOp): string {
  switch (op.op) {
    case 'insert':
      return `insert @ L${op.at_line}`
    case 'remove':
      return `remove L${op.from_line}–${op.to_line}`
    case 'update_lines':
      return `update_lines L${op.from_line}–${op.to_line}`
    case 'replace': {
      // Regex-style flags: i = ignore_case, s = dot_matches_newline.
      const flags = `${op.ignore_case ? 'i' : ''}${op.dot_matches_newline ? 's' : ''}`
      const base = `replace /${op.pattern}/${flags} → ${op.replacement || "''"}`
      // expect 0 asserts absence — still worth surfacing.
      if (op.expect_matches === null || op.expect_matches === undefined) {
        return base
      }
      return `${base} (expect ${op.expect_matches})`
    }
    default:
      return 'unknown op'
  }
}

export function truncateInline(text: string, max = 48): string {
  const oneLine = text.replace(/\s+/g, ' ').trim()
  if (oneLine.length <= max) return oneLine
  return `${oneLine.slice(0, max - 1)}…`
}

/**
 * Join a list-folder/tree entry `name` onto its parent's canonical path.
 * Entries carry basenames only; the wire contract says derive
 * `parent + "/" + name` (tolerating a trailing slash on the parent).
 */
export function joinEntryPath(parentPath: string, name: string): string {
  if (parentPath === '') return name
  if (parentPath.endsWith('/')) return `${parentPath}${name}`
  return `${parentPath}/${name}`
}
