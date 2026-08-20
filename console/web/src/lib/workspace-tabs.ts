/**
 * Workspace tabs — the header's closable tab strip. Each tab shows one or
 * more SCREENS; a screen is any routed page (`traces`,
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

/** Defensive persistence ceiling; the UI no longer has a three-panel cap. */
export const MAX_COLUMNS = 64

export interface WorkspaceTab {
  id: string
  /** Optional custom name; the default label derives from the screens. */
  name?: string
  /**
   * Column count (1..MAX_COLUMNS). Absent on tabs saved before columns
   * existed — resolve through `tabColumns`, which falls back to the
   * screen count.
   */
  columns?: number
  /**
   * The screen each column shows, index-aligned with the columns. A `null`
   * (or missing) entry is an empty column — the pane renders an attach
   * affordance instead of a page.
   */
  screens: (TabScreen | null)[]
  /**
   * Column width fractions (drag-to-resize), index-aligned with the
   * columns. Absent or mismatched = equal widths; normalized through
   * `tabSizes`.
   */
  sizes?: number[]
}

/** Effective column count: the explicit choice, else the screen count. */
export function tabColumns(tab: WorkspaceTab): number {
  const explicit = tab.columns
  if (typeof explicit === 'number' && Number.isInteger(explicit)) {
    return Math.min(MAX_COLUMNS, Math.max(1, explicit))
  }
  return Math.min(MAX_COLUMNS, Math.max(1, tab.screens.length))
}

/** Narrowest a column can be dragged/normalized to, as a fraction. */
export const MIN_COLUMN_FRACTION = 0.12

/**
 * Normalized column fractions (summing to 1, one per column). Stored
 * sizes are honored when they line up with the column count and are all
 * positive finite numbers; anything else falls back to equal widths.
 */
export function tabSizes(tab: WorkspaceTab): number[] {
  const columns = tabColumns(tab)
  const stored = tab.sizes
  if (
    Array.isArray(stored) &&
    stored.length === columns &&
    stored.every((s) => typeof s === 'number' && Number.isFinite(s) && s > 0)
  ) {
    const total = stored.reduce((sum, s) => sum + s, 0)
    return stored.map((s) => s / total)
  }
  return Array.from({ length: columns }, () => 1 / columns)
}

/**
 * Grow the tab by one column on the chosen side (no-op at MAX_COLUMNS).
 * The new column arrives empty at `1/(n+1)` width; existing columns keep
 * their ratios inside the remaining space.
 */
export function withColumnAdded(
  tab: WorkspaceTab,
  side: 'left' | 'right',
): WorkspaceTab {
  const columns = tabColumns(tab)
  if (columns >= MAX_COLUMNS) return tab
  const screens: (TabScreen | null)[] = Array.from(
    { length: columns },
    (_, i) => tab.screens[i] ?? null,
  )
  const sizes = tabSizes(tab)
  const newFraction = 1 / (columns + 1)
  const scaled = sizes.map((s) => s * (1 - newFraction))
  if (side === 'left') {
    screens.unshift(null)
    scaled.unshift(newFraction)
  } else {
    screens.push(null)
    scaled.push(newFraction)
  }
  return { ...tab, columns: columns + 1, screens, sizes: scaled }
}

/**
 * Place `screen` in an existing empty column or insert a new column beside
 * `anchor`. Returns `null` only when the tab is full and has no empty column.
 * Existing screens are never replaced.
 */
export function withScreenOpenedBeside(
  tab: WorkspaceTab,
  screen: TabScreen,
  anchor: TabScreen = CHAT_SCREEN,
): WorkspaceTab | null {
  const columns = tabColumns(tab)
  const screens: (TabScreen | null)[] = Array.from(
    { length: columns },
    (_, i) => tab.screens[i] ?? null,
  )
  if (screens.includes(screen)) return tab

  const anchorIndex = screens.indexOf(anchor)
  const adjacentEmpty =
    anchorIndex >= 0 && screens[anchorIndex + 1] === null ? anchorIndex + 1 : -1
  const emptyIndex = adjacentEmpty >= 0 ? adjacentEmpty : screens.indexOf(null)
  if (emptyIndex >= 0) {
    screens[emptyIndex] = screen
    return { ...tab, columns, screens }
  }
  if (columns >= MAX_COLUMNS) return null

  const insertAt = anchorIndex >= 0 ? anchorIndex + 1 : columns
  screens.splice(insertAt, 0, screen)
  const newFraction = 1 / (columns + 1)
  const sizes = tabSizes(tab).map((size) => size * (1 - newFraction))
  sizes.splice(insertAt, 0, newFraction)
  return { ...tab, columns: columns + 1, screens, sizes }
}

/**
 * Blank one column's screen (the column stays, showing the attach
 * affordance) — the single-column arm of a pane's ✕: the last column
 * never goes, so closing its screen detaches instead.
 */
export function withScreenDetached(
  tab: WorkspaceTab,
  column: number,
): WorkspaceTab {
  const columns = tabColumns(tab)
  if (column < 0 || column >= columns) return tab
  const screens: (TabScreen | null)[] = Array.from(
    { length: columns },
    (_, i) => tab.screens[i] ?? null,
  )
  if (screens[column] === null) return tab
  screens[column] = null
  return { ...tab, columns, screens }
}

