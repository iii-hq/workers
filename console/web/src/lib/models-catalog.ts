import { makeCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import type { ModelOption } from '@/types/chat'

/** Wire shape returned by `models::list` over the iii bus. */
export interface CatalogModelRow {
  id: string
  provider: string
  display_name: string
}

export async function fetchModelsCatalog(): Promise<CatalogModelRow[]> {
  const client = await getIiiClient()
  const res = await client.call<{ models?: unknown }>('models::list', {})
  const rows = res?.models
  if (!Array.isArray(rows)) return []
  const out: CatalogModelRow[] = []
  for (const raw of rows) {
    if (!raw || typeof raw !== 'object') continue
    const o = raw as Record<string, unknown>
    const id = typeof o.id === 'string' ? o.id : ''
    const provider = typeof o.provider === 'string' ? o.provider : ''
    const display_name =
      typeof o.display_name === 'string' ? o.display_name : id
    if (!id || !provider) continue
    out.push({ id, provider, display_name })
  }
  return out
}

/** Sorted picker options derived from catalog rows. */
export function catalogRowsToModelOptions(
  rows: CatalogModelRow[],
): ModelOption[] {
  const sorted = [...rows].sort((a, b) =>
    a.display_name.localeCompare(b.display_name),
  )
  return sorted.map((m) => ({
    id: makeCatalogModelKey(m.provider, m.id),
    label: m.display_name.toLowerCase(),
  }))
}
