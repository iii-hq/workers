/**
 * Typed wrappers over the memory worker's `memory::*` functions for the
 * injected console page.
 *
 * Ported from the console's `lib/memory.ts`: the function ids, payload
 * shapes, and zod parsing are verbatim; only the transport changed — every
 * call now takes the tab's `host` and routes through `host.iii.trigger(...)`
 * instead of a console-internal iii client. The memory functions return
 * their result object directly (no `{ value }` envelope), so reads parse the
 * response as-is.
 *
 * Conventions: zod schemas with optional fields for cross-version tolerance,
 * `safeParse` + drop unparseable rows on reads, plain `trigger` on mutations
 * (errors propagate to the page hook).
 */

import type { Host } from '@iii-dev/console-ui'
import { z } from 'zod'

const provenanceSchema = z.object({
  session_id: z.string().nullish(),
  entry_id: z.string().nullish(),
  agent: z.string().nullish(),
})

const memorySchema = z.object({
  id: z.string(),
  text: z.string(),
  entities: z.array(z.string()).default([]),
  tags: z.array(z.string()).default([]),
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

export async function listBanks(host: Host): Promise<MemoryBank[]> {
  const res = await host.iii.trigger<unknown>('memory::bank::list', {})
  const parsed = bankListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsed.data.banks
    .map((b) => bankInfoSchema.safeParse(b))
    .filter((p): p is { success: true; data: MemoryBank } => p.success)
    .map((p) => p.data)
}

export async function createBank(
  host: Host,
  name: string,
  description?: string,
): Promise<void> {
  await host.iii.trigger('memory::bank::create', { name, description })
}

export const MEMORIES_PAGE_SIZE = 100

export async function listMemories(
  host: Host,
  bank: string,
  includeSuperseded: boolean,
  offset = 0,
  limit = MEMORIES_PAGE_SIZE,
  tag: string | null = null,
): Promise<{ memories: MemoryItem[]; total: number }> {
  const res = await host.iii.trigger<unknown>('memory::list', {
    bank,
    limit,
    offset,
    include_superseded: includeSuperseded,
    ...(tag ? { tag } : {}),
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
  host: Host,
  bank: string,
  text: string,
  pinned: boolean,
): Promise<void> {
  await host.iii.trigger('memory::save', { bank, text, pinned })
}

export async function updateMemory(
  host: Host,
  bank: string,
  id: string,
  text: string,
): Promise<void> {
  await host.iii.trigger('memory::update', { bank, id, text })
}

export async function pinMemory(
  host: Host,
  bank: string,
  id: string,
  pinned: boolean,
): Promise<void> {
  await host.iii.trigger('memory::pin', { bank, id, pinned })
}

export async function deleteMemory(
  host: Host,
  bank: string,
  id: string,
): Promise<void> {
  await host.iii.trigger('memory::delete', { bank, id })
}

export async function listRules(
  host: Host,
  bank: string,
): Promise<MemoryRule[]> {
  const res = await host.iii.trigger<unknown>('memory::rule::list', { bank })
  const parsed = ruleListSchema.safeParse(res)
  if (!parsed.success) return []
  return parsed.data.rules
    .map((r) => ruleSchema.safeParse(r))
    .filter((p): p is { success: true; data: MemoryRule } => p.success)
    .map((p) => p.data)
}

export async function setRule(
  host: Host,
  bank: string,
  name: string,
  content: string,
): Promise<void> {
  await host.iii.trigger('memory::rule::set', { bank, name, content })
}

const tagsSchema = z.object({
  tags: z
    .array(z.object({ tag: z.string(), count: z.number().default(0) }))
    .default([]),
})

/** Distinct tags across a bank's live memories, most used first. */
export async function listTags(
  host: Host,
  bank: string,
): Promise<{ tag: string; count: number }[]> {
  const res = await host.iii.trigger<unknown>('memory::tags', { bank })
  const parsed = tagsSchema.safeParse(res)
  return parsed.success ? parsed.data.tags : []
}

/** Drop the worker's RAM state and re-read every bank from disk — picks
 * up hand-edited rule/memory files. */
export async function reloadStore(host: Host): Promise<void> {
  await host.iii.trigger('memory::reload', {})
}

const previewSchema = z.object({
  bank: z.string(),
  system_prompt_section: z.string().default(''),
  rules: z.number().default(0),
  rules_truncated: z.boolean().default(false),
  memories: z
    .array(z.object({ memory: z.unknown(), score: z.number() }))
    .default([]),
  message: z.string().nullish(),
  retrieval: z.string().default(''),
})

export interface TurnPreview {
  systemPromptSection: string
  rules: number
  rulesTruncated: boolean
  memories: RecalledMemory[]
  message: string | null
  retrieval: string
}

/** The full injection dry-run: exactly what a turn on this bank would be
 * given for a hypothetical chat message — same code path as the hook. */
export async function preview(
  host: Host,
  bank: string,
  query: string,
): Promise<TurnPreview> {
  const res = await host.iii.trigger<unknown>('memory::preview', { bank, query })
  const parsed = previewSchema.safeParse(res)
  if (!parsed.success) {
    return {
      systemPromptSection: '',
      rules: 0,
      rulesTruncated: false,
      memories: [],
      message: null,
      retrieval: '',
    }
  }
  return {
    systemPromptSection: parsed.data.system_prompt_section,
    rules: parsed.data.rules,
    rulesTruncated: parsed.data.rules_truncated,
    memories: parsed.data.memories
      .map((r) => {
        const memory = parseMemory(r.memory)
        return memory ? { memory, score: r.score } : null
      })
      .filter((r): r is RecalledMemory => r !== null),
    message: parsed.data.message ?? null,
    retrieval: parsed.data.retrieval,
  }
}

export async function recall(
  host: Host,
  bank: string,
  query: string,
  limit = 20,
): Promise<{ retrieval: string; memories: RecalledMemory[] }> {
  const res = await host.iii.trigger<unknown>('memory::recall', {
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

/**
 * Jump to the chat conversation a memory came from. The console page called
 * the conversations context's in-memory `select`; the injected page has no
 * such handle, so it persists the active id the console reads at load and
 * routes to the chat/traces view. The active conversation resolves on the
 * next console load.
 */
const ACTIVE_CONVERSATION_KEY = 'iii-chat-active'

export function openConversation(sessionId: string): void {
  if (!sessionId) return
  try {
    localStorage.setItem(ACTIVE_CONVERSATION_KEY, sessionId)
  } catch {
    // storage unavailable; the route change below still lands the user in chat
  }
  window.location.hash = '#/traces'
}
