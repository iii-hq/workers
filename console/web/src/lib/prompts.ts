import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Typed wrappers over the iii-directory worker's `directory::prompts::*`
 * functions — the filesystem-backed prompt library behind the chat's
 * system-prompt picker. Same conventions as `memory.ts`: zod schemas with
 * tolerant defaults, `safeParse` + drop unparseable rows on reads, plain
 * `trigger` on mutations (errors propagate to the caller).
 */

export type PromptStrategy = 'enrich' | 'override'

const promptEntrySchema = z.object({
  name: z.string(),
  description: z.string().default(''),
  // Absent (older workers) or unknown values degrade to the safe default.
  strategy: z.enum(['enrich', 'override']).catch('enrich'),
  modified_at: z.string().default(''),
})
export type PromptEntry = z.infer<typeof promptEntrySchema>

const promptListSchema = z.object({ prompts: z.array(z.unknown()).default([]) })

const promptGetSchema = promptEntrySchema.extend({ body: z.string() })
export type PromptWithBody = z.infer<typeof promptGetSchema>

/** Mirrors the server-side name rule (UX pre-flight only; the server's
 * `validate_name` is the authority). */
export const PROMPT_NAME_RE = /^[a-z0-9_-]{1,64}$/

/** Parser for `directory::prompts::list` rows; drops invalid rows. */
export function parsePromptEntries(rows: unknown[]): PromptEntry[] {
  return rows
    .map((p) => promptEntrySchema.safeParse(p))
    .filter((p): p is { success: true; data: PromptEntry } => p.success)
    .map((p) => p.data)
}

/** Parser for the `directory::prompts::get` response; null on mismatch. */
export function parsePromptWithBody(raw: unknown): PromptWithBody | null {
  const parsed = promptGetSchema.safeParse(raw)
  return parsed.success ? parsed.data : null
}

export async function listPrompts(): Promise<PromptEntry[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('directory::prompts::list', {})
  const parsed = promptListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsePromptEntries(parsed.data.prompts)
}

/**
 * THROWS on trigger error or shape mismatch — the send path relies on a
 * failed fetch failing the send instead of silently dropping the selected
 * prompt.
 */
export async function getPrompt(name: string): Promise<PromptWithBody> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('directory::prompts::get', { name })
  const parsed = parsePromptWithBody(res)
  if (!parsed) {
    throw new Error(
      `directory::prompts::get returned an unexpected shape for "${name}"`,
    )
  }
  return parsed
}

/** Create-only: the worker rejects names that already exist. */
export async function createPrompt(input: {
  name: string
  body: string
  strategy: PromptStrategy
  description?: string
}): Promise<{ name: string }> {
  const client = await getIiiClient()
  await client.trigger('directory::prompts::save', {
    name: input.name,
    body: input.body,
    strategy: input.strategy,
    ...(input.description ? { description: input.description } : {}),
  })
  return { name: input.name }
}
