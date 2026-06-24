/**
 * Zod schemas + envelope helpers for the three `engine::*::list` catalogue
 * tools. Mirrors the sandbox `parsers.ts` shape: non-strict schemas (so
 * unknown fields pass through), a re-exported `unwrapEnvelope`, and
 * `safeParseRequest` / `safeParseResponse` that unwrap the harness
 * `{ content, details, terminate }` envelope before parsing.
 *
 * Wire source: `motia/engine/src/workers/engine_fn/mod.rs` —
 *   FunctionsListInput / FunctionSummary
 *   TriggersListInput / TriggerTypeSummary
 *   RegisteredTriggersListInput / RegisteredTriggerSummary
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const ENGINE_FUNCTION_IDS = [
  'engine::functions::list',
  'engine::functions::info',
  'engine::triggers::list',
  'engine::registered-triggers::list',
  'engine::workers::list',
  'engine::workers::info',
  'engine::workers::register',
] as const

export type EngineFunctionId = (typeof ENGINE_FUNCTION_IDS)[number]

const ENGINE_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  ENGINE_FUNCTION_IDS,
)

/**
 * Predicate for both the list catalogues and the singular `info` lookup.
 * Name kept for backwards compatibility with the FCM wiring; the family
 * now covers every renderer in this module.
 */
export function isEngineListFunction(id: string): id is EngineFunctionId {
  return ENGINE_FUNCTION_ID_SET.has(id)
}

/* ---------------- engine::functions::list ---------------- */

export const functionsListRequestSchema = z.object({
  search: z.string().optional(),
  prefix: z.string().optional(),
  worker: z.string().optional(),
  include_internal: z.boolean().optional(),
})
export type FunctionsListRequest = z.infer<typeof functionsListRequestSchema>

export const functionSummarySchema = z.object({
  function_id: z.string(),
  worker_name: z.string(),
  description: z.string().nullable().optional(),
})
export type FunctionSummary = z.infer<typeof functionSummarySchema>

export const functionsListResponseSchema = z.object({
  functions: z.array(functionSummarySchema),
})
export type FunctionsListResponse = z.infer<typeof functionsListResponseSchema>

/* ---------------- engine::functions::info ---------------- */

export const functionInfoRequestSchema = z.object({
  function_id: z.string(),
})
export type FunctionInfoRequest = z.infer<typeof functionInfoRequestSchema>

/** Inline registered trigger payload from `FunctionDetail`. Different shape
 * than `RegisteredTriggerSummary` (used by `engine::registered-triggers::list`):
 * `config` is the raw JSON object, not a stringified `config_summary`. */
export const registeredTriggerRefSchema = z.object({
  id: z.string(),
  trigger_type: z.string(),
  config: z.unknown(),
})
export type RegisteredTriggerRef = z.infer<typeof registeredTriggerRefSchema>

export const functionDetailSchema = z.object({
  function_id: z.string(),
  worker_name: z.string(),
  description: z.string().nullable().optional(),
  request_schema: z.unknown().optional(),
  response_schema: z.unknown().optional(),
  metadata: z.unknown().optional(),
  registered_triggers: z.array(registeredTriggerRefSchema),
})
export type FunctionDetail = z.infer<typeof functionDetailSchema>

/* ---------------- engine::triggers::list ---------------- */

export const triggersListRequestSchema = z.object({
  search: z.string().optional(),
  prefix: z.string().optional(),
  worker: z.string().optional(),
  include_internal: z.boolean().optional(),
})
export type TriggersListRequest = z.infer<typeof triggersListRequestSchema>

export const triggerTypeSummarySchema = z.object({
  id: z.string(),
  worker_name: z.string(),
  description: z.string(),
})
export type TriggerTypeSummary = z.infer<typeof triggerTypeSummarySchema>

export const triggersListResponseSchema = z.object({
  triggers: z.array(triggerTypeSummarySchema),
})
export type TriggersListResponse = z.infer<typeof triggersListResponseSchema>

/* ------------- engine::registered-triggers::list ------------- */

export const registeredTriggersListRequestSchema = z.object({
  search: z.string().optional(),
  trigger_type: z.string().optional(),
  function_id: z.string().optional(),
  worker: z.string().optional(),
  include_internal: z.boolean().optional(),
})
export type RegisteredTriggersListRequest = z.infer<
  typeof registeredTriggersListRequestSchema
