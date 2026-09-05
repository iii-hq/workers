/* Per-pane persistence for the explorer's UI state (browsed root, open
   editor tabs, expanded folders, view, options, terminal layout), stored
   by the worker under its data directory (`data/shell/ui-state/panes/
   <pane key>.json`, `src/ui_state.rs`) through two console-only
   functions: `shell::ui-state::get { key, legacy_key }` → `{ state }` and
   `shell::ui-state::set { key, state }`.

   The key is the console's pane id (`pane-scope.ts`); saves made before
   panes had ids sit under the workspace tab id, which the worker reads
   as a fallback (`legacy_key`). One file per pane means a save touches
   only this pane's state: panes never clobber each other, and the worker
   serializes writers of one pane (its later state wins). Nothing here is
   read-modify-write any more — the state used to be one `shell-ui`
   configuration entry holding every pane, which two panes saving at once
   could lose each other's slice of, and which the engine persisted into
   the project's committable `config/` folder.

   The read that seeds a pane matters more than any write: a pane that
   boots believing nothing was stored replaces the stored state with its
   defaults on its first save. So a load that fails for any reason — the
   engine still coming up after a restart, the worker not yet registered
   (`function_not_found`), a transport error — is retried a few times
   before the pane gives up and boots fresh. Only a clean "nothing stored"
   answer is final. */

import type { Host } from '@iii-dev/console-ui'
import type { DiffOptions } from './DiffTab'
import { parseRootMemory, type RootUiState, serializeRootMemory } from './root-memory'
import {
  createTerminalWorkspace,
  normalizeTerminalWorkspace,
  type TerminalWorkspaceState,
} from './terminal-layout'

export const UI_STATE_GET_FN = 'shell::ui-state::get'
export const UI_STATE_SET_FN = 'shell::ui-state::set'

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

/** The stored object for the pane (or its legacy key), null when the
    worker has nothing for either. Any failure throws so the caller can
    tell "nothing there" from "could not ask". */
async function fetchStoredState(
  host: Host,
  key: string,
  legacyKey?: string,
): Promise<Record<string, unknown> | null> {
  const resp = await host.iii.trigger<{ state?: unknown }>(UI_STATE_GET_FN, {
    key,
    legacy_key: legacyKey && legacyKey !== '' ? legacyKey : undefined,
  })
  const state = resp?.state
  return state && typeof state === 'object' && !Array.isArray(state)
    ? (state as Record<string, unknown>)
    : null
}

/** Delays between load attempts: about four seconds in all, invisible
    behind the page's own "connecting" state. */
export const LOAD_RETRY_DELAYS_MS: readonly number[] = [300, 1000, 3000]

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

/** `fetchStoredState` with retries on failure; null once the retries are
    spent (the pane then boots fresh, the best it can do). */
async function fetchStoredStateWithRetry(
  host: Host,
  key: string,
  legacyKey: string | undefined,
  delays: readonly number[],
  wait: (ms: number) => Promise<void>,
): Promise<Record<string, unknown> | null> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await fetchStoredState(host, key, legacyKey)
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

/** The persisted state for one pane; null = none stored (or the worker
    could not be reached — callers can't tell, and don't need to).
    `legacyKey` is the workspace tab id: saves made before state was keyed
    by pane are read from there. */
export async function loadTabUiState(
  host: Host,
  key: string,
  legacyKey?: string,
  retry: { delays?: readonly number[]; wait?: (ms: number) => Promise<void> } = {},
): Promise<TabUiState | null> {
  if (key === '') return null
  const raw = await fetchStoredStateWithRetry(
    host,
    key,
    legacyKey,
    retry.delays ?? LOAD_RETRY_DELAYS_MS,
    retry.wait ?? sleep,
  )
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
   wins, and writes are chained so a slow set can't overtake the next one
   and land an older state last. A failed write is not final: the worker
   may be restarting, and the next state change tries again. */
const DEBOUNCE_MS = 600

export interface TabUiStateSaver {
  save(state: TabUiState): void
  /** Cancel the pending debounce (unmount). The in-flight write finishes. */
  dispose(): void
}

export function createTabUiStateSaver(
  host: Host,
  key: string,
): TabUiStateSaver {
  let timer: number | null = null
  let pending: TabUiState | null = null
  let chain: Promise<void> = Promise.resolve()

  const flush = () => {
    timer = null
    const state = pending
    pending = null
    if (!state) return
    chain = chain.then(async () => {
      try {
        await host.iii.trigger(UI_STATE_SET_FN, { key, state })
      } catch (err) {
        console.warn('[shell-ui] failed to persist pane state', err)
      }
    })
  }

  return {
    save(state) {
      if (key === '') return
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
