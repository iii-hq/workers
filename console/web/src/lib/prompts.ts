import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Typed wrappers over the directory worker's `directory::prompts::*`
 * functions. Same conventions as `memory.ts`: zod schemas with defaults
 * for cross-version tolerance, `safeParse` + drop unparseable rows on
 * reads, plain `trigger` on mutations (errors propagate to the page).
 *
 * Two kinds flow through one store: `command` (slash-style templates
 * injected into the message context) and `system` (full system prompts
 * applied via the router override or the send/spawn `system_prompt`
 * options). `source` separates worker-shipped templates (read-only from
 * the console's point of view) from user library entries.
 */

const promptRowSchema = z.object({
  name: z.string(),
  description: z.string().default(''),
  kind: z.string().default('command'),
  source: z.string().default('worker'),
  modified_at: z.string().default(''),
})
export type PromptRow = z.infer<typeof promptRowSchema>

const promptListSchema = z.object({
  prompts: z.array(z.unknown()).default([]),
})

const promptDetailSchema = promptRowSchema.extend({
  body: z.string().default(''),
})
export type PromptDetail = z.infer<typeof promptDetailSchema>

export async function listPrompts(): Promise<PromptRow[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('directory::prompts::list', {})
  const parsed = promptListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsed.data.prompts
    .map((p) => promptRowSchema.safeParse(p))
    .filter((p): p is { success: true; data: PromptRow } => p.success)
    .map((p) => p.data)
}

export async function getPrompt(name: string): Promise<PromptDetail | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('directory::prompts::get', { name })
  const parsed = promptDetailSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function savePrompt(input: {
  name: string
  description: string
  body: string
  kind: string
}): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('directory::prompts::save', input)
}

export async function deletePrompt(name: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('directory::prompts::delete', { name, yes: true })
}
