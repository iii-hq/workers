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

const factSchema = z.object({
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
export type MemoryFact = z.infer<typeof factSchema>

const bankInfoSchema = z.object({
  name: z.string(),
  description: z.string().default(''),
  facts: z.number().default(0),
  pinned: z.number().default(0),
  blocks: z.number().default(0),
})
export type MemoryBank = z.infer<typeof bankInfoSchema>

const blockSchema = z.object({
  name: z.string(),
  content: z.string(),
  updated_at: z.number().default(0),
})
export type MemoryBlock = z.infer<typeof blockSchema>

const bankListSchema = z.object({ banks: z.array(z.unknown()).default([]) })
const factListSchema = z.object({
  facts: z.array(z.unknown()).default([]),
  total: z.number().default(0),
})
const blockListSchema = z.object({ blocks: z.array(z.unknown()).default([]) })
const recallSchema = z.object({
  bank: z.string(),
  retrieval: z.string().default(''),
  facts: z
    .array(z.object({ fact: z.unknown(), score: z.number() }))
    .default([]),
})

export interface RecalledFact {
  fact: MemoryFact
  score: number
}

function parseFact(raw: unknown): MemoryFact | null {
  const parsed = factSchema.safeParse(raw)
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

export const FACTS_PAGE_SIZE = 100

export async function listFacts(
  bank: string,
  includeSuperseded: boolean,
  offset = 0,
  limit = FACTS_PAGE_SIZE,
): Promise<{ facts: MemoryFact[]; total: number }> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::list', {
    bank,
    limit,
    offset,
    include_superseded: includeSuperseded,
  })
  const parsed = factListSchema.safeParse(res)
  if (!parsed.success) return { facts: [], total: 0 }
  return {
    facts: parsed.data.facts
      .map(parseFact)
      .filter((f): f is MemoryFact => f !== null),
    total: parsed.data.total,
  }
}

export async function saveFact(
  bank: string,
  text: string,
  pinned: boolean,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::save', { bank, text, pinned })
}

export async function updateFact(
  bank: string,
  id: string,
  text: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::update', { bank, id, text })
}

export async function pinFact(
  bank: string,
  id: string,
  pinned: boolean,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::pin', { bank, id, pinned })
}

export async function deleteFact(bank: string, id: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::delete', { bank, id })
}

export async function listBlocks(bank: string): Promise<MemoryBlock[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::block::list', { bank })
  const parsed = blockListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsed.data.blocks
    .map((b) => blockSchema.safeParse(b))
    .filter((p): p is { success: true; data: MemoryBlock } => p.success)
    .map((p) => p.data)
}

export async function setBlock(
  bank: string,
  name: string,
  content: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::block::set', { bank, name, content })
}

export async function getFact(
  bank: string,
  id: string,
): Promise<MemoryFact | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::get', { bank, id })
  const parsed = z.object({ fact: z.unknown() }).safeParse(res)
  if (!parsed.success) return null
  return parseFact(parsed.data.fact)
}

/** Drop the worker's RAM state and re-read every bank from disk — picks
 * up hand-edited blocks/facts files. */
export async function reloadStore(): Promise<void> {
  const client = await getIiiClient()
  await client.trigger('memory::reload', {})
}

export async function recall(
  bank: string,
  query: string,
  limit = 20,
): Promise<{ retrieval: string; facts: RecalledFact[] }> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>('memory::recall', {
    bank,
    query,
    limit,
  })
  const parsed = recallSchema.safeParse(res)
  if (!parsed.success) return { retrieval: '', facts: [] }
  return {
    retrieval: parsed.data.retrieval,
    facts: parsed.data.facts
      .map((r) => {
        const fact = parseFact(r.fact)
        return fact ? { fact, score: r.score } : null
      })
      .filter((r): r is RecalledFact => r !== null),
  }
}
