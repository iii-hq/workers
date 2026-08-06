import { CircleQuestionMark, SettingsIcon, X } from 'lucide-react'
import { Fragment, useCallback, useEffect, useRef, useState } from 'react'
import { ChatPanel } from '@/components/chat/ChatPanel'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Sheet } from '@/components/ui/Sheet'
import { Wordmark } from '@/components/ui/Wordmark'
import { EmptyPane } from '@/components/workspace/EmptyPane'
import { EdgeAddZone, ResizeHandle } from '@/components/workspace/pane-controls'
import { TabStrip } from '@/components/workspace/TabStrip'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import {
  hashForExtPage,
  useExtPageRoute,
  useHashRoute,
  type View,
} from '@/hooks/use-hash-route'
import { useTheme } from '@/hooks/use-theme'
import {
  type UseWorkspaceTabsReturn,
  useWorkspaceTabs,
} from '@/hooks/use-workspace-tabs'
import { ConversationsProvider } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import {
  CHAT_SCREEN,
  extPageIdForScreen,
  MAX_COLUMNS,
  MIN_COLUMN_FRACTION,
  screenForView,
  type TabScreen,
  tabColumns,
  tabSizes,
} from '@/lib/workspace-tabs'
import { Browser } from '@/pages/Browser'
import { Configuration } from '@/pages/Configuration'
import { ExtPage } from '@/pages/Ext'
import { Github } from '@/pages/Github'
import { Memory } from '@/pages/Memory'
import { TracesV2 } from '@/pages/TracesV2'
import { Workers } from '@/pages/Workers'
import { Worktrees } from '@/pages/Worktrees'
import type { PanelSide } from '@/types/injectable-ui'

export function App() {
  const [theme, setTheme] = useTheme()
  const [view, setView] = useHashRoute()
  const extPageId = useExtPageRoute()
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  const workspace = useWorkspaceTabs()
  const { activeTab, activeTabId } = workspace

  // An active extension page that disappears (hot-reload failure, worker
  // disconnect, unregister) falls back to the default view.
  const onExtMissing = useCallback(() => {
    setView('traces')
  }, [setView])

  // ── Hash → tabs ──
  // A hash navigation (deep link, in-app `window.location.hash = …`) must
  // land on a tab showing that screen: the active tab if it already does,
  // else an existing tab, else a freshly created one. Guarded by a ref so
  // it only reacts to genuine HASH changes — tab activation must never
  // bounce the hash back. On mount an explicit hash wins over the stored
  // active tab; a bare `#/` defers to it.
  const hashScreen = screenForView(view, extPageId)
  // Deep-link fallback for closing settings from a chat-only/empty tab.
  const lastTabViewRef = useRef<View>('traces')
  useEffect(() => {
    if (view !== 'configuration' && view !== 'ext')
      lastTabViewRef.current = view
  }, [view])
  const lastHashScreenRef = useRef<TabScreen | null>(
    typeof window !== 'undefined' &&
      window.location.hash &&
      window.location.hash !== '#' &&
      window.location.hash !== '#/'
      ? null
      : hashScreen,
  )
  const workspaceRef = useRef(workspace)
  workspaceRef.current = workspace
  // Closing settings routes back to the ACTIVE tab's own screen (never to
  // whichever tab happens to own the previous view — that would switch
  // tabs under the user). Pre-marking keeps the hash-inbound effect quiet.
  const closeSettings = useCallback(() => {
    const primary = workspaceRef.current.activeTab.screens.find(
      (s): s is TabScreen => s !== null && s !== CHAT_SCREEN,
    )
    if (primary) {
      lastHashScreenRef.current = primary
      const extId = extPageIdForScreen(primary)
      if (extId) window.location.hash = hashForExtPage(extId)
      else setView(primary as View)
    } else {
      lastHashScreenRef.current = lastTabViewRef.current
      setView(lastTabViewRef.current)
    }
  }, [setView])
  const toggleSettings = useCallback(() => {
    if (view === 'configuration') closeSettings()
    else setView('configuration')
  }, [view, setView, closeSettings])
  useEffect(() => {
    if (lastHashScreenRef.current === hashScreen) return
    lastHashScreenRef.current = hashScreen
    // No tab representation (settings overlay, unresolved ext route):
    // the tab strip has nothing to react to — and reacting to the ext
    // transient is what used to conjure duplicate tabs.
    if (hashScreen === null) return
    const ws = workspaceRef.current
    if (ws.activeTab.screens.includes(hashScreen)) return
    const existing = ws.tabs.find((t) => t.screens.includes(hashScreen))
    if (existing) ws.activateTab(existing.id)
    else ws.createTab({ columns: 1, screens: [hashScreen] })
  }, [hashScreen])

  // ── Tabs → hash ──
  // Activating a tab whose screens don't cover the current hash points the
  // hash at the tab's first routed screen, so page-internal sub-routes and
  // deep links keep working. Chat-only and empty tabs leave the hash alone.
  const prevActiveTabIdRef = useRef<string | null>(null)
  useEffect(() => {
    const prev = prevActiveTabIdRef.current
    prevActiveTabIdRef.current = activeTabId
    if (prev === null || prev === activeTabId) return
    // hashScreen null (settings overlay open / ext transient): always
    // route to the activated tab's primary screen. The null-safe check
    // matters — `screens.includes(null)` would match an EMPTY column.
    if (hashScreen !== null && activeTab.screens.includes(hashScreen)) return
    const primary = activeTab.screens.find(
      (s): s is TabScreen => s !== null && s !== CHAT_SCREEN,
    )
    if (!primary) return
    // Pre-mark so the hash-inbound effect treats this as already handled.
    lastHashScreenRef.current = primary
    const extId = extPageIdForScreen(primary)
    if (extId) window.location.hash = hashForExtPage(extId)
    else setView(primary as View)
  }, [activeTabId, activeTab, hashScreen, setView])

  /* `?` opens the shortcuts overlay. Ignored when the user is typing into
     editable elements so we don't fight the composer. */
  useEffect(() => {
    if (typeof window === 'undefined') return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== '?') return
      if (e.metaKey || e.ctrlKey || e.altKey) return
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable) return
      const tag = target?.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
      e.preventDefault()
      setShortcutsOpen(true)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  return (
    <ConversationsProvider>
      <Sheet>
        <Header
          workspace={workspace}
          settingsOpen={view === 'configuration'}
          onToggleSettings={toggleSettings}
          onOpenShortcuts={() => setShortcutsOpen(true)}
        />
        <WorkspacePanes workspace={workspace} onExtMissing={onExtMissing} />
        {view === 'configuration' ? (
          <ConfigurationOverlay
            theme={theme}
            onThemeChange={setTheme}
            onClose={closeSettings}
          />
        ) : null}
        <ShortcutsDialog open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
      </Sheet>
    </ConversationsProvider>
  )
}

