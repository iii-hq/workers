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
  'engine::triggers::info',
  'engine::registered-triggers::list',
  'engine::workers::list',
  'engine::workers::info',
  'engine::workers::register',
  'engine::register_trigger',
] as const

export type EngineFunctionId = (typeof ENGINE_FUNCTION_IDS)[number]

const ENGINE_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  ENGINE_FUNCTION_IDS,
)

/** Predicate for every engine-rendered function in this module. */
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
  // Single lookup, or a batch (`function_ids`) — the engine accepts both.
  function_id: z.string().optional(),
  function_ids: z.array(z.string()).optional(),
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

export const functionInfoBatchResponseSchema = z.object({
  functions: z.array(functionDetailSchema),
})

/**
 * `engine::functions::info` answers a `function_id` lookup with a bare
 * detail and a `function_ids` batch with `{ functions: [...] }` — normalize
 * both to a list. `null` means neither shape parsed; the caller should fall
 * back to the generic panes rather than render a blank terminal tab.
 */
export function parseFunctionInfoResponse(
  output: unknown,
): FunctionDetail[] | null {
  const single = safeParseResponse(functionDetailSchema, output)
  if (single) return [single]
  const batch = safeParseResponse(functionInfoBatchResponseSchema, output)
  return batch ? batch.functions : null
}

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

/* ---------------- engine::triggers::info ---------------- */

export const triggerInfoRequestSchema = z.object({
  id: z.string(),
})
export type TriggerInfoRequest = z.infer<typeof triggerInfoRequestSchema>

export const triggerTypeDetailSchema = z.object({
  id: z.string(),
  worker_name: z.string(),
  description: z.string().nullable().optional(),
  /** Live registrations of this trigger type. */
  instance_count: z.number().optional(),
  /** Per-binding `config` shape accepted by `engine::register_trigger`. */
  configuration_schema: z.unknown().optional(),
  /** Payload shape delivered to the bound function when the trigger fires. */
  request_schema: z.unknown().optional(),
})
export type TriggerTypeDetail = z.infer<typeof triggerTypeDetailSchema>

/**
 * `null` means the output didn't parse; the caller should fall back to the
 * generic panes rather than render a blank terminal tab (same contract as
 * `parseFunctionInfoResponse`).
 */
export function parseTriggerInfoResponse(
  output: unknown,
): TriggerTypeDetail | null {
  return safeParseResponse(triggerTypeDetailSchema, output)
}

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

/* ---------------- engine::register_trigger ---------------- */

/**
 * Registration request. Covers both wire shapes seen under this id:
 *  - engine `RegisterTriggerInput` (iii-sdk `protocol.rs`): `{ trigger_type,
 *    function_id, config, metadata? }`.
 *  - harness `SubscribeArgs` (`harness/src/functions/subscribe.rs`):
 *    `{ trigger_type, config?, label?, once?, function_id?, metadata? }` —
 *    `function_id` omitted means "notify this session".
 * Only `trigger_type` is guaranteed; everything else is optional so the view
 * always renders. `config`/`metadata` are opaque JSON parsed per-provider.
 */
export const registerTriggerRequestSchema = z.object({
  trigger_type: z.string(),
  function_id: z.string().optional(),
  config: z.unknown().optional(),
  metadata: z.unknown().optional(),
  label: z.string().optional(),
  once: z.boolean().optional(),
})
export type RegisterTriggerRequest = z.infer<
  typeof registerTriggerRequestSchema
>

/** `config` shape for `trigger_type: "state"` (all fields optional filters). */
export const stateTriggerConfigSchema = z.object({
  scope: z.string().optional(),
  key: z.string().optional(),
  condition_function_id: z.string().optional(),
})
export type StateTriggerConfig = z.infer<typeof stateTriggerConfigSchema>

/**
 * `metadata` shape for `function_id: "harness::react"` — the reactive bridge.
 * Wire source: `harness/src/functions/react.rs` (`ReactSpec` / `JoinSpec`).
 * `options` is free-form (mirrors `harness::spawn` SpawnOptions); the common
 * `options.functions.allow: string[]` is surfaced by the view.
 */
export const joinSpecSchema = z.object({
  id: z.string(),
  expect: z.array(z.string()),
  key: z.string(),
  rearm: z.boolean().optional(),
})
export type JoinSpec = z.infer<typeof joinSpecSchema>

export const reactSpecSchema = z.object({
  model: z.string(),
  task: z.string(),
  session_id: z.string().optional(),
  provider: z.string().optional(),
  options: z.unknown().optional(),
  parent_session_id: z.string().optional(),
  join: joinSpecSchema.optional(),
})
export type ReactSpec = z.infer<typeof reactSpecSchema>

/** `options.functions.allow` — the only bit of the free-form `options` the
 * view reads. Non-strict so unknown option keys pass through. */
export const reactOptionsSchema = z.object({
  functions: z.object({ allow: z.array(z.string()).optional() }).optional(),
})
export type ReactOptions = z.infer<typeof reactOptionsSchema>

/** Engine returns `{ id }`; the harness-intercepted path returns
 * `{ subscription_id, once }`. Model both loosely. */
export const registerTriggerResponseSchema = z.object({
  id: z.string().optional(),
  subscription_id: z.string().optional(),
  once: z.boolean().optional(),
})
export type RegisterTriggerResponse = z.infer<
  typeof registerTriggerResponseSchema
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