/** Drop one column (no-op for the last one); the rest re-normalize. */
export function withColumnRemoved(
  tab: WorkspaceTab,
  column: number,
): WorkspaceTab {
  const columns = tabColumns(tab)
  if (columns <= 1 || column < 0 || column >= columns) return tab
  const screens = Array.from(
    { length: columns },
    (_, i) => tab.screens[i] ?? null,
  ).filter((_, i) => i !== column)
  const remaining = tabSizes(tab).filter((_, i) => i !== column)
  const total = remaining.reduce((sum, s) => sum + s, 0)
  return {
    ...tab,
    columns: columns - 1,
    screens,
    sizes: remaining.map((s) => s / total),
  }
}

export const EXT_SCREEN_PREFIX = 'ext:'
export const CHAT_SCREEN: TabScreen = 'chat'

/**
 * First-party screens whose page migrated to injected UI, mapped to the
 * worker's page id (mirrors MIGRATED_ROUTES in `use-hash-route.ts`).
 * Persisted tabs saved before the migration rewrite through this map.
 */
const MIGRATED_SCREENS: Record<string, string> = {
  worktrees: 'worktree',
  memory: 'memory',
  browser: 'browser',
  github: 'github',
}

/** Configuration is deliberately NOT here: console settings open as an
    overlay page, never as a workspace-tab screen. Per-worker pages
    (worktrees, memory, browser, github) are injected UI — they attach as
    `ext:<page-id>` screens, not first-party routes. */
const ROUTED_SCREENS: readonly View[] = ['traces', 'workers']

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

/**
 * The screen a routed view (+ ext page id) resolves to; `null` when the
 * view has no tab representation — configuration (an overlay page, not a
 * tab screen) and a not-yet-resolved ext route (the view and the page id
 * arrive from two hashchange listeners, so one commit can see `ext` with
 * a null id; reacting to that transient with a fallback screen used to
 * conjure duplicate tabs).
 */
export function screenForView(
  view: View,
  extPageId: string | null,
): TabScreen | null {
  if (view === 'ext') {
    return extPageId ? screenForExtPage(extPageId) : null
  }
  if (view === 'configuration') return null
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
    tab.screens.length <= MAX_COLUMNS &&
    tab.screens.every((s) => s === null || isValidScreen(s)) &&
    (tab.name === undefined || typeof tab.name === 'string') &&
    (tab.columns === undefined ||
      (Number.isInteger(tab.columns) &&
        (tab.columns as number) >= 1 &&
        (tab.columns as number) <= MAX_COLUMNS)) &&
    (tab.sizes === undefined ||
      (Array.isArray(tab.sizes) &&
        tab.sizes.length <= MAX_COLUMNS &&
        tab.sizes.every(
          (s) => typeof s === 'number' && Number.isFinite(s) && s > 0,
        )))
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
  // Migration: 'configuration' was a tab screen before it became an
  // overlay page — blank those columns instead of dropping whole tabs.
  // The per-worker pages were first-party screens before they moved to
  // injected UI — rewrite them to their `ext:<page-id>` form.
  const sanitized = tabs.map((t) => {
    if (
      !t ||
      typeof t !== 'object' ||
      !Array.isArray((t as WorkspaceTab).screens)
    )
      return t
    const tab = t as WorkspaceTab
    return {
      ...tab,
      screens: tab.screens.map((s) => {
        if (s === 'configuration') return null
        if (typeof s === 'string' && s in MIGRATED_SCREENS)
          return screenForExtPage(MIGRATED_SCREENS[s])
        return s
      }),
    }
  })
  return sanitized.filter(isValidTab)
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

/** Where the tabs in hand come from: `pending` until the first server
    answer, `server` while the configuration entry is readable, `local` when
    it is not (the localStorage copy). */
export type WorkspaceLayoutSource = 'pending' | 'server' | 'local'

/** A queued pre-hydration write should replay once the layout source is no
    longer `pending` (server hydrated, or known unavailable) and something is
    actually queued. */
export function shouldFlushPendingWrite(
  source: WorkspaceLayoutSource,
  hasPending: boolean,
): boolean {
  return source !== 'pending' && hasPending
}

export function workspaceLayoutSource(
  fetched: boolean,
  available: boolean,
): WorkspaceLayoutSource {
  if (available) return 'server'
  return fetched ? 'local' : 'pending'
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

export interface OpenWorkspaceScreenResult {
  tabs: WorkspaceTab[]
  activeTabId: string
}

/**
 * Reuse a screen wherever it is already mounted; otherwise place it beside
 * chat in the active tab without replacing any pane. A saturated workspace
 * gets a fresh chat + context tab.
 */
export function withWorkspaceScreenOpened(
  tabs: WorkspaceTab[],
  activeTabId: string,
  screen: TabScreen,
  makeTabId: () => string = newTabId,
): OpenWorkspaceScreenResult {
  const active = tabs.find((tab) => tab.id === activeTabId) ?? tabs[0]
  // Where you already are beats where it happens to be mounted: opening chat
  // from a workspace that shows chat should stay put, not send you to
  // whichever other tab was created first.
  if (active?.screens.includes(screen)) {
    return { tabs, activeTabId: active.id }
  }

  const existing = tabs.find((tab) => tab.screens.includes(screen))
  if (existing) return { tabs, activeTabId: existing.id }

  if (active) {
    const placed = withScreenOpenedBeside(active, screen)
    if (placed) {
      return {
        tabs: tabs.map((tab) => (tab.id === active.id ? placed : tab)),
        activeTabId: active.id,
      }
    }
  }

  const tab: WorkspaceTab = {
    id: makeTabId(),
    columns: 2,
    screens: [CHAT_SCREEN, screen],
  }
  return { tabs: [...tabs, tab], activeTabId: tab.id }
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