interface WorkspacePanesProps {
  workspace: UseWorkspaceTabsReturn
  onExtMissing: () => void
}

/**
 * The active tab's columns, each a floating panel over the canvas. An
 * unattached column renders the attach affordance instead of a page.
 * Rendered under `ConversationsProvider` (the screen options need it).
 *
 * Columns are proportioned by the tab's stored `sizes` fractions; the
 * 6px gap between panes is a drag handle (live-resized locally, persisted
 * on release). The container's edge slivers grow the split — hover (or
 * tap) one to reveal the add-panel affordance.
 */
function WorkspacePanes({ workspace, onExtMissing }: WorkspacePanesProps) {
  const { screenOptions } = useScreenOptions()
  const { activeTab } = workspace
  const columns = tabColumns(activeTab)
  const containerRef = useRef<HTMLDivElement>(null)

  // Fractions while a divider drag is live. Committing does NOT clear
  // them: the store notifies through useSyncExternalStore, which doesn't
  // batch with our setState — clearing here would render one frame of
  // the OLD stored sizes (a visible blink) before the write lands. The
  // override instead stays on until the stored sizes catch up, and is
  // dropped in the render below exactly when doing so changes nothing.
  // Keyed so switching tabs or changing the split drops a stale drag
  // instead of applying it to the wrong columns.
  const [dragSizes, setDragSizes] = useState<number[] | null>(null)
  const dragSizesRef = useRef<number[] | null>(null)
  const commitPendingRef = useRef(false)
  const sizesKey = `${activeTab.id}:${columns}`
  const prevSizesKeyRef = useRef(sizesKey)
  if (prevSizesKeyRef.current !== sizesKey) {
    prevSizesKeyRef.current = sizesKey
    dragSizesRef.current = null
    commitPendingRef.current = false
    if (dragSizes !== null) setDragSizes(null)
  }

  const storedSizes = tabSizes(activeTab)
  if (
    dragSizes !== null &&
    commitPendingRef.current &&
    dragSizes.length === storedSizes.length &&
    dragSizes.every((s, i) => Math.abs(s - storedSizes[i]) < 0.001)
  ) {
    // The store caught up with the committed drag — retire the override
    // while it's a visual no-op (guarded render-phase state update).
    commitPendingRef.current = false
    dragSizesRef.current = null
    setDragSizes(null)
  }
  const sizes = dragSizes ?? storedSizes

  const resizePair = (index: number, delta: number) => {
    const current = dragSizesRef.current ?? tabSizes(activeTab)
    // Clamp so neither neighbor dips under the minimum fraction.
    const bounded = Math.max(
      -(current[index] - MIN_COLUMN_FRACTION),
      Math.min(current[index + 1] - MIN_COLUMN_FRACTION, delta),
    )
    if (bounded === 0) return
    const next = [...current]
    next[index] += bounded
    next[index + 1] -= bounded
    commitPendingRef.current = false
    dragSizesRef.current = next
    setDragSizes(next)
  }

  const commitResize = () => {
    const next = dragSizesRef.current
    if (!next) return
    commitPendingRef.current = true
    workspace.resizeColumns(activeTab.id, next)
  }

  return (
    <div
      ref={containerRef}
      className="relative flex-1 flex min-h-0 px-1.5 pb-1.5"
    >
      {Array.from({ length: columns }, (_, column) => {
        const screen = activeTab.screens[column] ?? null
        // 'right' only for the rightmost column of a multi-column tab —
        // a full-width single column keeps the default 'left' orientation.
        const panelSide: PanelSide =
          columns > 1 && column === columns - 1 ? 'right' : 'left'
        // The header ✕ on every screen: in a split the column goes; the
        // last column detaches its screen instead (back to the attach
        // affordance) — a tab never loses its final pane.
        const closePane = () =>
          columns > 1
            ? workspace.removeColumn(activeTab.id, column)
            : workspace.detachScreen(activeTab.id, column)
        const pane = (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: the column POSITION is the identity — the composite key deliberately remounts a pane when its tab or attached screen changes
            key={`${activeTab.id}:${column}:${screen ?? 'empty'}`}
            // ×1000: flex-grow sums below 1 only distribute that fraction
            // of the free space — scaling keeps the ratios AND fills the row.
            style={{ flexGrow: sizes[column] * 1000 }}
            className="basis-0 flex flex-col min-w-0 min-h-0 rounded-sm border border-edge bg-panel overflow-hidden"
          >
            {screen === null ? (
              <EmptyPane
                screenOptions={screenOptions}
                onAttach={(next) =>
                  workspace.attachScreen(activeTab.id, column, next)
                }
                onRemove={
                  columns > 1
                    ? () => workspace.removeColumn(activeTab.id, column)
                    : undefined
                }
              />
            ) : (
              <ScreenBody
                screen={screen}
                panelSide={panelSide}
                tabId={activeTab.id}
                onClose={closePane}
                onExtMissing={onExtMissing}
              />
            )}
          </div>
        )
        if (column === 0) return pane
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: handles are positional by nature
          <Fragment key={`divider:${activeTab.id}:${column}`}>
            <ResizeHandle
              value={sizes[column - 1] * 100}
              onResize={(delta) => resizePair(column - 1, delta)}
              onCommit={commitResize}
              containerWidth={() =>
                containerRef.current?.getBoundingClientRect().width ?? 0
              }
            />
            {pane}
          </Fragment>
        )
      })}

      {columns < MAX_COLUMNS ? (
        <>
          <EdgeAddZone
            side="left"
            onAdd={() => workspace.addColumn(activeTab.id, 'left')}
          />
          <EdgeAddZone
            side="right"
            onAdd={() => workspace.addColumn(activeTab.id, 'right')}
          />
        </>
      ) : null}
    </div>
  )
}

