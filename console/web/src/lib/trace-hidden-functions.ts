/**
 * Functions whose producers marked them `trace_hidden: true` in their
 * registration metadata (see workers/docs/sops/trace-hidden-functions.md):
 * dispatch machinery and per-turn bookkeeping (session/context managers,
 * `harness::turn`) whose spans would drown trace timelines. The traces
 * page hides these span groups BY DEFAULT; users can unhide them from the
 * funnel menu, and that override persists (`traces.spanFilters.shownGroups`).
 *
 * Separate from `functions-catalog.ts` (the `@`-mention autocomplete):
 * this fetch passes `include_internal: true` — internal plumbing is
 * exactly what tends to be tagged — while the mention catalog must keep
 * internal functions out.
 */

import { getIiiClient } from '@/lib/iii-client'

const FUNCTIONS_LIST_RPC = 'engine::functions::list'

export const EMPTY_HIDDEN_IDS: ReadonlySet<string> = new Set()

function parseTraceHiddenIds(res: unknown): ReadonlySet<string> {
  if (!res || typeof res !== 'object') return EMPTY_HIDDEN_IDS
  const rows = (res as Record<string, unknown>).functions
  if (!Array.isArray(rows)) return EMPTY_HIDDEN_IDS

  const out = new Set<string>()
  for (const raw of rows) {
    if (!raw || typeof raw !== 'object') continue
    const o = raw as Record<string, unknown>
    const id = typeof o.function_id === 'string' ? o.function_id : ''
    if (!id) continue
    const metadata = o.metadata
    if (!metadata || typeof metadata !== 'object') continue
    if ((metadata as Record<string, unknown>).trace_hidden === true) out.add(id)
  }
  return out
}

/**
 * Function ids registered with `trace_hidden: true`. Resolves to the empty
 * set on any failure — the traces page then simply hides nothing by default.
 */
export async function fetchTraceHiddenFunctionIds(): Promise<
  ReadonlySet<string>
> {
  try {
    const client = await getIiiClient()
    const res = await client.trigger<unknown>(FUNCTIONS_LIST_RPC, {
      include_internal: true,
    })
    return parseTraceHiddenIds(res)
  } catch {
    return EMPTY_HIDDEN_IDS
  }
}
