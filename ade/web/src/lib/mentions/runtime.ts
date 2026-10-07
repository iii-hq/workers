/**
 * Where mention data comes from. The default runtime talks to the engine:
 * providers are the functions whose registration metadata carries a
 * `mention` descriptor (listed with `include_internal: true` — mention
 * functions are internal), search and get are plain function calls.
 *
 * Mock backends, Storybook and tests swap in their own runtime with
 * `setMentionRuntime`; every cache (providers, searches, views) resets.
 */

import { getIiiClient } from '@/lib/iii-client'
import type { JsonValue } from '@/types/injectable-ui'
import { isValidMentionName } from './token'
import type {
  MentionItem,
  MentionProvider,
  MentionSearchContext,
  MentionView,
} from './types'

export interface MentionSearchOptions {
  query: string
  limit?: number
  context?: MentionSearchContext
  signal?: AbortSignal
}

export interface MentionRuntime {
  listProviders(): Promise<MentionProvider[]>
  search(
    provider: MentionProvider,
    options: MentionSearchOptions,
  ): Promise<MentionItem[]>
  get(provider: MentionProvider, id: string): Promise<MentionView | null>
}

const FUNCTIONS_LIST_RPC = 'engine::functions::list'
const SEARCH_TIMEOUT_MS = 4_000
const GET_TIMEOUT_MS = 8_000

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

/**
 * Providers out of an `engine::functions::list` answer: one per function
 * whose metadata carries a valid descriptor. Two functions claiming the
 * same name: the lexicographically first function id wins (deterministic
 * across tabs and the judge's resolver), and the rest are reported.
 */
export function parseMentionProviders(res: unknown): {
  providers: MentionProvider[]
  conflicts: string[]
} {
  const rows =
    isRecord(res) && Array.isArray(res.functions) ? res.functions : []
  const candidates: MentionProvider[] = []
  for (const row of rows) {
    if (!isRecord(row) || typeof row.function_id !== 'string') continue
    const metadata = row.metadata
    if (!isRecord(metadata) || !isRecord(metadata.mention)) continue
    const raw = metadata.mention
    const name = optionalString(raw.name)
    const label = optionalString(raw.label)
    const search = optionalString(raw.search)
    if (!name || !label || !search || !isValidMentionName(name)) continue
    const details = isRecord(raw.details) ? raw.details : null
    const detailsFn = details ? optionalString(details.function_id) : undefined
    candidates.push({
      v: typeof raw.v === 'number' ? raw.v : 1,
      name,
      label,
      search,
      description: optionalString(raw.description),
      icon: optionalString(raw.icon),
      color: optionalString(raw.color),
      details: detailsFn
        ? {
            function_id: detailsFn,
            id_field: optionalString(details?.id_field) ?? 'id',
          }
        : undefined,
      getFunctionId: row.function_id,
      workerName: optionalString(row.worker_name),
    })
  }
  candidates.sort((a, b) => a.getFunctionId.localeCompare(b.getFunctionId))
  const byName = new Map<string, MentionProvider>()
  const conflicts: string[] = []
  for (const provider of candidates) {
    const first = byName.get(provider.name)
    if (first) {
      conflicts.push(
        `@${provider.name}: ${provider.getFunctionId} ignored, ${first.getFunctionId} already provides it`,
      )
      continue
    }
    byName.set(provider.name, provider)
  }
  const providers = [...byName.values()].sort((a, b) =>
    a.name.localeCompare(b.name),
  )
  return { providers, conflicts }
}

function parseItems(res: unknown): MentionItem[] {
  if (!isRecord(res) || !Array.isArray(res.items)) return []
  const items: MentionItem[] = []
  for (const raw of res.items) {
    if (!isRecord(raw)) continue
    const id = optionalString(raw.id)
    const label = optionalString(raw.label)
    if (!id || !label) continue
    items.push({
      id,
      label,
      hint: optionalString(raw.hint),
      description: optionalString(raw.description),
      icon: optionalString(raw.icon),
      color: optionalString(raw.color),
    })
  }
  return items
}

/** A get answer, or `null` for an unknown id / an unusable shape. */
export function parseMentionView(res: unknown): MentionView | null {
  if (!isRecord(res)) return null
  const id = optionalString(res.id)
  const label = optionalString(res.label)
  if (!id || !label) return null
  const fields = Array.isArray(res.fields)
    ? res.fields.flatMap((field) => {
        if (!isRecord(field)) return []
        const fieldLabel = optionalString(field.label)
        if (!fieldLabel || typeof field.value !== 'string') return []
        return [
          {
            label: fieldLabel,
            value: field.value,
            tone: optionalString(field.tone),
          },
        ]
      })
    : undefined
  let open: MentionView['open']
  if (isRecord(res.open)) {
    const target = res.open
    if (typeof target.page === 'string') {
      open =
        target.context === undefined
          ? { page: target.page }
          : { page: target.page, context: target.context as JsonValue }
    } else if (typeof target.session === 'string') {
      open = { session: target.session }
    } else if (
      typeof target.url === 'string' &&
      /^https?:\/\//i.test(target.url)
    ) {
      open = { url: target.url }
    }
  }
  return {
    id,
    label,
    hint: optionalString(res.hint),
    description: optionalString(res.description),
    icon: optionalString(res.icon),
    color: optionalString(res.color),
    fields,
    open,
    summary: optionalString(res.summary),
    data: res.data,
    updated_at: optionalString(res.updated_at),
  }
}

export const iiiMentionRuntime: MentionRuntime = {
  async listProviders() {
    const client = await getIiiClient()
    const res = await client.trigger<unknown>(FUNCTIONS_LIST_RPC, {
      include_internal: true,
    })
    const { providers, conflicts } = parseMentionProviders(res)
    for (const conflict of conflicts) console.warn(`[mentions] ${conflict}`)
    return providers
  },
  async search(provider, { query, limit, context }) {
    const client = await getIiiClient()
    const res = await client.trigger<unknown>(
      provider.search,
      {
        query,
        ...(limit === undefined ? {} : { limit }),
        ...(context ? { context } : {}),
      },
      { timeoutMs: SEARCH_TIMEOUT_MS },
    )
    return parseItems(res)
  },
  async get(provider, id) {
    const client = await getIiiClient()
    const res = await client.trigger<unknown>(
      provider.getFunctionId,
      { id },
      { timeoutMs: GET_TIMEOUT_MS },
    )
    return parseMentionView(res)
  },
}

let runtime: MentionRuntime = iiiMentionRuntime
let generation = 0
const resetListeners = new Set<() => void>()

export function getMentionRuntime(): MentionRuntime {
  return runtime
}

/** Bumped on every runtime swap; caches keyed on it start over. */
export function mentionRuntimeGeneration(): number {
  return generation
}

/** Swap the runtime (mock backend, stories, tests). Every cache resets. */
export function setMentionRuntime(next: MentionRuntime): void {
  runtime = next
  generation++
  for (const listener of [...resetListeners]) listener()
}

/** Caches register here to drop their state on a runtime swap. */
export function onMentionRuntimeReset(listener: () => void): () => void {
  resetListeners.add(listener)
  return () => resetListeners.delete(listener)
}
