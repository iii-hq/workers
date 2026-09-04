/* Per-pane persistence for the explorer's UI state (browsed root, open
   editor tabs, expanded folders), stored in the engine's `configuration`
   worker under the worker-registered `shell-ui` entry (src/ui.rs
   registers it with `initial_value: {}`).

   The entry's value is one JSON object shared by every browser:
   `{ tabs: { [paneKey]: TabUiState } }`, the key being the console's pane
   id (`pane-scope.ts`; older saves sit under the workspace tab id and
   are read as a fallback). `configuration::set`
   replaces the WHOLE value, so writes are read-modify-write and
   debounced; concurrent tabs are last-write-wins (the console's own
   config transport accepts the same trade-off). A missing configuration
   worker degrades to non-persistent silently.

   The read that seeds a pane matters more than any write: a pane that
   boots believing nothing was stored replaces the stored state with its
   defaults on its first save. So a load that fails for any reason other
   than "nothing there" (the worker or the entry missing) is retried a
   few times before the pane gives up and boots fresh — an engine that is
   still coming up after a restart answers within that window. The worker
   side has the matching care: `src/ui.rs` seeds the entry only when
   nothing is stored, because `configuration::register` replaces the
   value whenever a seed is present. */

import type { Host } from '@iii-dev/console-ui'
import type { DiffOptions } from './DiffTab'
import { parseRootMemory, type RootUiState, serializeRootMemory } from './root-memory'
import {
  createTerminalWorkspace,
  normalizeTerminalWorkspace,
  type TerminalWorkspaceState,
} from './terminal-layout'

export const UI_STATE_CONFIG_ID = 'shell-ui'

export type TerminalDock = 'bottom' | 'right' | 'editor'

export interface TabUiState {
  root?: string
  /** True when the user picked `root` here: it then outranks the chat's
      folder on reload (the chat's next move still re-roots the pane). */
  rootPinned?: boolean
  /** What was open in other folders this pane browsed (`root-memory.ts`). */
  roots?: Record<string, RootUiState>
  /** Open tabs in their persisted form (`persistedTabs`); restored through
      `restoreTabs`, which also reads the older `{ path, pinned }` rows. */
  open: unknown[]
  active: string | null
  expanded: string[]
  /** The active sidebar view; absent in legacy saves = explorer. */
  sideView?: string
  diffOptions?: Partial<DiffOptions>
  /** Files-tab dot-entries toggle; absent in legacy saves = hidden. */
  showHidden?: boolean
  /** Sidebar width in px from the drag handle; absent = default. */
  sideWidth?: number
  /** Dockable terminal panel; absent in legacy saves = closed at bottom. */
  terminalOpen?: boolean
  terminalDock?: TerminalDock
  terminalActive?: boolean
  terminalBottomSize?: number
  terminalRightSize?: number
  terminalJobIds?: string[]
  terminalWorkspace?: TerminalWorkspaceState
}

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

/** No configuration worker on this engine: nothing will ever persist. */
function isWorkerMissing(err: unknown): boolean {
  return /function[_ ]not[_ ]found/i.test(messageOf(err))
}

/** The `shell-ui` entry is not registered (yet): nothing stored, and a
    write has to wait for the worker to register it. */
function isEntryMissing(err: unknown): boolean {
  return !isWorkerMissing(err) && /not[_ ]found/i.test(messageOf(err))
}

/** The stored value as an object, `{}` when the entry holds nothing
    usable, null when the worker or the entry is missing. Any other
    failure (the engine still coming up, a transport error) throws so the
    caller can tell "nothing there" from "could not ask". The read is
    `raw`: the page's state carries no `${ENV}` templates, and a raw read
    also survives a null value the schema would otherwise reject. */
async function fetchWholeValue(
  host: Host,
): Promise<Record<string, unknown> | null> {
  try {
    const resp = await host.iii.trigger<{ value?: unknown }>(
      'configuration::get',
      { id: UI_STATE_CONFIG_ID, raw: true },
    )
    const value = resp?.value
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {}
  } catch (err) {
    if (isWorkerMissing(err) || isEntryMissing(err)) return null
    throw err instanceof Error ? err : new Error(String(err))
  }
}

/** Delays between load attempts: about four seconds in all, invisible
    behind the page's own "connecting" state. */
export const LOAD_RETRY_DELAYS_MS: readonly number[] = [300, 1000, 3000]

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

