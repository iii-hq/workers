/**
 * Workspace tabs — the header's closable tab strip. Each tab shows one or
 * two SCREENS side by side; a screen is any routed page (`traces`,
 * `workers`, …), a worker-injected page (`ext:<page-id>`), or the chat
 * view (`chat`).
 *
 * Tabs are server-persisted in the engine's `console` configuration entry
 * under `workspace.tabs` (+ the active pointer under
 * `workspace.activeTabId`), following the same read-modify-write
 * convention as the traces saved views (`lib/tracesViews.ts`), so the
 * layout follows the engine — every browser pointing at this engine sees
 * the same tabs.
 */

import type { View } from '@/hooks/use-hash-route'

/** `chat`, a routed first-party view, or `ext:<page-id>`. */
export type TabScreen = string

export interface WorkspaceTab {
  id: string
  /** Optional custom name; the default label derives from the screens. */
  name?: string
  /**
   * Column count (1 or 2). Absent on tabs saved before columns existed —
   * resolve through `tabColumns`, which falls back to the screen count.
   */
  columns?: number
  /**
   * The screen each column shows, index-aligned with the columns. A `null`
   * (or missing) entry is an empty column — the pane renders an attach
   * affordance instead of a page.
   */
  screens: (TabScreen | null)[]
}

/** Effective column count: the explicit choice, else the screen count. */
export function tabColumns(tab: WorkspaceTab): number {
  if (tab.columns === 2) return 2
  if (tab.columns === 1) return 1
  return Math.max(1, tab.screens.length)
}

export const EXT_SCREEN_PREFIX = 'ext:'
export const CHAT_SCREEN: TabScreen = 'chat'

const ROUTED_SCREENS: readonly View[] = [
  'traces',
  'workers',
  'worktrees',
  'browser',
  'memory',
  'github',
  'configuration',
]

export function isRoutedScreen(screen: TabScreen): screen is View {
  return (ROUTED_SCREENS as readonly string[]).includes(screen)
}

export function extPageIdForScreen(screen: TabScreen): string | null {
  return screen.startsWith(EXT_SCREEN_PREFIX)
    ? screen.slice(EXT_SCREEN_PREFIX.length)
    : null
}

export function screenForExtPage(pageId: string): TabScreen {
  return `${EXT_SCREEN_PREFIX}${pageId}`
}

/** The screen a routed view (+ ext page id) resolves to. */
export function screenForView(view: View, extPageId: string | null): TabScreen {
  if (view === 'ext') {
    return extPageId ? screenForExtPage(extPageId) : 'traces'
  }
  return view
}

const isValidScreen = (s: unknown): s is TabScreen =>
  typeof s === 'string' &&
  (s === CHAT_SCREEN ||
    isRoutedScreen(s) ||
    (s.startsWith(EXT_SCREEN_PREFIX) && s.length > EXT_SCREEN_PREFIX.length))

function isValidTab(v: unknown): v is WorkspaceTab {
  if (!v || typeof v !== 'object') return false
  const tab = v as WorkspaceTab
  return (
    typeof tab.id === 'string' &&
    tab.id.length > 0 &&
    Array.isArray(tab.screens) &&
    tab.screens.length <= 2 &&
    tab.screens.every((s) => s === null || isValidScreen(s)) &&
    (tab.name === undefined || typeof tab.name === 'string') &&
    (tab.columns === undefined || tab.columns === 1 || tab.columns === 2)
  )
}

/** A fresh workspace: the classic chat-beside-traces layout, one tab. */
export function defaultTabs(): WorkspaceTab[] {
  return [{ id: 'tab-home', columns: 2, screens: [CHAT_SCREEN, 'traces'] }]
}

/**
 * The classic chat-beside-traces tab — the one the strip lands on when no
 * active pointer is recorded (or the recorded one no longer exists).
 */
export function isDefaultLayoutTab(tab: WorkspaceTab): boolean {
  return tab.screens[0] === CHAT_SCREEN && tab.screens[1] === 'traces'
}

/** `activeTab` resolution: the pointer's tab, else chat+traces, else first. */
export function resolveActiveTab(
  tabs: WorkspaceTab[],
  pointer: string | undefined,
): WorkspaceTab {
  return (
    tabs.find((t) => t.id === pointer) ??
    tabs.find(isDefaultLayoutTab) ??
    tabs[0]
  )
}

/** Parse `workspace.tabs` out of the raw console-config value. */
export function parseWorkspaceTabs(
  configValue: Record<string, unknown>,
): WorkspaceTab[] {
  const workspace = configValue.workspace
  if (!workspace || typeof workspace !== 'object') return []
  const tabs = (workspace as Record<string, unknown>).tabs
  if (!Array.isArray(tabs)) return []
  return tabs.filter(isValidTab)
}

/** Parse `workspace.activeTabId`; `undefined` = no pointer recorded. */
export function parseActiveTabId(
  configValue: Record<string, unknown>,
): string | undefined {
  const workspace = configValue.workspace
  if (!workspace || typeof workspace !== 'object') return undefined
  const id = (workspace as Record<string, unknown>).activeTabId
  return typeof id === 'string' && id.length > 0 ? id : undefined
}

function withWorkspace(
  configValue: Record<string, unknown>,
  patch: Record<string, unknown>,
): Record<string, unknown> {
  const workspace =
    configValue.workspace && typeof configValue.workspace === 'object'
      ? { ...(configValue.workspace as Record<string, unknown>) }
      : {}
  return { ...configValue, workspace: { ...workspace, ...patch } }
}

export function withWorkspaceTabs(
  configValue: Record<string, unknown>,
  tabs: WorkspaceTab[],
): Record<string, unknown> {
  return withWorkspace(configValue, { tabs })
}

export function withActiveTabId(
  configValue: Record<string, unknown>,
  id: string,
): Record<string, unknown> {
  return withWorkspace(configValue, { activeTabId: id })
}

export function newTabId(): string {
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `tab-${crypto.randomUUID()}`
  }
  return `tab-${Math.random().toString(36).slice(2, 10)}`
}

/** Display label for one screen; ext screens resolve through the registry. */
export function screenLabel(
  screen: TabScreen,
  extPageTitles: ReadonlyMap<string, string>,
): string {
  const extId = extPageIdForScreen(screen)
  if (extId !== null) return extPageTitles.get(extId) ?? extId
  return screen
}

/**
 * Tab label: the custom name, else its attached screens joined with `+`,
 * else (nothing attached yet) "new tab".
 */
export function tabLabel(
  tab: WorkspaceTab,
  extPageTitles: ReadonlyMap<string, string>,
): string {
  if (tab.name?.trim()) return tab.name
  const attached = tab.screens.filter((s): s is TabScreen => s !== null)
  if (attached.length === 0) return 'new tab'
  return attached.map((s) => screenLabel(s, extPageTitles)).join(' + ')
}
