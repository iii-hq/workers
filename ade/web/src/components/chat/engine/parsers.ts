/**
 * Zod schemas + envelope helpers for the engine catalogue tools and the
 * agent's observe surface. Mirrors the sandbox `parsers.ts` shape:
 * non-strict schemas (so unknown fields pass through), a re-exported
 * `unwrapEnvelope`, and `safeParseRequest` / `safeParseResponse` that unwrap
 * the harness `{ content, details, terminate }` envelope before parsing.
 *
 * Wire sources:
 *   `iii/engine/src/workers/engine_fn/mod.rs` — FunctionsListInput /
 *     FunctionSummary / FunctionDetail, WorkersListInput / WorkerSummary
 *   `iii/engine/src/observer/builtins.rs` — ListObserversInput /
 *     ObserverInfo / DisposeObserverResult
 *   `harness/src/functions/subscribe.rs` — `harness::observe` /
 *     `harness::unobserve`, the agent's observers
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const ENGINE_FUNCTION_IDS = [
  'engine::functions::list',
  'engine::functions::info',
  'engine::observers::list',
  'engine::observers::get',
  'engine::observers::dispose',
  'engine::workers::list',
  'engine::workers::info',
  'engine::workers::register',
  'harness::observe',
  'harness::unobserve',
] as const

export type EngineFunctionId = (typeof ENGINE_FUNCTION_IDS)[number]

const ENGINE_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  ENGINE_FUNCTION_IDS,
)

/** Predicate for every engine-rendered function in this module. */
export function isEngineListFunction(id: string): id is EngineFunctionId {
  return ENGINE_FUNCTION_ID_SET.has(id)
}

/** The agent's observe call, rendered as a registration receipt. */
export const OBSERVE_FUNCTION_ID = 'harness::observe'

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
  namespace: z.string().optional(),
  description: z.string().nullable().optional(),
  /** Observers of this function, active or pending. */
  observer_count: z.number().optional(),
  /** Present when observers of it pass args. */
  args_format: z.unknown().optional(),
  /** Only its own worker may call it. */
  private: z.boolean().optional(),
  /** `false` when nobody may observe it. */
  observable: z.boolean().optional(),
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

