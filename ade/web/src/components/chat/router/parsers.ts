/**
 * Zod schemas for the `router::*` namespace (the llm-router model catalog).
 *
 * Wire source: `iii/workers/llm-router/src/types/router.rs` + `types/model.rs`
 *   - ModelsListRequest { provider?, capability? }   (router.rs)
 *   - ModelsListResponse { models: Model[] }          (router.rs)
 *   - Model / Pricing capability record               (model.rs)
 *
 * Lenient (passthrough, optional) so a payload with extra/new capability flags
 * still parses and renders rather than falling back to raw JSON. Mirrors the
 * workflow:: / engine:: parser modules.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const ROUTER_FUNCTION_IDS = ['router::models::list'] as const
export type RouterFunctionId = (typeof ROUTER_FUNCTION_IDS)[number]

const ROUTER_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  ROUTER_FUNCTION_IDS,
)

export function isRouterFunction(id: string): id is RouterFunctionId {
  return ROUTER_FUNCTION_ID_SET.has(id)
}

/* ---------------- model descriptor ---------------- */

export const pricingSchema = z
  .object({
    input: z.number().nullable().optional(),
    output: z.number().nullable().optional(),
    cache_read: z.number().nullable().optional(),
    cache_write: z.number().nullable().optional(),
  })
  .passthrough()
export type Pricing = z.infer<typeof pricingSchema>

export const modelSchema = z
  .object({
    id: z.string(),
    provider: z.string(),
    display_name: z.string().nullable().optional(),
    context_window: z.number().optional(),
    max_output_tokens: z.number().optional(),
    input_limit: z.number().nullable().optional(),
    supports_thinking: z.boolean().nullable().optional(),
    supports_xhigh: z.boolean().nullable().optional(),
    supports_tools: z.boolean().nullable().optional(),
    supports_vision: z.boolean().nullable().optional(),
    supports_cache: z.boolean().nullable().optional(),
    supports_structured_output: z.boolean().nullable().optional(),
    thinking_budgets: z.record(z.string(), z.number()).nullable().optional(),
    pricing: pricingSchema.nullable().optional(),
  })
  .passthrough()
export type RouterModel = z.infer<typeof modelSchema>

/* ---------------- router::models::list ---------------- */

export const modelsListRequestSchema = z
  .object({
    provider: z.string().nullable().optional(),
    capability: z.string().nullable().optional(),
  })
  .passthrough()
export type ModelsListRequest = z.infer<typeof modelsListRequestSchema>

export const modelsListResponseSchema = z
  .object({
    models: z.array(modelSchema).optional().default([]),
  })
  .passthrough()
export type ModelsListResponse = z.infer<typeof modelsListResponseSchema>

/* ---------------- display helpers (pure) ---------------- */

/** Compact a token count: 1_000_000 → "1M", 200_000 → "200k", 4096 → "4096". */
export function formatTokens(n: number | undefined): string | null {
  if (typeof n !== 'number' || n <= 0) return null
  if (n >= 1_000_000) {
    const m = n / 1_000_000
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`
  }
  if (n >= 1_000) {
    const k = n / 1_000
    return `${Number.isInteger(k) ? k : k.toFixed(1)}k`
  }
  return String(n)
}

/* ---------------- generic parse helpers (mirror workflow/parsers) ---------------- */

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
