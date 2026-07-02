/**
 * Parsers for the `state::*` family (`state::get|set|delete|update|list|
 * list_groups`). All ops are keyed by `scope` (+ `key`, except `list` /
 * `list_groups`); stored values are arbitrary JSON. The minimal view only
 * surfaces `scope`/`key` from the request and renders the unwrapped result
 * as JSON, so the request schema stays deliberately loose.
 *
 * Wire source: engine state worker (`state::*` are engine built-ins).
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

/** Every `state::` id is handled by this family (prefix match). */
export function isStateFunction(id: string): boolean {
  return id.startsWith('state::')
}

export const stateRequestSchema = z.object({
  scope: z.string().optional(),
  key: z.string().optional(),
})
export type StateRequest = z.infer<typeof stateRequestSchema>

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}
