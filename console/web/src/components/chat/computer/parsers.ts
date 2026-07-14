/**
 * Zod schemas + helpers for the `computer::*` worker family.
 *
 * Wire source: workers/computer/src/functions/*.rs (Serialize structs).
 *
 * `screenshot` / `observe` return a viewable envelope
 *   { content: [ {type:'image', mime, data}, {type:'text', text} ], details }
 * — the same content-block shape the browser worker uses. Through the harness
 * the image blocks stay at the top-level `content` while `details` becomes the
 * whole worker return (so the real metadata sits one level under, at
 * `details.details`). `parseScreenshotResponse` reads `content` off the top and
 * accepts either details nesting; `unwrapEnvelope` is deliberately NOT used for
 * it because it would return `details` and drop the image blocks.
 *
 * The other ops (`act`, `sessions::*`) return flat structured payloads, so
 * `safeParseResponse` unwraps the harness envelope exactly like scrapling.
 *
 * Schemas are non-strict so additive wire fields don't break the UI.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const COMPUTER_FUNCTION_IDS = [
  'computer::sessions::start',
  'computer::sessions::list',
  'computer::sessions::stop',
  'computer::screenshot',
  'computer::observe',
  'computer::act',
  'computer::screencast::start',
  'computer::screencast::stop',
  'computer::frame',
] as const
export type ComputerFunctionId = (typeof COMPUTER_FUNCTION_IDS)[number]

const COMPUTER_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  COMPUTER_FUNCTION_IDS,
)

export function isComputerFunction(id: string): id is ComputerFunctionId {
  return COMPUTER_FUNCTION_ID_SET.has(id)
}

/** Screenshot + observe render an inline image; the rest get compact lines. */
export const SCREENSHOT_FUNCTION_IDS: ReadonlySet<string> = new Set([
  'computer::screenshot',
  'computer::observe',
])

/** Console-only plumbing (screencast + frame) — no agent-facing view. */
export const INTERNAL_FUNCTION_IDS: ReadonlySet<string> = new Set([
  'computer::screencast::start',
  'computer::screencast::stop',
  'computer::frame',
])

/* ---------------- shared ---------------- */

export const screenSchema = z.object({
  width: z.number(),
  height: z.number(),
})
export type Screen = z.infer<typeof screenSchema>

/* ---------------- screenshot / observe ---------------- */

export const screenshotRequestSchema = z.object({
  session_id: z.string().optional(),
  include_a11y: z.boolean().nullable().optional(),
})
export type ScreenshotRequest = z.infer<typeof screenshotRequestSchema>

/** Harness content blocks: an image tile plus a trailing text caption. */
export const screenshotBlockSchema = z.object({
  type: z.string(),
  mime: z.string().optional(),
  data: z.string().optional(),
  text: z.string().optional(),
})
export type ScreenshotBlock = z.infer<typeof screenshotBlockSchema>

/** Flat metadata — `screenshot` reports width/height directly, `observe` nests
 *  them under `screen`. Both are accepted. */
export const screenshotDetailsSchema = z.object({
  session_id: z.string().optional(),
  mime: z.string().optional(),
  width: z.number().optional(),
  height: z.number().optional(),
  screen: screenSchema.optional(),
})
export type ScreenshotDetails = z.infer<typeof screenshotDetailsSchema>

export const screenshotResponseSchema = z.object({
  content: z.array(screenshotBlockSchema),
  // Nested first: through the harness `details` is the whole worker return, so
  // the real metadata sits at `details.details`; a direct bus call has it flat.
  details: z
    .union([
      z.object({
        details: screenshotDetailsSchema,
        accessibility: z.unknown().optional(),
      }),
      screenshotDetailsSchema,
    ])
    .optional(),
  accessibility: z.unknown().optional(),
})
export type ScreenshotResponse = z.infer<typeof screenshotResponseSchema>

export interface ParsedScreenshot {
  images: ScreenshotBlock[]
  caption?: string
  sessionId?: string
  width?: number
  height?: number
  mime: string
  hasAccessibility: boolean
}

/**
 * Parse a `computer::screenshot` / `computer::observe` result. The image blocks
 * live at the top-level `content` in both the direct bus result and the harness
 * transcript output, so `content` is read off the top (`unwrapEnvelope` would
 * drop it by returning `details`). Metadata reads from `details` or, through the
 * harness, from `details.details`.
 */
export function parseScreenshotResponse(
  output: unknown,
): ParsedScreenshot | null {
  const parsed = screenshotResponseSchema.safeParse(output)
  if (!parsed.success) return null
  const shot = parsed.data
  const images = shot.content.filter((b) => b.type === 'image' && b.data)
  if (images.length === 0) return null
  const caption = shot.content.find((b) => b.type === 'text')?.text
  const d = shot.details
  const nested = d && 'details' in d
  const meta = nested ? d.details : d
  const hasAccessibility = Boolean(
    shot.accessibility != null || (nested && d.accessibility != null),
  )
  return {
    images,
    caption,
    sessionId: meta?.session_id,
    width: meta?.width ?? meta?.screen?.width,
    height: meta?.height ?? meta?.screen?.height,
    mime: meta?.mime || images[0]?.mime || 'image/png',
    hasAccessibility,
  }
}

/* ---------------- act ---------------- */

export const actRequestSchema = z.object({
  session_id: z.string().optional(),
  action: z.string().optional(),
  x: z.number().nullable().optional(),
  y: z.number().nullable().optional(),
  to_x: z.number().nullable().optional(),
  to_y: z.number().nullable().optional(),
  button: z.string().nullable().optional(),
  text: z.string().nullable().optional(),
  keys: z.array(z.string()).nullable().optional(),
  scroll_x: z.number().nullable().optional(),
  scroll_y: z.number().nullable().optional(),
})
export type ActRequest = z.infer<typeof actRequestSchema>

export const actResponseSchema = z.object({
  ok: z.boolean(),
  detail: z.string(),
})
export type ActResponse = z.infer<typeof actResponseSchema>

/* ---------------- sessions ---------------- */

export const sessionStartRequestSchema = z.object({
  endpoint: z.string().nullable().optional(),
  os: z.string().nullable().optional(),
})
export type SessionStartRequest = z.infer<typeof sessionStartRequestSchema>

export const sessionStartResponseSchema = z.object({
  session_id: z.string(),
  endpoint: z.string(),
  os: z.string(),
  screen: screenSchema,
})
export type SessionStartResponse = z.infer<typeof sessionStartResponseSchema>

export const sessionInfoSchema = z.object({
  session_id: z.string(),
  endpoint: z.string(),
  os: z.string(),
  screen: screenSchema,
  created_ms: z.number().optional(),
  last_used_ms: z.number().optional(),
  screencast_active: z.boolean().optional(),
})
export type SessionInfo = z.infer<typeof sessionInfoSchema>

export const sessionListResponseSchema = z.object({
  sessions: z.array(sessionInfoSchema),
})
export type SessionListResponse = z.infer<typeof sessionListResponseSchema>

export const sessionStopRequestSchema = z.object({
  session_id: z.string().optional(),
})
export type SessionStopRequest = z.infer<typeof sessionStopRequestSchema>

export const sessionStopResponseSchema = z.object({
  ok: z.boolean(),
  was_running: z.boolean(),
})
export type SessionStopResponse = z.infer<typeof sessionStopResponseSchema>

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