interface ScreenBodyProps {
  screen: TabScreen
  /** Which side of the tab this column occupies (forwarded to ext pages). */
  panelSide: PanelSide
  /** Hosting workspace tab id (forwarded to ext pages for per-tab state). */
  tabId: string
  /** Close this pane — the standard PageHeader ✕ on screens that carry it. */
  onClose: () => void
  onExtMissing: () => void
}

/** One workspace-tab column: the page (or chat view) the screen names.
    Configuration never appears here — it opens as an overlay page. */
function ScreenBody({
  screen,
  panelSide,
  tabId,
  onClose,
  onExtMissing,
}: ScreenBodyProps) {
  const extId = extPageIdForScreen(screen)
  if (extId !== null) {
    return (
      <ExtPage
        pageId={extId}
        panelSide={panelSide}
        tabId={tabId}
        onRequestClose={onClose}
        onMissing={onExtMissing}
      />
    )
  }
  switch (screen) {
    case CHAT_SCREEN:
      // The compact header variant — a tab column is width-constrained the
      // same way the old side dock was, especially in two-column layouts.
      return <ChatPanel density="dock" onRequestClose={onClose} />
    case 'workers':
      return <Workers />
    case 'worktrees':
      return <Worktrees />
    case 'browser':
      return <Browser />
    case 'memory':
      return <Memory />
    case 'github':
      return <Github />
    default:
      return <TracesV2 onRequestClose={onClose} />
  }
}

