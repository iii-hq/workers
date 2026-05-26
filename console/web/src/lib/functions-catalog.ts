import type { FunctionEntry } from '@/lib/functions'
import { getIiiClient } from '@/lib/iii-client'

const FUNCTIONS_LIST_RPC = 'directory::engine::functions::list'

let cache: { entries: FunctionEntry[]; fetchedAt: number } | null = null

function getCacheTtlMs(): number {
  const raw = import.meta.env.VITE_FUNCTIONS_LIST_CACHE_MS
  const parsed = raw ? Number.parseInt(raw, 10) : 10_000
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : 10_000
}

function parseFunctionsCatalogResponse(res: unknown): FunctionEntry[] {
  if (!res || typeof res !== 'object') return []
  const rows = (res as Record<string, unknown>).functions
  if (!Array.isArray(rows)) return []

  const out: FunctionEntry[] = []
  for (const raw of rows) {
    if (!raw || typeof raw !== 'object') continue
    const o = raw as Record<string, unknown>
    const id = typeof o.function_id === 'string' ? o.function_id : ''
    const description = typeof o.description === 'string' ? o.description : ''
    if (!id) continue
    out.push({ id, description })
  }
  return out
}

export async function fetchFunctionsCatalog(): Promise<FunctionEntry[]> {
  const now = Date.now()
  const ttl = getCacheTtlMs()
  if (cache && now - cache.fetchedAt < ttl) {
    return cache.entries
  }

  const client = await getIiiClient()
  const res = await client.call<unknown>(FUNCTIONS_LIST_RPC, {})
  const entries = parseFunctionsCatalogResponse(res)
  cache = { entries, fetchedAt: now }
  return entries
}
