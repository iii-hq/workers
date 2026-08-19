import {
  CircleQuestionMark,
  Menu,
  Plus,
  Search,
  SettingsIcon,
  SquarePen,
  X,
} from 'lucide-react'
import {
  type CSSProperties,
  Fragment,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import { ChatPanel } from '@/components/chat/ChatPanel'
import { PaletteHost } from '@/components/PaletteHost'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { Sheet } from '@/components/ui/Sheet'
import { Wordmark } from '@/components/ui/Wordmark'
import { EmptyPane } from '@/components/workspace/EmptyPane'
import { MobileWorkspaceMenu } from '@/components/workspace/MobileWorkspaceMenu'
import { EdgeAddZone, ResizeHandle } from '@/components/workspace/pane-controls'
import { TabStrip } from '@/components/workspace/TabStrip'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import {
  hashForExtPage,
  useExtPageRoute,
  useHashRoute,
  type View,
} from '@/hooks/use-hash-route'
import { useKeybindings } from '@/hooks/use-keybindings'
import { useTheme } from '@/hooks/use-theme'
import {
  type UseWorkspaceTabsReturn,
  useWorkspaceTabs,
} from '@/hooks/use-workspace-tabs'
import {
  ConversationsProvider,
  type InjectableUiRuntime,
  useConversationsCtx,
} from '@/lib/conversations-context'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import { keybindingGroups, resolveBindings } from '@/lib/keybindings/registry'
import { subscribePanelOpen } from '@/lib/panel-context'
import { loadEdgeAddDiscovered, saveEdgeAddDiscovered } from '@/lib/storage'
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
import { Configuration } from '@/pages/Configuration'
import { ExtPage } from '@/pages/Ext'
import { TracesV2 } from '@/pages/TracesV2'
import { Workers } from '@/pages/Workers'
import type { PanelSide } from '@/types/injectable-ui'

export function App({
  injectableUiRuntime,
}: {
  injectableUiRuntime?: Promise<InjectableUiRuntime>
}) {
  const [theme, setTheme] = useTheme()
  const [view, setView] = useHashRoute()
  const extPageId = useExtPageRoute()
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  // Held here, not in `PaletteHost`: ⌘K is one way in and the phone header's
  // search affordance is the other, so the state has to sit above both.
  const [paletteOpen, setPaletteOpen] = useState(false)
  const workspace = useWorkspaceTabs()
  const { activeTab, activeTabId } = workspace
  const [mobilePanelIndex, setMobilePanelIndex] = useState(0)

  const mobileActiveTabRef = useRef(activeTabId)
  useEffect(() => {
    if (mobileActiveTabRef.current === activeTabId) return
    mobileActiveTabRef.current = activeTabId
    setMobilePanelIndex(0)
  }, [activeTabId])

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
  useEffect(
    () =>
      subscribePanelOpen((event) => {
        workspaceRef.current.openScreen(`ext:${event.pageId}`)
      }),
    [],
  )
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

  /* Every global shortcut the console has, dispatched from the registry.
     Which keys reach here while the caret is in a field, and which stand
     down, is the registry's call — not this component's. */
  useKeybindings({
    'palette.toggle': () => setPaletteOpen((current) => !current),
    'shortcuts.open': () => setShortcutsOpen(true),
    'app.settings': toggleSettings,
    'workspace.create': () => workspaceRef.current.createTab({ columns: 1 }),
    'panel.split': () =>
      workspaceRef.current.addColumn(
        workspaceRef.current.activeTab.id,
        'right',
      ),
    // Out of range is a no-op rather than a wrap: pressing 7 with four
    // workspaces open should do nothing, not land somewhere surprising.
    'workspace.selectByIndex': (index) => {
      const tab = workspaceRef.current.tabs[index]
      if (tab) workspaceRef.current.activateTab(tab.id)
    },
  })

  return (
    <ConversationsProvider injectableUiRuntime={injectableUiRuntime}>
      <Sheet>
        <Header
          workspace={workspace}
          mobilePanelIndex={mobilePanelIndex}
          onMobilePanelIndexChange={setMobilePanelIndex}
          settingsOpen={view === 'configuration'}
          onToggleSettings={toggleSettings}
          onOpenShortcuts={() => setShortcutsOpen(true)}
          onOpenPalette={() => setPaletteOpen(true)}
        />
        <WorkspacePanes
          workspace={workspace}
          mobilePanelIndex={mobilePanelIndex}
          onMobilePanelIndexChange={setMobilePanelIndex}
          onExtMissing={onExtMissing}
        />
        {view === 'configuration' ? (
          <ConfigurationOverlay
            theme={theme}
            onThemeChange={setTheme}
            onClose={closeSettings}
          />
        ) : null}
        <ShortcutsDialog open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
        <PaletteHost
          open={paletteOpen}
          onOpenChange={setPaletteOpen}
          openScreen={workspace.openScreen}
          onOpenSettings={() => setView('configuration')}
          onOpenShortcuts={() => setShortcutsOpen(true)}
          theme={theme}
          onThemeChange={setTheme}
        />
      </Sheet>
    </ConversationsProvider>
  )
}

interface WorkspacePanesProps {
  workspace: UseWorkspaceTabsReturn
  mobilePanelIndex: number
  onMobilePanelIndexChange: (index: number) => void
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
function WorkspacePanes({
  workspace,
  mobilePanelIndex,
  onMobilePanelIndexChange,
  onExtMissing,
}: WorkspacePanesProps) {
  const { screenOptions } = useScreenOptions()
  const { activeTab } = workspace
  const columns = tabColumns(activeTab)
  const containerRef = useRef<HTMLElement>(null)
  const pendingMobileColumnRef = useRef<{
    tabId: string
    index: number
  } | null>(null)

  // First-run discoverability for the edge add zones: nudge until the user
  // adds a panel THROUGH a zone (either side), then remember in localStorage
  // so it never plays again. Deliberately not inferred from existing splits —
  // the default workspace already ships a 2-column tab.
  const [edgeNudge, setEdgeNudge] = useState(() => !loadEdgeAddDiscovered())
  const addEdgeColumn = useCallback(
    (side: 'left' | 'right') => {
      if (edgeNudge) {
        saveEdgeAddDiscovered()
        setEdgeNudge(false)
      }
      workspace.addColumn(activeTab.id, side)
    },
    [activeTab.id, edgeNudge, workspace],
  )

  const addMobileColumn = useCallback(() => {
    if (columns >= MAX_COLUMNS || pendingMobileColumnRef.current) return
    pendingMobileColumnRef.current = {
      tabId: activeTab.id,
      index: columns,
    }
    addEdgeColumn('right')
  }, [activeTab.id, addEdgeColumn, columns])

  // A tab switch always lands on its first panel. Inside a tab, native
  // horizontal scrolling does the gesture work and this index only mirrors
  // the snapped position for the header indicator.
  useEffect(() => {
    const container = containerRef.current
    if (!container || container.dataset.activeTab === activeTab.id) return
    pendingMobileColumnRef.current = null
    container.dataset.activeTab = activeTab.id
    container.scrollTo({ left: 0, behavior: 'auto' })
  }, [activeTab.id])

  useEffect(() => {
    const pending = pendingMobileColumnRef.current
    if (pending?.tabId === activeTab.id && pending.index < columns) {
      pendingMobileColumnRef.current = null
      onMobilePanelIndexChange(pending.index)
      const container = containerRef.current
      if (container) {
        container.scrollTo({
          left: container.clientWidth * pending.index,
          behavior: 'auto',
        })
      }
      return
    }
    if (mobilePanelIndex < columns) return
    const next = Math.max(0, columns - 1)
    onMobilePanelIndexChange(next)
    const container = containerRef.current
    if (container) container.scrollTo({ left: container.clientWidth * next })
  }, [activeTab.id, columns, mobilePanelIndex, onMobilePanelIndexChange])

  const handleMobileScroll = useCallback(
    (element: HTMLElement) => {
      if (window.matchMedia('(min-width: 640px)').matches) return
      const width = element.clientWidth
      if (width <= 0) return
      const position = element.scrollLeft / width
      // The extra snap page after the last real panel is a creation gesture.
      // Wait until it is effectively reached so an ordinary partial drag does
      // not create panels accidentally.
      if (columns < MAX_COLUMNS && position >= columns - 0.05) {
        addMobileColumn()
        return
      }
      const next = Math.max(0, Math.min(columns - 1, Math.round(position)))
      if (next !== mobilePanelIndex) onMobilePanelIndexChange(next)
    },
    [addMobileColumn, columns, mobilePanelIndex, onMobilePanelIndexChange],
  )

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
    // With many horizontally scrollable panels, an equal share can be below
    // the normal split minimum. Scale the floor with the panel count so the
    // resize math remains valid instead of forcing a pair in one direction.
    const minFraction = Math.min(MIN_COLUMN_FRACTION, 1 / (columns * 2))
    const bounded = Math.max(
      -(current[index] - minFraction),
      Math.min(current[index + 1] - minFraction, delta),
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
    <section
      ref={containerRef}
      onScroll={(event) => handleMobileScroll(event.currentTarget)}
      className="relative flex min-h-0 flex-1 snap-x snap-mandatory overflow-x-auto overflow-y-hidden pb-0 sm:snap-none sm:px-4 sm:pb-1.5"
      aria-label="workspace panels"
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
          <section
            // biome-ignore lint/suspicious/noArrayIndexKey: the column POSITION is the identity — the composite key deliberately remounts a pane when its tab or attached screen changes
            key={`${activeTab.id}:${column}:${screen ?? 'empty'}`}
            // ×1000: flex-grow sums below 1 only distribute that fraction
            // of the free space — scaling keeps the ratios AND fills the row.
            style={
              {
                '--panel-grow': sizes[column] * 1000,
              } as CSSProperties
            }
            className="flex min-h-0 min-w-full basis-full shrink-0 snap-center flex-col overflow-hidden border-y border-edge bg-panel [scroll-snap-stop:always] sm:min-w-[17.5rem] sm:basis-0 sm:shrink sm:grow-[var(--panel-grow)] sm:rounded-sm sm:border"
            aria-label={`panel ${column + 1} of ${columns}`}
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
          </section>
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
        <section
          aria-label="swipe to create a new panel"
          className="flex min-h-0 min-w-full basis-full shrink-0 snap-center items-center justify-center border-y border-dashed border-edge bg-panel/60 px-6 text-center [scroll-snap-stop:always] sm:hidden"
        >
          <div className="flex flex-col items-center gap-2 font-mono lowercase text-ink-faint">
            <span className="flex size-12 items-center justify-center rounded-sm bg-surface">
              <Plus className="size-5 shrink-0" aria-hidden />
            </span>
            <span className="text-base">new panel</span>
            <span className="text-base text-ink-ghost">
              keep swiping to add it
            </span>
          </div>
        </section>
      ) : null}

      {columns < MAX_COLUMNS ? (
        <>
          <EdgeAddZone
            side="left"
            nudge={edgeNudge}
            onAdd={() => addEdgeColumn('left')}
          />
          <EdgeAddZone
            side="right"
            nudge={edgeNudge}
            onAdd={() => addEdgeColumn('right')}
          />
        </>
      ) : null}
    </section>
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
  // The active conversation's working dir, forwarded live so ext pages
  // (e.g. the shell explorer) can follow the chat's folder in a split.
  const { active } = useConversationsCtx()
  const extId = extPageIdForScreen(screen)
  if (extId !== null) {
    return (
      <ExtPage
        pageId={extId}
        panelSide={panelSide}
        tabId={tabId}
        onRequestClose={onClose}
        onMissing={onExtMissing}
        workingDir={active?.workingDir ?? null}
        conversationId={active?.id ?? null}
      />
    )
  }
  switch (screen) {
    case CHAT_SCREEN:
      // The compact header variant — a tab column is width-constrained the
      // same way the old side dock was, especially in two-column layouts.
      return <ChatPanel density="dock" onRequestClose={onClose} />
    case 'workers':
      return <Workers onRequestClose={onClose} />
    default:
      return <TracesV2 onRequestClose={onClose} />
  }
}

interface HeaderProps {
  workspace: UseWorkspaceTabsReturn
  mobilePanelIndex: number
  onMobilePanelIndexChange: (index: number) => void
  settingsOpen: boolean
  onToggleSettings: () => void
  onOpenShortcuts: () => void
  onOpenPalette: () => void
}

function Header({
  workspace,
  mobilePanelIndex,
  onMobilePanelIndexChange,
  settingsOpen,
  onToggleSettings,
  onOpenShortcuts,
  onOpenPalette,
}: HeaderProps) {
  const { extPageTitles } = useScreenOptions()
  const { createNew } = useConversationsCtx()
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)
  const columns = tabColumns(workspace.activeTab)
  const mobileScreen = workspace.activeTab.screens[mobilePanelIndex] ?? null

  return (
    <>
      <header className="grid h-16 shrink-0 grid-cols-[1fr_auto_1fr] items-center px-3 sm:hidden">
        <button
          type="button"
          onClick={() => setMobileMenuOpen(true)}
          aria-label="open workspace menu"
          className="flex size-12 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
        >
          <Menu className="size-6" aria-hidden />
        </button>

        <div className="flex size-12 items-center justify-center">
          {columns > 1 ? (
            <span
              className="flex items-center gap-1.5"
              role="status"
              aria-label={`panel ${mobilePanelIndex + 1} of ${columns}`}
            >
              {columns <= 7 ? (
                Array.from({ length: columns }, (_, index) => (
                  <span
                    // biome-ignore lint/suspicious/noArrayIndexKey: panel dots are positional
                    key={index}
                    className={cn(
                      'size-1.5 rounded-full',
                      index === mobilePanelIndex
                        ? 'bg-accent'
                        : 'bg-ink-ghost/60',
                    )}
                    aria-hidden
                  />
                ))
              ) : (
                <span className="font-mono text-sm text-ink-faint" aria-hidden>
                  {mobilePanelIndex + 1} / {columns}
                </span>
              )}
            </span>
          ) : null}
        </div>

        <div className="flex items-center justify-end">
          {/* A phone has no ⌘K. Search is the console's way of reaching
              anything, so it gets a first-class affordance rather than a row
              buried in the workspace menu. */}
          <button
            type="button"
            onClick={onOpenPalette}
            aria-label="search the console"
            className="flex size-12 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
          >
            <Search className="size-6" aria-hidden />
          </button>
          <button
            type="button"
            onClick={() => {
              if (mobileScreen === CHAT_SCREEN) createNew()
              else workspace.createTab({ columns: 1 })
            }}
            aria-label={
              mobileScreen === CHAT_SCREEN ? 'new chat' : 'new workspace'
            }
            className="flex size-12 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
          >
            {mobileScreen === CHAT_SCREEN ? (
              <SquarePen className="size-6" aria-hidden />
            ) : (
              <Plus className="size-6" aria-hidden />
            )}
          </button>
        </div>
      </header>

      <MobileWorkspaceMenu
        open={mobileMenuOpen}
        onOpenChange={setMobileMenuOpen}
        workspace={workspace}
        extPageTitles={extPageTitles}
        settingsOpen={settingsOpen}
        onActivate={(tabId) => {
          onMobilePanelIndexChange(0)
          workspace.activateTab(tabId)
        }}
        onToggleSettings={onToggleSettings}
        onOpenShortcuts={onOpenShortcuts}
        onOpenPalette={onOpenPalette}
      />

      <header className="hidden h-14 shrink-0 items-center justify-between gap-3 pr-6 pl-3 sm:flex">
        <div className="flex min-w-0 flex-1 items-center gap-3">
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
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onOpenShortcuts}
            aria-label="keyboard shortcuts (?)"
            title="keyboard shortcuts (?)"
            className="relative flex size-10 items-center justify-center rounded-md border border-transparent bg-transparent font-sans text-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:border-accent focus-visible:outline-none"
          >
            <span aria-hidden>
              <CircleQuestionMark className="size-4 shrink-0" />
            </span>
          </button>
          <button
            type="button"
            onClick={onToggleSettings}
            aria-pressed={settingsOpen}
            aria-label="console settings"
            title="console settings"
            className={cn(
              'relative flex size-10 items-center justify-center rounded-md border font-sans text-sm',
              settingsOpen
                ? 'border-transparent bg-ink text-bg'
                : 'border-transparent bg-transparent text-ink-faint hover:bg-surface-hover hover:text-ink',
            )}
          >
            <SettingsIcon className="size-4 shrink-0" aria-hidden />
          </button>
        </div>
      </header>
    </>
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

/* Generated from the registry, so the overlay cannot drift from what the
   keys actually do, and each chord is spelled for the reader's keyboard. */
function ShortcutsDialog({ open, onOpenChange }: ShortcutsDialogProps) {
  const platform = shortcutPlatform()
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
        {keybindingGroups().map(([group, entries]) => (
          <section key={group} className="mt-4">
            <h3 className="text-[11px] uppercase tracking-[0.18em] text-ink-ghost">
              {group}
            </h3>
            <ul className="mt-1 divide-y divide-rule-2 border-t border-b border-rule-2">
              {entries.map((entry) => (
                <li
                  key={entry.id}
                  className="flex items-center justify-between gap-6 py-2 font-mono text-[12px] text-ink"
                >
                  <span className="text-ink-faint">{entry.title}</span>
                  {/* Alternatives, not one chord: without a separator `tab`
                      and `shift tab` read as a single four-key press. */}
                  <span className="flex shrink-0 items-center gap-2">
                    {resolveBindings(entry.bindings, platform).map(
                      (binding, index) => (
                        <Fragment key={binding}>
                          {index > 0 ? (
                            <span className="text-ink-ghost">or</span>
                          ) : null}
                          <KeyCombo
                            binding={binding}
                            platform={platform}
                            digitRange={entry.digitIndex}
                          />
                        </Fragment>
                      ),
                    )}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </DialogContent>
    </Dialog>
  )
}