interface HeaderProps {
  workspace: UseWorkspaceTabsReturn
  settingsOpen: boolean
  onToggleSettings: () => void
  onOpenShortcuts: () => void
}

function Header({
  workspace,
  settingsOpen,
  onToggleSettings,
  onOpenShortcuts,
}: HeaderProps) {
  const { extPageTitles } = useScreenOptions()
  return (
    <header className="flex items-center justify-between gap-3 pl-3 pr-6 h-12 shrink-0">
      <div className="flex items-center gap-3 min-w-0 flex-1">
        <Wordmark />
        <TabStrip
          tabs={workspace.tabs}
          activeTabId={workspace.activeTabId}
          extPageTitles={extPageTitles}
          onActivate={workspace.activateTab}
          onClose={workspace.closeTab}
          onCreate={() => workspace.createTab({ columns: 1 })}
          onRename={workspace.renameTab}
          onReorder={workspace.reorderTab}
        />
      </div>
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onOpenShortcuts}
          aria-label="keyboard shortcuts (?)"
          title="keyboard shortcuts (?)"
          className="font-mono text-[14px] leading-none w-8 h-8 flex items-center justify-center rounded-sm border border-transparent bg-transparent text-ink-faint hover:text-ink hover:bg-surface-hover transition-colors focus-visible:border-accent focus-visible:outline-none"
        >
          <span aria-hidden>
            <CircleQuestionMark className="w-4 h-4" />
          </span>
        </button>
        <button
          type="button"
          onClick={onToggleSettings}
          aria-pressed={settingsOpen}
          aria-label="console settings"
          title="console settings"
          className={cn(
            'font-mono text-[14px] leading-none w-8 h-8 flex items-center justify-center rounded-sm border transition-colors',
            settingsOpen
              ? 'bg-ink text-bg border-transparent'
              : 'bg-transparent text-ink-faint border-transparent hover:text-ink hover:bg-surface-hover',
          )}
        >
          <SettingsIcon className="w-4 h-4" />
        </button>
      </div>
    </header>
  )
}

interface ConfigurationOverlayProps {
  theme: ReturnType<typeof useTheme>[0]
  onThemeChange: (next: ReturnType<typeof useTheme>[0]) => void
  onClose: () => void
}

/**
 * Console settings as a PAGE over the workspace — never a tab screen (the
 * tab model rejects it; `screenForView` maps the route to null). The
 * workspace stays mounted underneath, so closing restores the panes
 * exactly as they were. Deep-linkable via `#/configuration`; Escape or
 * the close affordance returns to the last tab-backed view.
 */
function ConfigurationOverlay({
  theme,
  onThemeChange,
  onClose,
}: ConfigurationOverlayProps) {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return (
    <div className="fixed inset-0 z-40 flex flex-col bg-bg">
      <div className="flex h-12 shrink-0 items-center justify-end pr-6">
        <button
          type="button"
          onClick={onClose}
          aria-label="close settings"
          title="close settings (esc)"
          className="font-mono text-[14px] leading-none w-8 h-8 flex items-center justify-center rounded-sm border border-transparent text-ink-faint hover:text-ink hover:bg-surface-hover transition-colors focus-visible:border-accent focus-visible:outline-none"
        >
          <X className="w-4 h-4" />
        </button>
      </div>
      <div className="flex-1 min-h-0 flex flex-col">
        <Configuration theme={theme} onThemeChange={onThemeChange} />
      </div>
    </div>
  )
}

interface ShortcutsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

const SHORTCUTS: { combo: string; description: string }[] = [
  { combo: '?', description: 'open this shortcut overlay' },
]

function ShortcutsDialog({ open, onOpenChange }: ShortcutsDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogTitle className="text-[11px] uppercase tracking-[0.18em] text-ink-faint">
          keyboard shortcuts
        </DialogTitle>
        <DialogDescription className="mt-1">
          press <kbd className="font-mono text-ink">?</kbd> any time to reopen
          this list.
        </DialogDescription>
        <ul className="mt-4 divide-y divide-rule-2 border-t border-b border-rule-2">
          {SHORTCUTS.map(({ combo, description }) => (
            <li
              key={combo}
              className="flex items-center justify-between gap-6 py-2 font-mono text-[12px] text-ink"
            >
              <span className="text-ink-faint">{description}</span>
              <kbd className="text-ink tracking-[0.06em]">{combo}</kbd>
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  )
}
