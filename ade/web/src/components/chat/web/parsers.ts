/**
 * Zod schemas + envelope helpers for `web::fetch`.
 *
 * Wire source:
 *   workers/harness/src/web/schemas.ts  -> FetchPayload (request)
 *                                       -> FetchResult  (response union)
 *   workers/harness/src/web/fetch.ts    -> handler returns FetchResult
 *
 * The response is a discriminated union on `ok`:
 *   { ok: true,  status, status_text, headers, body, response_format,
 *                bytes_truncated, json?, parse_error?, redirect_chain? }
 *   { ok: false, error: <variant>, message, status? }
 *
 * Schemas are non-strict so additive wire fields don't break the UI.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const WEB_FUNCTION_IDS = ['web::fetch'] as const
export type WebFunctionId = (typeof WEB_FUNCTION_IDS)[number]

const WEB_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  WEB_FUNCTION_IDS,
)

export function isWebFunction(id: string): id is WebFunctionId {
  return WEB_FUNCTION_ID_SET.has(id)
}

/* ---------------- request ---------------- */

export const fetchMethodSchema = z.enum([
  'GET',
  'HEAD',
  'POST',
  'PUT',
  'PATCH',
  'DELETE',
  'OPTIONS',
])
export type FetchMethod = z.infer<typeof fetchMethodSchema>

export const fetchResponseFormatSchema = z.enum(['text', 'base64', 'json'])
export type FetchResponseFormat = z.infer<typeof fetchResponseFormatSchema>

export const fetchRequestSchema = z.object({
  url: z.string(),
  method: fetchMethodSchema.optional(),
  headers: z.record(z.string(), z.string()).optional(),
  body: z.string().optional(),
  json: z.unknown().optional(),
  timeout_ms: z.number().optional(),
  max_bytes: z.number().optional(),
  follow_redirects: z.boolean().optional(),
  response_format: fetchResponseFormatSchema.optional(),
})
export type FetchRequest = z.infer<typeof fetchRequestSchema>

/* ---------------- response ---------------- */

export const fetchSuccessSchema = z.object({
  ok: z.literal(true),
  status: z.number(),
  status_text: z.string(),
  headers: z.record(z.string(), z.string()),
  body: z.string(),
  response_format: fetchResponseFormatSchema,
  bytes_truncated: z.boolean(),
  json: z.unknown().optional(),
  parse_error: z.string().optional(),
  redirect_chain: z.array(z.string()).optional(),
})
export type FetchSuccess = z.infer<typeof fetchSuccessSchema>

export const fetchErrorVariantSchema = z.enum([
  'invalid_payload',
  'invalid_url',
  'blocked_host',
  'timeout',
  'too_large',
  'too_many_redirects',
  'transport_error',
])
export type FetchErrorVariant = z.infer<typeof fetchErrorVariantSchema>

export const fetchErrorSchema = z.object({
  ok: z.literal(false),
  error: fetchErrorVariantSchema,
  message: z.string(),
  status: z.number().nullable().optional(),
})
export type FetchError = z.infer<typeof fetchErrorSchema>

export const fetchResultSchema = z.union([fetchSuccessSchema, fetchErrorSchema])
export type FetchResult = z.infer<typeof fetchResultSchema>

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

const ERROR_VARIANT_LABEL: Record<FetchErrorVariant, string> = {
  invalid_payload: 'Invalid payload',
  invalid_url: 'Invalid URL',
  blocked_host: 'Blocked host',
  timeout: 'Request timed out',
  too_large: 'Response too large',
  too_many_redirects: 'Too many redirects',
  transport_error: 'Transport error',
}

export function fetchErrorVariantLabel(variant: FetchErrorVariant): string {
  return ERROR_VARIANT_LABEL[variant]
}
