/**
 * Zod schemas + helpers for the `scrapling::*` worker family.
 *
 * Wire source:
 *   workers/scrapling/src/schemas.py -> FUNCTIONS (request/response schemas)
 *   workers/scrapling/src/core.py    -> serialize_page / op_* return shapes
 *
 * Fetch responses (fetch / stealthy-fetch / dynamic-fetch) are one of:
 *   single page  { status, url, headers, cookies, encoding, extracted?,
 *                  captured_xhr?, html? }
 *   bulk         { results: [ page | { url, error } ] }   (when called with `urls`)
 *
 * Schemas are non-strict so additive wire fields don't break the UI.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const SCRAPLING_FUNCTION_IDS = [
  'scrapling::fetch',
  'scrapling::stealthy-fetch',
  'scrapling::dynamic-fetch',
  'scrapling::screenshot',
  'scrapling::extract',
  'scrapling::css',
  'scrapling::xpath',
  'scrapling::regex',
  'scrapling::find-similar',
] as const
export type ScraplingFunctionId = (typeof SCRAPLING_FUNCTION_IDS)[number]

const SCRAPLING_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  SCRAPLING_FUNCTION_IDS,
)

export function isScraplingFunction(id: string): id is ScraplingFunctionId {
  return SCRAPLING_FUNCTION_ID_SET.has(id)
}

export const FETCH_FUNCTION_IDS: ReadonlySet<string> = new Set([
  'scrapling::fetch',
  'scrapling::stealthy-fetch',
  'scrapling::dynamic-fetch',
])

/** Which fetch tier a function runs on — drives the engine chip. */
export function fetchEngineLabel(functionId: string): string {
  switch (functionId) {
    case 'scrapling::stealthy-fetch':
      return 'camoufox'
    case 'scrapling::dynamic-fetch':
      return 'chromium'
    default:
      return 'http'
  }
}

/* ---------------- selectors (shared by fetch + parse ops) ---------------- */

export const selectorSpecSchema = z.object({
  name: z.string(),
  css: z.string().optional(),
  xpath: z.string().optional(),
  regex: z.string().optional(),
  attr: z.string().optional(),
  html: z.boolean().optional(),
  all: z.boolean().optional(),
})
export type SelectorSpec = z.infer<typeof selectorSpecSchema>

/* ---------------- fetch / stealthy-fetch / dynamic-fetch ---------------- */

export const fetchRequestSchema = z.object({
  url: z.string().optional(),
  urls: z.array(z.string()).optional(),
  method: z.string().optional(),
  impersonate: z.string().optional(),
  proxy: z.string().optional(),
  headless: z.boolean().optional(),
  network_idle: z.boolean().optional(),
  solve_cloudflare: z.boolean().optional(),
  real_chrome: z.boolean().optional(),
  cdp_url: z.string().optional(),
  wait_selector: z.string().optional(),
  timeout: z.number().optional(),
  selectors: z.array(selectorSpecSchema).optional(),
  include_html: z.boolean().optional(),
})
export type FetchRequest = z.infer<typeof fetchRequestSchema>

export const pageResultSchema = z.object({
  status: z.number().nullable().optional(),
  url: z.string().optional(),
  headers: z.record(z.string(), z.unknown()).optional(),
  cookies: z.record(z.string(), z.unknown()).optional(),
  encoding: z.string().nullable().optional(),
  extracted: z.record(z.string(), z.unknown()).optional(),
  captured_xhr: z.array(z.unknown()).optional(),
  html: z.string().optional(),
  /** bulk per-url failure rows are `{ url, error }` */
  error: z.string().optional(),
})
export type PageResult = z.infer<typeof pageResultSchema>

export const bulkResponseSchema = z.object({
  results: z.array(pageResultSchema),
})
export type BulkResponse = z.infer<typeof bulkResponseSchema>

/** Bulk first: the loose page schema would match (and strip) a bulk payload. */
export const fetchResponseSchema = z.union([
  bulkResponseSchema,
  pageResultSchema,
])
export type FetchResponse = z.infer<typeof fetchResponseSchema>

/* ---------------- screenshot ---------------- */

export const screenshotRequestSchema = z.object({
  url: z.string(),
  fetcher: z.string().optional(),
  full_page: z.boolean().optional(),
  format: z.string().optional(),
})
export type ScreenshotRequest = z.infer<typeof screenshotRequestSchema>

export const screenshotResponseSchema = z.object({
  image_base64: z.string(),
  mime: z.string().optional(),
  url: z.string().optional(),
})
export type ScreenshotResponse = z.infer<typeof screenshotResponseSchema>

/* ---------------- parse-only ops ---------------- */

export const extractRequestSchema = z.object({
  html: z.string().optional(),
  selectors: z.array(selectorSpecSchema).optional(),
})
export type ExtractRequest = z.infer<typeof extractRequestSchema>

export const extractResponseSchema = z.object({
  extracted: z.record(z.string(), z.unknown()),
})
export type ExtractResponse = z.infer<typeof extractResponseSchema>

export const queryRequestSchema = z.object({
  html: z.string().optional(),
  query: z.string().optional(),
  pattern: z.string().optional(),
  first: z.boolean().optional(),
  attr: z.string().optional(),
})
export type QueryRequest = z.infer<typeof queryRequestSchema>

/** `attr` misses inside an `all` list come back as null items. */
export const queryResponseSchema = z.object({
  result: z.union([z.array(z.string().nullable()), z.string(), z.null()]),
})
export type QueryResponse = z.infer<typeof queryResponseSchema>

export const findSimilarRequestSchema = z.object({
  html: z.string().optional(),
  anchor: z.string().optional(),
  similarity_threshold: z.number().optional(),
  match_text: z.boolean().optional(),
  selectors: z.array(selectorSpecSchema).optional(),
})
export type FindSimilarRequest = z.infer<typeof findSimilarRequestSchema>

export const findSimilarResponseSchema = z.object({
  count: z.number(),
  items: z.array(z.record(z.string(), z.unknown())),
})
export type FindSimilarResponse = z.infer<typeof findSimilarResponseSchema>

/* ---------------- helpers ---------------- */

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

export function formatChars(n: number): string {
  if (n < 1000) return `${n} chars`
  return `${(n / 1000).toFixed(1)}k chars`
}
