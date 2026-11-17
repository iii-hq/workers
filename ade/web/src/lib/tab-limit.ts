/**
 * How many workspace tabs a browser may hold, as the `console` configuration
 * entry states it under `tabs.limit`. The Rust side declares the key and its
 * default in the entry's schema (ade/src/configuration.rs — keep the number
 * in sync); the web reads it here and falls back to the same default when
 * the entry is absent, unreadable or hand-edited into nonsense, so the strip
 * always has a limit and never a hole.
 *
 * Only creation is limited. Tabs already open above a lowered limit stay
 * open and usable — the limit says nothing can be added, not that
 * something must go.
 */

import type { ConsoleConfigValue } from '@/lib/console-config'

/** The nine keyboard-addressable tabs plus one: the strip shares its width
 *  among them and never has to scroll at the default. */
export const DEFAULT_TAB_LIMIT = 10

/** The schema's floor and ceiling; a stored value outside them is clamped. */
export const MIN_TAB_LIMIT = 1
export const MAX_TAB_LIMIT = 64

/** Parse `tabs.limit` out of the raw console-config value, clamped to the
 *  schema's range; the default when the key is absent or not a number. */
export function parseTabLimit(
  configValue: ConsoleConfigValue | null | undefined,
): number {
  const tabs = configValue?.tabs
  if (!tabs || typeof tabs !== 'object') return DEFAULT_TAB_LIMIT
  const limit = (tabs as Record<string, unknown>).limit
  if (typeof limit !== 'number' || !Number.isFinite(limit)) {
    return DEFAULT_TAB_LIMIT
  }
  return Math.min(MAX_TAB_LIMIT, Math.max(MIN_TAB_LIMIT, Math.floor(limit)))
}

/** Whether one more tab may open under `limit`. */
export function canCreateTab(tabCount: number, limit: number): boolean {
  return tabCount < limit
}

/** The reason a new-tab control is disabled, as its hover title. */
export function tabLimitReachedTitle(limit: number): string {
  return `Tab limit reached (${limit}). Close a tab to open another, or raise the limit in the ADE configuration.`
}

type TabLimitListener = () => void
const tabLimitListeners = new Set<TabLimitListener>()

/**
 * Say the limit out loud: a refused ⌃T has no disabled `+` under the
 * pointer to explain itself, so without this it did nothing, silently.
 */
export function requestTabLimitNotice(): void {
  for (const listener of tabLimitListeners) listener()
}

export function onTabLimitNotice(listener: TabLimitListener): () => void {
  tabLimitListeners.add(listener)
  return () => {
    tabLimitListeners.delete(listener)
  }
}

/**
 * Open a workspace, or — at the limit — say why it cannot. Returns whether
 * one opened.
 */
export function createTabOrNotice(workspace: {
  canCreateTab: boolean
  createTab: (opts: { columns: 1 }) => void
}): boolean {
  if (!workspace.canCreateTab) {
    requestTabLimitNotice()
    return false
  }
  workspace.createTab({ columns: 1 })
  return true
}
