/* Per-console-tab persistence for the explorer's UI state (browsed root,
   open editor tabs, expanded folders), stored in the engine's
   `configuration` worker under the worker-registered `shell-ui` entry
   (src/ui.rs registers it with `initial_value: {}`).

   The entry's value is one JSON object shared by every browser:
   `{ tabs: { [workspaceTabId]: TabUiState } }`. `configuration::set`
   replaces the WHOLE value, so writes are read-modify-write and
   debounced; concurrent tabs are last-write-wins (the console's own
   config transport accepts the same trade-off). A missing configuration
   worker degrades to non-persistent silently. */

import type { Host } from '@iii-dev/console-ui'
import type { OpenTab } from './tabs'

export const UI_STATE_CONFIG_ID = 'shell-ui'

export interface TabUiState {
  root?: string
  open: OpenTab[]
  active: string | null
  expanded: string[]
  /** Files-tab dot-entries toggle; absent in legacy saves = hidden. */
  showHidden?: boolean
  /** Sidebar width in px from the drag handle; absent = default. */
  sideWidth?: number
}

function isUnavailable(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err)
  return /function[_ ]not[_ ]found|not[_ ]found/i.test(message)
}

async function fetchWholeValue(
  host: Host,
): Promise<Record<string, unknown> | null> {
  try {
    const resp = await host.iii.trigger<{ value?: unknown }>(
      'configuration::get',
      { id: UI_STATE_CONFIG_ID },
    )
    const value = resp?.value
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {}
  } catch (err) {
    if (isUnavailable(err)) return null
    throw err instanceof Error ? err : new Error(String(err))
  }
}

/** The persisted state for one workspace tab; null = none stored (or the
    configuration worker is unavailable — callers can't tell, and don't
    need to). */
export async function loadTabUiState(
  host: Host,
  tabId: string,
): Promise<TabUiState | null> {
  if (tabId === '') return null
  const whole = await fetchWholeValue(host).catch(() => null)
  const tabs = whole?.tabs
  if (!tabs || typeof tabs !== 'object' || Array.isArray(tabs)) return null
  const entry = (tabs as Record<string, unknown>)[tabId]
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return null
  const raw = entry as Record<string, unknown>
  return {
    root: typeof raw.root === 'string' ? raw.root : undefined,
    open: Array.isArray(raw.open) ? (raw.open as OpenTab[]) : [],
    active: typeof raw.active === 'string' ? raw.active : null,
    expanded: Array.isArray(raw.expanded)
      ? raw.expanded.filter((p): p is string => typeof p === 'string')
      : [],
  }
}

/* One debounced writer per (page-instance, tabId): the trailing state
   wins, writes are chained so a slow set can't interleave with the next
   read-modify-write. */
const DEBOUNCE_MS = 600

export interface TabUiStateSaver {
  save(state: TabUiState): void
  /** Cancel the pending debounce (unmount). The in-flight write finishes. */
  dispose(): void
}

export function createTabUiStateSaver(
  host: Host,
  tabId: string,
): TabUiStateSaver {
  let timer: number | null = null
  let pending: TabUiState | null = null
  let chain: Promise<void> = Promise.resolve()
  let unavailable = false

  const flush = () => {
    timer = null
    const state = pending
    pending = null
    if (!state || unavailable) return
    chain = chain.then(async () => {
      try {
        const whole = (await fetchWholeValue(host)) ?? {}
        const tabs =
          whole.tabs &&
          typeof whole.tabs === 'object' &&
          !Array.isArray(whole.tabs)
            ? (whole.tabs as Record<string, unknown>)
            : {}
        await host.iii.trigger('configuration::set', {
          id: UI_STATE_CONFIG_ID,
          value: { ...whole, tabs: { ...tabs, [tabId]: state } },
        })
      } catch (err) {
        if (isUnavailable(err)) {
          unavailable = true
          return
        }
        console.warn('[shell-ui] failed to persist tab state', err)
      }
    })
  }

  return {
    save(state) {
      if (tabId === '' || unavailable) return
      pending = state
      if (timer != null) window.clearTimeout(timer)
      timer = window.setTimeout(flush, DEBOUNCE_MS)
    },
    dispose() {
      if (timer != null) {
        window.clearTimeout(timer)
        // Unmount flushes immediately — a closing tab's last state matters.
        flush()
      }
    },
  }
}
