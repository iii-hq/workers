/**
 * Zod schemas + envelope helpers for batched `coder::*` mutators.
 *
 * Wire source:
 *   workers/coder/src/functions/create_file.rs  -> CreateFileInput/Output
 *   workers/coder/src/functions/update_file.rs  -> UpdateFileInput/Output
 *   workers/coder/src/functions/delete_file.rs  -> DeleteFileInput/Output
 *
 * Schemas are non-strict so additive wire fields don't break the UI.
 */
import { z } from 'zod'
import {
  safeParseRequest,
  safeParseResponse,
  unwrapEnvelope,
} from '@/components/chat/sandbox/parsers'

export { safeParseRequest, safeParseResponse, unwrapEnvelope }

export const CODER_MUTATE_FUNCTION_IDS = [
  'coder::create-file',
  'coder::update-file',
  'coder::delete-file',
] as const
export type CoderMutateFunctionId = (typeof CODER_MUTATE_FUNCTION_IDS)[number]

const CODER_MUTATE_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  CODER_MUTATE_FUNCTION_IDS,
)

export function isCoderMutateFunction(id: string): id is CoderMutateFunctionId {
  return CODER_MUTATE_FUNCTION_ID_SET.has(id)
}

/* ---------------- create-file ---------------- */

export const createFileSpecSchema = z.object({
  path: z.string(),
  content: z.string(),
  mode: z.string().optional(),
  parents: z.boolean().optional(),
  overwrite: z.boolean().optional(),
})
export type CreateFileSpec = z.infer<typeof createFileSpecSchema>

export const createFileRequestSchema = z.object({
  files: z.array(createFileSpecSchema).min(1),
})
export type CreateFileRequest = z.infer<typeof createFileRequestSchema>

export const createFileResultSchema = z.object({
  path: z.string(),
  success: z.boolean(),
  bytes_written: z.number(),
  error: z.string().optional(),
})
export type CreateFileResult = z.infer<typeof createFileResultSchema>

export const createFileResponseSchema = z.object({
  results: z.array(createFileResultSchema),
})
export type CreateFileResponse = z.infer<typeof createFileResponseSchema>

/* ---------------- update-file ---------------- */

export const updateOpInsertSchema = z.object({
  op: z.literal('insert'),
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
  replacement: z.string(),
  ignore_case: z.boolean().optional(),
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
  ops: z.array(updateOpSchema).min(1),
})
export type UpdateFileSpec = z.infer<typeof updateFileSpecSchema>

export const updateFileRequestSchema = z.object({
  files: z.array(updateFileSpecSchema).min(1),
})
export type UpdateFileRequest = z.infer<typeof updateFileRequestSchema>

export const updateFileResultSchema = z.object({
  path: z.string(),
  success: z.boolean(),
  applied: z.number(),
  new_line_count: z.number(),
  before: z.string().optional(),
  after: z.string().optional(),
  error: z.string().optional(),
})
export type UpdateFileResult = z.infer<typeof updateFileResultSchema>

export const updateFileResponseSchema = z.object({
  results: z.array(updateFileResultSchema),
})
export type UpdateFileResponse = z.infer<typeof updateFileResponseSchema>

/* ---------------- delete-file ---------------- */

export const deleteFileRequestSchema = z.object({
  paths: z.array(z.string()).min(1),
  recursive: z.boolean().optional(),
})
export type DeleteFileRequest = z.infer<typeof deleteFileRequestSchema>

export const deleteFileResultSchema = z.object({
  path: z.string(),
  success: z.boolean(),
  removed: z.boolean(),
  error: z.string().optional(),
})
export type DeleteFileResult = z.infer<typeof deleteFileResultSchema>

export const deleteFileResponseSchema = z.object({
  results: z.array(deleteFileResultSchema),
})
export type DeleteFileResponse = z.infer<typeof deleteFileResponseSchema>

/** Human-readable one-liner for a single update op (approval + done views). */
export function formatUpdateOp(op: UpdateOp): string {
  switch (op.op) {
    case 'insert':
      return `insert @ L${op.at_line}`
    case 'remove':
      return `remove L${op.from_line}–${op.to_line}`
    case 'update_lines':
      return `update_lines L${op.from_line}–${op.to_line}`
    case 'replace':
      return `replace /${op.pattern}/ → ${op.replacement || "''"}`
    default:
      return 'unknown op'
  }
}

export function truncateInline(text: string, max = 48): string {
  const oneLine = text.replace(/\s+/g, ' ').trim()
  if (oneLine.length <= max) return oneLine
  return `${oneLine.slice(0, max - 1)}…`
}
