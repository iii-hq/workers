/**
 * Zod schemas + helpers for the fp tool family.
 *
 * Wire sources:
 *   workers/fp/src/pipe.rs -> PipeRequest / PipeStep / PipeResponse
 *   workers/fp/src/util.rs -> per-op requests + UtilResponse { value }
 *
 * Schemas are non-strict so additive wire fields don't break the UI, and
 * optionals are `.nullish()` — serde skips `None` but model-emitted JSON may
 * carry explicit nulls.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const FP_PIPE_ID = 'fp::pipe'
export const FP_PREFIX = 'fp::'

/** The seventeen pure transforms — an EXPLICIT id set, not a `fp::` prefix
    match, so `fp::pipe` and the worker's internal hook function
    (`fp::inject-guidance`) never fall into the transform view. */
export const FP_TRANSFORM_OPS = [
  'get',
  'pick',
  'omit',
  'take',
  'drop',
  'map',
  'filter',
  'split',
  'join',
  'uniq',
  'size',
  'compact',
  'nth',
  'getOr',
  'flatten',
  'sortBy',
  'reverse',
] as const

const FP_TRANSFORM_ID_SET: ReadonlySet<string> = new Set<string>(
  FP_TRANSFORM_OPS.map((op) => `${FP_PREFIX}${op}`),
)

export function isFpTransformFunction(id: string): boolean {
  return FP_TRANSFORM_ID_SET.has(id)
}

export function isFpFunction(id: string): boolean {
  return id === FP_PIPE_ID || isFpTransformFunction(id)
}

/** `fp::take` → `take` (label text for the op pill). */
export function transformOp(id: string): string {
  return id.startsWith(FP_PREFIX) ? id.slice(FP_PREFIX.length) : id
}

/* ---------------- fp::pipe ---------------- */

/** `through` is optional here: a gated call's preview input can be a
    clipped `arguments_excerpt`. */
export const pipeStepSchema = z.object({
  function: z.string(),
  payload: z.unknown().nullish(),
  into: z.string().nullish(),
})
export type PipeStep = z.infer<typeof pipeStepSchema>

/** The harness-stamped trusted filesystem scope riding on a pipe call
    (harness/src/filesystem_scope.rs). Rendered so an approver reviewing
    relative shell/coder steps can see where they will run — the pipe call
    is the approval surface. */
export const pipeFsScopeSchema = z.object({
  root: z.string().nullish(),
  grants: z.array(z.string()).nullish(),
  boundary: z.string().nullish(),
})
export type PipeFsScope = z.infer<typeof pipeFsScopeSchema>

export const pipeRequestSchema = z.object({
  through: z.array(pipeStepSchema).nullish(),
  preview_chars: z.number().nullish(),
  fs_scope: pipeFsScopeSchema.nullish(),
})
export type PipeRequest = z.infer<typeof pipeRequestSchema>

/** Success details: per-step receipts + a preview, never the value. */
export const stepReceiptSchema = z.object({
  function: z.string(),
  chars: z.number(),
})
export const pipeResponseSchema = z.object({
  steps: z.array(stepReceiptSchema).nullish(),
  value_preview: z.string().nullish(),
})
export type PipeResponse = z.infer<typeof pipeResponseSchema>

/* ---------------- fp transforms ---------------- */

/** One loose schema covers all ten lodash-style ops — each op reads only its
    own params. */
export const utilRequestSchema = z.object({
  value: z.unknown().optional(),
  path: z.string().nullish(),
  paths: z.array(z.string()).nullish(),
  n: z.number().nullish(),
  separator: z.string().nullish(),
  matches: z.record(z.string(), z.unknown()).nullish(),
  default: z.unknown().optional(),
})
export type UtilRequest = z.infer<typeof utilRequestSchema>

/** Success details: the `UtilResponse { value }` wrapper. */
export const utilResponseSchema = z.object({ value: z.unknown() })

/* ---------------- helpers ---------------- */

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}
