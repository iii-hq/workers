/* Harness-envelope helpers + the wire building blocks the shell family
   shares with the worker's other fs-shaped payloads. Ported from the
   console's sandbox/parsers.ts when the shell function-trigger family
   moved into this worker's injected UI. */

import { z } from 'zod'

/** `engine/src/protocol.rs::StreamChannelRef` (untagged JSON object). */
export const streamChannelRefSchema = z.object({
  channel_id: z.string(),
  access_key: z.string(),
  direction: z.enum(['read', 'write']),
})
export type StreamChannelRef = z.infer<typeof streamChannelRefSchema>

/** `iii-shell-proto::FsEntry`. */
export const fsEntrySchema = z.object({
  name: z.string(),
  is_dir: z.boolean(),
  size: z.number(),
  mode: z.string(),
  mtime: z.number(),
  is_symlink: z.boolean(),
})
export type FsEntry = z.infer<typeof fsEntrySchema>

/** `iii-shell-proto::FsMatch`. `path` is canonical; older guests sent
    `file` — the transform peels that legacy spelling. */
export const fsMatchSchema = z
  .object({
    path: z.string().optional(),
    file: z.string().optional(),
    line: z.number(),
    content: z.string(),
  })
  .transform((m) => ({
    path: m.path ?? m.file ?? '',
    line: m.line,
    content: m.content,
  }))
export type FsMatch = z.infer<typeof fsMatchSchema>

/** `iii-shell-proto::FsSedFileResult`. */
export const fsSedFileResultSchema = z
  .object({
    path: z.string().optional(),
    file: z.string().optional(),
    replacements: z.number(),
    success: z.boolean(),
    error: z.string().nullable().optional(),
  })
  .transform((r) => ({
    path: r.path ?? r.file ?? '',
    replacements: r.replacements,
    success: r.success,
    error: r.error ?? null,
  }))
export type FsSedFileResult = z.infer<typeof fsSedFileResultSchema>

/**
 * The harness wraps every tool result in a `{ content: ContentBlock[],
 * details: unknown, terminate: boolean }` envelope before relaying it to
 * the agent; the console receives the same shape via the engine's
 * function_call output stream.
 *
 * This peels the wrapper so renderers operate on the flat response.
 * Idempotent — calling it on an already-flat payload returns the input
 * unchanged. The discriminator is `Array.isArray(value.content)` —
 * that's the structural marker the harness sets unconditionally for
 * every wrapped result.
 */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return value
  }
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) {
    return obj.details
  }
  return value
}

/** Pull the first balanced `{…}` JSON object out of a string. */
export function extractFirstJsonObject(text: string): unknown | null {
  const start = text.indexOf('{')
  if (start === -1) return null
  let depth = 0
  for (let i = start; i < text.length; i++) {
    const ch = text[i]
    if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) {
        try {
          return JSON.parse(text.slice(start, i + 1))
        } catch {
          return null
        }
      }
    }
  }
  return null
}

/** Text pulled out of a harness `content: ContentBlock[]` array. */
export function contentBlocksText(content: unknown): string | undefined {
  if (!Array.isArray(content)) return undefined
  const parts: string[] = []
  for (const block of content) {
    if (!block || typeof block !== 'object') continue
    const obj = block as Record<string, unknown>
    if (
      obj.type === 'text' &&
      typeof obj.text === 'string' &&
      obj.text.length > 0
    ) {
      parts.push(obj.text)
    }
  }
  return parts.length > 0 ? parts.join('\n') : undefined
}

/** Gather every sub-value that might carry an error payload: the value
    itself, its unwrapped envelope, `value.error`, `error.details`,
    `error.message`, and `content[]` text blocks. */
export function collectErrorCandidates(value: unknown): unknown[] {
  const seen = new Set<unknown>()
  const out: unknown[] = []
  const push = (candidate: unknown) => {
    if (seen.has(candidate)) return
    seen.add(candidate)
    out.push(candidate)
  }

  push(value)
  push(unwrapEnvelope(value))

  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const obj = value as Record<string, unknown>
    if (obj.error && typeof obj.error === 'object') {
      const err = obj.error as Record<string, unknown>
      push(err)
      if ('details' in err) push(err.details)
      if (typeof err.message === 'string') push(err.message)
      const text = contentBlocksText(err.content)
      if (text) push(text)
    }
  }

  return out
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
