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
 *    `{ trigger_type, config?, label?, once?, function_id?, metadata?,
 *    target? }` — `function_id` omitted means "notify this session", while
 *    `target` is its explicit long form.
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
  target: z
    .object({
      function_id: z.string(),
      payload: z.unknown().optional(),
      event_into: z.string().optional(),
    })
    .optional(),
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
 * The known filter fields across trigger `config` shapes — state
 * (`scope`/`key`/`condition_function_id`) and turn events
 * (`session_id`/`parent_session_id`) — as labeled chips, in a stable order.
 * `null` when the config carries none of them (the caller shows raw JSON or
 * "no filter"); unknown fields stay visible in the RAW JSON tab.
 */
export function configFilters(
  config: unknown,
): { label: string; value: string }[] | null {
  if (!config || typeof config !== 'object' || Array.isArray(config)) {
    return null
  }
  const c = config as Record<string, unknown>
  const pick = (key: string, label: string) =>
    typeof c[key] === 'string' && c[key]
      ? { label, value: c[key] as string }
      : null
  const chips = [
    pick('scope', 'scope'),
    pick('key', 'key'),
    pick('session_id', 'session'),
    pick('parent_session_id', 'parent'),
    pick('condition_function_id', 'if'),
  ].filter((x): x is { label: string; value: string } => x !== null)
  return chips.length ? chips : null
}

const MONTHS = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
]
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

/**
 * Humanize the COMMON cron shapes — fixed time, minute steps, single
 * day-of-month/month, weekday lists — and return `null` for anything
 * fancier (ranges, multi-field lists, `L`/`#` extensions), where the raw
 * expression is less misleading than a wrong translation. Accepts 5-field
 * classic and the 6/7-field seconds-first form the Rust `cron` crate uses
 * (`"0 0 17 21 7 *"` → "at 17:00 on Jul 21").
 */
export function describeCron(expression: string): string | null {
  const f = expression.trim().split(/\s+/)
  if (f.length < 5 || f.length > 7) return null
  const withSeconds = f.length >= 6
  const [min, hour, dom, mon, dow] = withSeconds ? f.slice(1) : f
  const sec = withSeconds ? f[0] : '0'
  const year = f.length === 7 ? f[6] : '*'
  if (year !== '*') return null

  const num = (s: string, max: number): number | null =>
    /^\d+$/.test(s) && Number(s) <= max ? Number(s) : null
  const secondsWild = sec === '*'
  const secNum = num(sec, 59)
  if (!secondsWild && secNum == null) return null
  const step = (s: string): number | null => {
    const m = /^\*\/(\d+)$/.exec(s)
    return m ? Number(m[1]) : null
  }
  const pad = (n: number) => String(n).padStart(2, '0')

  const h = num(hour, 23)
  const m = num(min, 59)
  let time: string | null = null
  let daily = false
  if (h != null && m != null) {
    time = `at ${pad(h)}:${pad(m)}`
    daily = true
  } else if (hour === '*') {
    const minStep = step(min)
    if (minStep != null) time = `every ${minStep} min`
    else if (min === '*') time = sec === '*' ? 'every second' : 'every minute'
    else if (m != null) time = `at :${pad(m)} every hour`
    else return null
  } else {
    const hourStep = step(hour)
    if (hourStep != null && m != null) time = `every ${hourStep}h at :${pad(m)}`
    else return null
  }
  // Every description except "every second" speaks at minute granularity, so
  // it is only honest when the schedule fires once per matching minute
  // (seconds pinned to 0). `* 0 17 * * *` fires every second DURING 17:00 —
  // saying "at 17:00" would hide sixty firings; a nonzero pin drops detail.
  if (time !== 'every second' && (secondsWild || secNum !== 0)) return null

  const dn = num(dom, 31)
  const mn = num(mon, 12)
  if (dom !== '*' && (dn == null || dn < 1)) return null
  if (mon !== '*' && (mn == null || mn < 1)) return null
  let date: string | null = null
  if (dn != null && mn != null) date = `on ${MONTHS[mn - 1]} ${dn}`
  else if (dn != null) date = `on day ${dn} of every month`
  else if (mn != null) date = `in ${MONTHS[mn - 1]}`

  let week: string | null = null
  if (dow !== '*' && dow !== '?') {
    const names = dow.split(',').map((d) => {
      const n = num(d, 7)
      if (n == null) return null
      // Numeric weekday numbering differs by dialect: the seconds-first form
      // is the Rust `cron` crate's (Quartz-style, 1=Sun..7=Sat); classic
      // five-field cron is 0=Sun..6=Sat with 7 also Sunday.
      if (withSeconds) return n >= 1 ? WEEKDAYS[n - 1] : null
      return WEEKDAYS[n % 7]
    })
    if (names.some((n) => n == null)) return null
    week = `every ${names.join(', ')}`
  }

  // Both a day-of-month AND a weekday restriction is OR semantics in cron —
  // subtle enough that the raw expression is the honest rendering.
  if (date && week) return null
  if (week) return daily ? `${week} ${time}` : `${time} ${week}`
  if (date) return `${time} ${date}`
  return daily ? `every day ${time}` : (time as string)
}

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
