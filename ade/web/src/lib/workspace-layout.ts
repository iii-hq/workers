/**
 * Transport for the server-persisted workspace layout — the tab strip, its
 * panes and the active-tab pointer.
 *
 * The layout is ephemeral per-instance state (it changes on every click), so
 * it does NOT live in the `console` configuration entry — whose YAML is meant
 * to be committed — but in `<data_dir>/workspace.json`, owned by the console
 * worker and exposed through `console::workspace::get` / `set`
 * (`ade/src/functions/workspace.rs`). `data_dir` is the entry's `data_dir`
 * setting (default `data/ade`).
 *
 * The value is one JSON object shared by every browser pointing at this
 * engine. `set` replaces the WHOLE document, so writers must
 * read-modify-write (`hooks/lib/workspace-layout-writer.ts` serializes
 * that); concurrent browsers are last-write-wins.
 *
 * When the console worker's functions are unavailable (an older worker, or
 * the engine is unreachable), reads resolve to `null` and the strip degrades
 * to its localStorage copy.
 */

import { getIiiClient } from '@/lib/iii-client'

export const WORKSPACE_GET_FUNCTION_ID = 'console::workspace::get'
export const WORKSPACE_SET_FUNCTION_ID = 'console::workspace::set'

/** The stored document: `tabs`, `activeTabId`, `activatedAt`, `activatedBy`, … */
export type WorkspaceLayoutValue = Record<string, unknown>

/** Identify a missing worker function so reads can fall back without noisy warnings. */
function isUnavailable(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err)
  return /function[_ ]not[_ ]found|not[_ ]found/i.test(message)
}

/**
 * Read the whole layout document. `null` means the console worker does not
 * expose the store — callers should fall back to the local copy rather than
 * erroring. An empty store answers `{}`.
 */
export async function fetchWorkspaceLayout(): Promise<WorkspaceLayoutValue | null> {
  try {
    const client = await getIiiClient()
    const resp = await client.trigger<{ value?: unknown }>(
      WORKSPACE_GET_FUNCTION_ID,
      {},
    )
    const value = resp?.value
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as WorkspaceLayoutValue)
      : {}
  } catch (err) {
    if (isUnavailable(err)) return null
    throw err instanceof Error ? err : new Error(String(err))
  }
}

/** Replace the whole layout document (read-modify-write). */
export async function setWorkspaceLayout(
  value: WorkspaceLayoutValue,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(WORKSPACE_SET_FUNCTION_ID, { value })
}