export const functionDetailSchema = z.object({
  function_id: z.string(),
  worker_name: z.string(),
  description: z.string().nullable().optional(),
  request_schema: z.unknown().optional(),
  response_schema: z.unknown().optional(),
  metadata: z.unknown().optional(),
  /** Observers of this function, active or pending. */
  observer_count: z.number().optional(),
  /** JSON Schema for observers' `args`; absent when it takes none. */
  args_format: z.unknown().optional(),
  /** Only its own worker may call it; others may observe it. */
  private: z.boolean().optional(),
  /** `false` when nobody may observe it. */
  observable: z.boolean().optional(),
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

/* ---------------- engine::observers::list / get ---------------- */

export const observersListRequestSchema = z.object({
  function_id: z.string().optional(),
  namespace: z.string().optional(),
  worker: z.string().optional(),
})
export type ObserversListRequest = z.infer<typeof observersListRequestSchema>

export const observerInfoSchema = z.object({
  id: z.string(),
  /** The observed function. */
  function_id: z.string(),
  namespace: z.string().optional(),
  /** Read by the observed function's worker (a route, a schedule, a topic). */
  args: z.unknown().optional(),
  /** Which calls reach the target, normalized. */
  filters: z.unknown().optional(),
  target: z.object({
    function_id: z.string(),
    namespace: z.string().optional(),
  }),
  capture: z.array(z.string()).optional(),
  on: z.string().optional(),
  deliver: z.string().optional(),
  /** The worker that created the observer. */
  worker: z
    .object({
      worker_id: z.string(),
      worker_name: z.string().nullable().optional(),
    })
    .optional(),
  state: z.string(),
  reason: z.object({ code: z.string(), message: z.string() }).optional(),
  fires: z.number().optional(),
  dropped: z.record(z.string(), z.number()).optional(),
  activation_failures: z.record(z.string(), z.number()).optional(),
  max_fires: z.number().optional(),
  expires_in_ms: z.number().optional(),
  max_in_flight: z.number().optional(),
  created_at_ms: z.number().optional(),
})
export type ObserverInfo = z.infer<typeof observerInfoSchema>

export const observersListResponseSchema = z.object({
  observers: z.array(observerInfoSchema),
})
export type ObserversListResponse = z.infer<typeof observersListResponseSchema>

export const observerIdRequestSchema = z.object({
  id: z.string(),
})
export type ObserverIdRequest = z.infer<typeof observerIdRequestSchema>

/**
 * `engine::observers::get` answers with the observer, bare or under
 * `observer`. `null` means neither shape parsed; the caller falls back to
 * the generic panes.
 */
export function parseObserverInfoResponse(
  output: unknown,
): ObserverInfo | null {
  const bare = safeParseResponse(observerInfoSchema, output)
  if (bare) return bare
  const wrapped = safeParseResponse(
    z.object({ observer: observerInfoSchema }),
    output,
  )
  return wrapped ? wrapped.observer : null
}

/* ---------------- engine::observers::dispose ---------------- */

export const disposeObserverResponseSchema = z.object({
  disposed: z.boolean(),
})
export type DisposeObserverResponse = z.infer<
  typeof disposeObserverResponseSchema
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
  namespace: z.string().optional(),
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
  /** Ids of the observers this worker created. */
  observers: z.array(z.string()).optional(),
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

/* ---------------- harness::observe ---------------- */

/**
 * The agent's observe request (`harness/src/functions/subscribe.rs`):
 * `{ function_id, namespace?, args?, filters?, target?, label?, once?,
 * conditions?, lifecycle? }` — `target` omitted means "wake this session".
 * Only `function_id` is guaranteed; everything else is optional so the view
 * always renders. `args` and `filters` are opaque JSON read per function.
 */
export const observeRequestSchema = z.object({
  function_id: z.string(),
  namespace: z.string().optional(),
  args: z.unknown().optional(),
  filters: z.unknown().optional(),
  label: z.string().optional(),
  /** What a fire means in user-facing prose. */
  action: z.string().optional(),
  once: z.boolean().optional(),
  /** Gating predicates: each fire runs these before delivery. */
  conditions: z
    .array(
      z.object({
        function_id: z.string().optional(),
        config: z.unknown().optional(),
      }),
    )
    .optional(),
  lifecycle: z
    .object({
      once: z.boolean().optional(),
      max_fires: z.number().optional(),
      expires_in_ms: z.number().optional(),
    })
    .optional(),
  target: z
    .object({
      function_id: z.string(),
      payload: z.unknown().optional(),
      event_into: z.string().optional(),
    })
    .optional(),
})
export type ObserveRequest = z.infer<typeof observeRequestSchema>

/** The harness answers `{ observer_id, once, note? }`. */
export const observeResponseSchema = z.object({
  observer_id: z.string(),
  once: z.boolean().optional(),
  note: z.string().optional(),
})
export type ObserveResponse = z.infer<typeof observeResponseSchema>

/* ---------------- harness::unobserve ---------------- */

export const unobserveRequestSchema = z.object({
  id: z.string(),
})
export type UnobserveRequest = z.infer<typeof unobserveRequestSchema>

export const unobserveResponseSchema = z.object({
  removed: z.boolean(),
})
export type UnobserveResponse = z.infer<typeof unobserveResponseSchema>

/* ---------------- generic helpers ---------------- */

/**
 * Some agents pass a function's whole payload as a JSON *string* (double
 * encoding), which arrives here as an escaped one-liner the rich views can't
 * parse. Recover the object/array so schema parsing and the structured views
 * work; anything that isn't a JSON-object/array string passes through
 * untouched (a genuine string request is left alone).
 */
export function coerceJsonObject(value: unknown): unknown {
  if (typeof value !== 'string') return value
  const t = value.trim()
  if (!t.startsWith('{') && !t.startsWith('[')) return value
  try {
    return JSON.parse(t)
  } catch {
    return value
  }
}

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