>

export const registeredTriggerSummarySchema = z.object({
  id: z.string(),
  trigger_type: z.string(),
  function_id: z.string(),
  worker_name: z.string(),
  config_summary: z.string(),
})
export type RegisteredTriggerSummary = z.infer<
  typeof registeredTriggerSummarySchema
>

export const registeredTriggersListResponseSchema = z.object({
  registered_triggers: z.array(registeredTriggerSummarySchema),
})
export type RegisteredTriggersListResponse = z.infer<
  typeof registeredTriggersListResponseSchema
>

/* ---------------- engine::workers::list ---------------- */

export const workersListRequestSchema = z.object({
  search: z.string().optional(),
  runtime: z.string().optional(),
  status: z.string().optional(),
  tag: z.string().optional(),
})
export type WorkersListRequest = z.infer<typeof workersListRequestSchema>

export const workerSummarySchema = z.object({
  id: z.string(),
  name: z.string().nullable().optional(),
  description: z.string().nullable().optional(),
  version: z.string().nullable().optional(),
  runtime: z.string().nullable().optional(),
  os: z.string().nullable().optional(),
  status: z.string(),
  function_count: z.number(),
  connected_at_ms: z.number(),
  active_invocations: z.number(),
  isolation: z.string().nullable().optional(),
  ip_address: z.string().nullable().optional(),
  tag: z.string().nullable().optional(),
})
export type WorkerSummary = z.infer<typeof workerSummarySchema>

export const workersListResponseSchema = z.object({
  workers: z.array(workerSummarySchema),
})
export type WorkersListResponse = z.infer<typeof workersListResponseSchema>

/* ---------------- engine::workers::info ---------------- */

export const workerInfoRequestSchema = z.object({
  name: z.string(),
})
export type WorkerInfoRequest = z.infer<typeof workerInfoRequestSchema>

export const workerMetricsSchema = z.object({
  memory_heap_used: z.number().optional(),
  memory_heap_total: z.number().optional(),
  memory_rss: z.number().optional(),
  memory_external: z.number().optional(),
  cpu_user_micros: z.number().optional(),
  cpu_system_micros: z.number().optional(),
  cpu_percent: z.number().optional(),
  event_loop_lag_ms: z.number().optional(),
  uptime_seconds: z.number().optional(),
  timestamp_ms: z.number(),
  runtime: z.string(),
})
export type WorkerMetrics = z.infer<typeof workerMetricsSchema>

export const workerDetailEnvelopeSchema = workerSummarySchema.extend({
  pid: z.number().optional(),
  internal: z.boolean(),
  latest_metrics: workerMetricsSchema.nullable().optional(),
})
export type WorkerDetailEnvelope = z.infer<typeof workerDetailEnvelopeSchema>

/** `functions` inside `workers::info` only carries `function_id` +
 * `worker_name` (no description). The `functionSummarySchema` already
 * marks `description` optional/nullable, so we reuse it here. */
export const workerInfoResponseSchema = z.object({
  worker: workerDetailEnvelopeSchema,
  functions: z.array(functionSummarySchema),
  trigger_types: z.array(triggerTypeSummarySchema),
  registered_triggers: z.array(registeredTriggerSummarySchema),
})
export type WorkerInfoResponse = z.infer<typeof workerInfoResponseSchema>

/* ---------------- engine::workers::register ---------------- */

export const workerTelemetryMetaSchema = z.object({
  device_id: z.string().optional(),
  install_kind: z.string().optional(),
})

export const workersRegisterRequestSchema = z.object({
  _caller_worker_id: z.string(),
  runtime: z.string().nullable().optional(),
  version: z.string().nullable().optional(),
  name: z.string().nullable().optional(),
  os: z.string().nullable().optional(),
  telemetry: workerTelemetryMetaSchema.nullable().optional(),
})
export type WorkersRegisterRequest = z.infer<
  typeof workersRegisterRequestSchema
>

export const workersRegisterResponseSchema = z.object({
  success: z.boolean(),
})
export type WorkersRegisterResponse = z.infer<
  typeof workersRegisterResponseSchema
>

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
