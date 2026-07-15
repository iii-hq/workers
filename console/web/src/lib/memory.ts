import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Typed wrappers over the memory worker's `memory::*` functions. Same
 * conventions as `worktrees.ts`: zod schemas with optional fields for
 * cross-version tolerance, `safeParse` + drop unparseable rows on reads,
 * plain `trigger` on mutations (errors propagate to the page hook).
 */

const provenanceSchema = z.object({
  session_id: z.string().nullish(),
  entry_id: z.string().nullish(),
  agent: z.string().nullish(),
})

const memorySchema = z.object({
  id: z.string(),
  text: z.string(),
  entities: z.array(z.string()).default([]),
  confidence: z.string().default('extracted'),
  corroboration: z.number().default(0),
  pinned: z.boolean().default(false),
  source: provenanceSchema.nullish(),
  created_at: z.number(),
  updated_at: z.number(),
  invalid_at: z.number().nullish(),
  superseded_by: z.string().nullish(),
  revision: z.number().default(0),
})
export type MemoryItem = z.infer<typeof memorySchema>

const bankInfoSchema = z.object({
  name: z.string(),
  description: z.string().default(''),
  memories: z.number().default(0),
  pinned: z.number().default(0),
  rules: z.number().default(0),
})
export type MemoryBank = z.infer<typeof bankInfoSchema>

const ruleSchema = z.object({
  name: z.string(),
  content: z.string(),
  updated_at: z.number().default(0),
})
export type MemoryRule = z.infer<typeof ruleSchema>

const bankListSchema = z.object({ banks: z.array(z.unknown()).default([]) })
const memoryListSchema = z.object({
  memories: z.array(z.unknown()).default([]),
  total: z.number().default(0),
})
const ruleListSchema = z.object({ rules: z.array(z.unknown()).default([]) })
const recallSchema = z.object({
  bank: z.string(),
  retrieval: z.string().default(''),
  memories: z
    .array(z.object({ memory: z.unknown(), score: z.number() }))
    .default([]),
})

export interface RecalledMemory {
  memory: MemoryItem
  score: number
}

function parseMemory(raw: unknown): MemoryItem | null {
  const parsed = memorySchema.safeParse(raw)
  return parsed.success ? parsed.data : null
}

export async function listBanks(): Promise<MemoryBank[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::bank::list', {})
  const parsed = bankListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsed.data.banks
    .map((b) => bankInfoSchema.safeParse(b))
    .filter((p): p is { success: true; data: MemoryBank } => p.success)
    .map((p) => p.data)
}

export async function createBank(
  name: string,
  description?: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::bank::create', { name, description })
}

export async function deleteBank(name: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::bank::delete', { name })
}

export const MEMORIES_PAGE_SIZE = 100

export async function listMemories(
  bank: string,
  includeSuperseded: boolean,
  offset = 0,
  limit = MEMORIES_PAGE_SIZE,
): Promise<{ memories: MemoryItem[]; total: number }> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::list', {
    bank,
    limit,
    offset,
    include_superseded: includeSuperseded,
  })
  const parsed = memoryListSchema.safeParse(res)
  if (!parsed.success) return { memories: [], total: 0 }
  return {
    memories: parsed.data.memories
      .map(parseMemory)
      .filter((m): m is MemoryItem => m !== null),
    total: parsed.data.total,
  }
}

export async function saveMemory(
  bank: string,
  text: string,
  pinned: boolean,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::save', { bank, text, pinned })
}

export async function updateMemory(
  bank: string,
  id: string,
  text: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::update', { bank, id, text })
}

export async function pinMemory(
  bank: string,
  id: string,
  pinned: boolean,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::pin', { bank, id, pinned })
}

export async function deleteMemory(bank: string, id: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::delete', { bank, id })
}

export async function listRules(bank: string): Promise<MemoryRule[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::rule::list', { bank })
  const parsed = ruleListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsed.data.rules
    .map((r) => ruleSchema.safeParse(r))
    .filter((p): p is { success: true; data: MemoryRule } => p.success)
    .map((p) => p.data)
}

export async function setRule(
  bank: string,
  name: string,
  content: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::rule::set', { bank, name, content })
}

export async function getMemory(
  bank: string,
  id: string,
): Promise<MemoryItem | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::get', { bank, id })
  const parsed = z.object({ memory: z.unknown() }).safeParse(res)
  if (!parsed.success) return null
  return parseMemory(parsed.data.memory)
}

/** Drop the worker's RAM state and re-read every bank from disk — picks
 * up hand-edited rule/memory files. */
export async function reloadStore(): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::reload', {})
}

export async function recall(
  bank: string,
  query: string,
  limit = 20,
): Promise<{ retrieval: string; memories: RecalledMemory[] }> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::recall', {
    bank,
    query,
    limit,
  })
  const parsed = recallSchema.safeParse(res)
  if (!parsed.success) return { retrieval: '', memories: [] }
  return {
    retrieval: parsed.data.retrieval,
    memories: parsed.data.memories
      .map((r) => {
        const memory = parseMemory(r.memory)
        return memory ? { memory, score: r.score } : null
      })
      .filter((r): r is RecalledMemory => r !== null),
  }
}