/** `fetchWholeValue` with retries on transient failure; null once the
    retries are spent (the pane then boots fresh, the best it can do). */
async function fetchWholeValueWithRetry(
  host: Host,
  delays: readonly number[],
  wait: (ms: number) => Promise<void>,
): Promise<Record<string, unknown> | null> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await fetchWholeValue(host)
    } catch (err) {
      const delay = delays[attempt]
      if (delay === undefined) {
        console.warn('[shell-ui] could not load the pane state; starting fresh', err)
        return null
      }
      await wait(delay)
    }
  }
}

function entryFor(tabs: Record<string, unknown>, key: string): Record<string, unknown> | null {
  if (key === '') return null
  const entry = tabs[key]
  return entry && typeof entry === 'object' && !Array.isArray(entry) ? (entry as Record<string, unknown>) : null
}

/** The persisted state for one pane; null = none stored (or the
    configuration worker is unavailable — callers can't tell, and don't
    need to). `legacyKey` is the workspace tab id: saves made before
    state was keyed by pane are read from there once. */
export async function loadTabUiState(
  host: Host,
  key: string,
  legacyKey?: string,
  retry: { delays?: readonly number[]; wait?: (ms: number) => Promise<void> } = {},
): Promise<TabUiState | null> {
  if (key === '') return null
  const whole = await fetchWholeValueWithRetry(host, retry.delays ?? LOAD_RETRY_DELAYS_MS, retry.wait ?? sleep)
  const tabs = whole?.tabs
  if (!tabs || typeof tabs !== 'object' || Array.isArray(tabs)) return null
  const raw = entryFor(tabs as Record<string, unknown>, key) ?? (legacyKey ? entryFor(tabs as Record<string, unknown>, legacyKey) : null)
  if (raw === null) return null
  const root = typeof raw.root === 'string' ? raw.root : undefined
  const terminalOpen =
    raw.terminalOpen === true || raw.workspaceMode === 'terminal'
      ? true
      : undefined
  let terminalWorkspace: TerminalWorkspaceState | undefined
  if (raw.terminalWorkspace != null) {
    terminalWorkspace = normalizeTerminalWorkspace(
      raw.terminalWorkspace,
      root ?? '/',
    )
  } else if (terminalOpen) {
    terminalWorkspace = createTerminalWorkspace(root ?? '/')
  }
  return {
    root,
    rootPinned: raw.rootPinned === true ? true : undefined,
    roots: raw.roots === undefined ? undefined : serializeRootMemory(parseRootMemory(raw.roots)),
    open: Array.isArray(raw.open) ? raw.open : [],
    active: typeof raw.active === 'string' ? raw.active : null,
    expanded: Array.isArray(raw.expanded)
      ? raw.expanded.filter((p): p is string => typeof p === 'string')
      : [],
    sideView: typeof raw.sideView === 'string' ? raw.sideView : undefined,
    diffOptions:
      raw.diffOptions && typeof raw.diffOptions === 'object' && !Array.isArray(raw.diffOptions)
        ? (raw.diffOptions as Partial<DiffOptions>)
        : undefined,
    showHidden:
      typeof raw.showHidden === 'boolean' ? raw.showHidden : undefined,
    sideWidth: typeof raw.sideWidth === 'number' ? raw.sideWidth : undefined,
    terminalOpen,
    terminalDock:
      raw.terminalDock === 'bottom' ||
      raw.terminalDock === 'right' ||
      raw.terminalDock === 'editor'
        ? raw.terminalDock
        : undefined,
    terminalActive:
      typeof raw.terminalActive === 'boolean' ? raw.terminalActive : undefined,
    terminalBottomSize:
      typeof raw.terminalBottomSize === 'number'
        ? raw.terminalBottomSize
        : undefined,
    terminalRightSize:
      typeof raw.terminalRightSize === 'number'
        ? raw.terminalRightSize
        : undefined,
    terminalJobIds: Array.isArray(raw.terminalJobIds)
      ? raw.terminalJobIds.filter((id): id is string => typeof id === 'string')
      : undefined,
    terminalWorkspace,
  }
}

/* One debounced writer per (page-instance, pane key): the trailing state
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
        if (isWorkerMissing(err)) {
          unavailable = true
          return
        }
        // The entry not being registered yet (a worker still booting) is
        // not final: the next state change tries again.
        if (!isEntryMissing(err)) console.warn('[shell-ui] failed to persist tab state', err)
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
