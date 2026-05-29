/**
 * Zod schemas for the `worker::*` namespace (iii-worker manager daemon).
 *
 * Wire source: `motia/crates/iii-worker/src/core/types.rs`
 *   - AddOptions / AddOutcome
 *   - RemoveOptions / RemoveOutcome
 *   - UpdateOptions / UpdateOutcome
 *   - StartOptions / StartOutcome
 *   - StopOptions / StopOutcome
 *   - ListOptions / ListOutcome
 *   - ClearOptions / ClearOutcome
 *
 * Distinct from `engine::workers::*` (engine catalogue surface): this
 * namespace is the worker-manager CLI/daemon and uses different ids and
 * shapes — singular `worker::` prefix, focused on lifecycle ops.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const WORKER_FUNCTION_IDS = [
  'worker::add',
  'worker::remove',
  'worker::update',
  'worker::start',
  'worker::stop',
  'worker::list',
  'worker::clear',
  'worker::schema',
] as const

export type WorkerFunctionId = (typeof WORKER_FUNCTION_IDS)[number]

const WORKER_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  WORKER_FUNCTION_IDS,
)

export function isWorkerFunction(id: string): id is WorkerFunctionId {
  return WORKER_FUNCTION_ID_SET.has(id)
}

/* ---------------- shared building blocks ---------------- */

export const workerSourceSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('registry'),
    name: z.string(),
    version: z.string().optional(),
  }),
  z.object({
    kind: z.literal('oci'),
    reference: z.string(),
  }),
  z.object({
    kind: z.literal('local'),
    path: z.string(),
  }),
])
export type WorkerSource = z.infer<typeof workerSourceSchema>

/* ---------------- worker::list ---------------- */

export const workerListRequestSchema = z.object({
  running_only: z.boolean().optional(),
})
export type WorkerListRequest = z.infer<typeof workerListRequestSchema>

export const workerEntrySchema = z.object({
  name: z.string(),
  version: z.string().nullable().optional(),
  running: z.boolean(),
  pid: z.number().nullable().optional(),
})
export type WorkerEntry = z.infer<typeof workerEntrySchema>

export const workerListResponseSchema = z.object({
  workers: z.array(workerEntrySchema),
})
export type WorkerListResponse = z.infer<typeof workerListResponseSchema>

/* ---------------- worker::add ---------------- */

export const workerAddRequestSchema = z.object({
  source: workerSourceSchema,
  force: z.boolean().optional(),
  reset_config: z.boolean().optional(),
  wait: z.boolean().optional(),
})
export type WorkerAddRequest = z.infer<typeof workerAddRequestSchema>

export const workerAddStatusSchema = z.enum([
  'installed',
  'already_current',
  'repaired',
  'replaced',
])
export type WorkerAddStatus = z.infer<typeof workerAddStatusSchema>

export const workerAddResponseSchema = z.object({
  name: z.string(),
  version: z.string().nullable().optional(),
  status: workerAddStatusSchema,
  awaited_ready: z.boolean(),
  config_path: z.string(),
})
export type WorkerAddResponse = z.infer<typeof workerAddResponseSchema>

/* ---------------- worker::remove ---------------- */

export const workerRemoveRequestSchema = z.object({
  names: z.array(z.string()).optional(),
  all: z.boolean().optional(),
  yes: z.boolean().optional(),
})
export type WorkerRemoveRequest = z.infer<typeof workerRemoveRequestSchema>

export const workerRemoveResponseSchema = z.object({
  removed: z.array(z.string()),
})
export type WorkerRemoveResponse = z.infer<typeof workerRemoveResponseSchema>

/* ---------------- worker::update ---------------- */

export const workerUpdateRequestSchema = z.object({
  names: z.array(z.string()).optional(),
})
export type WorkerUpdateRequest = z.infer<typeof workerUpdateRequestSchema>

export const workerUpdateEntrySchema = z.object({
  name: z.string(),
  from_version: z.string(),
  to_version: z.string(),
})
export type WorkerUpdateEntry = z.infer<typeof workerUpdateEntrySchema>

export const workerUpdateResponseSchema = z.object({
  updated: z.array(workerUpdateEntrySchema),
})
export type WorkerUpdateResponse = z.infer<typeof workerUpdateResponseSchema>

/* ---------------- worker::start ---------------- */

export const workerStartRequestSchema = z.object({
  name: z.string(),
  port: z.number().optional(),
  config: z.string().optional(),
  wait: z.boolean().optional(),
})
export type WorkerStartRequest = z.infer<typeof workerStartRequestSchema>

export const workerStartResponseSchema = z.object({
  name: z.string(),
  pid: z.number().nullable().optional(),
  port: z.number().nullable().optional(),
})
export type WorkerStartResponse = z.infer<typeof workerStartResponseSchema>

/* ---------------- worker::stop ---------------- */

export const workerStopRequestSchema = z.object({
  name: z.string(),
  yes: z.boolean().optional(),
})
export type WorkerStopRequest = z.infer<typeof workerStopRequestSchema>

export const workerStopResponseSchema = z.object({
  name: z.string(),
  stopped: z.boolean(),
})
export type WorkerStopResponse = z.infer<typeof workerStopResponseSchema>

/* ---------------- worker::clear ---------------- */

export const workerClearRequestSchema = z.object({
  names: z.array(z.string()).optional(),
  all: z.boolean().optional(),
  yes: z.boolean().optional(),
})
export type WorkerClearRequest = z.infer<typeof workerClearRequestSchema>

export const workerClearResponseSchema = z.object({
  cleared: z.array(z.string()),
})
export type WorkerClearResponse = z.infer<typeof workerClearResponseSchema>

/* ---------------- generic helpers ---------------- */

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}

export function safeParseResponse<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(unwrapEnvelope(value))
  return parsed.success ? parsed.data : null
}
