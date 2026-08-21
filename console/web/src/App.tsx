import { Menu, Plus, Search, SettingsIcon, SquarePen, X } from 'lucide-react'
import {
  type CSSProperties,
  Fragment,
  type MutableRefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { ChatPanel } from '@/components/chat/ChatPanel'
import { PaletteHost, type PaletteWorkspace } from '@/components/PaletteHost'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
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
import {
  enqueuePanelCommand,
  type PendingPanelCommand,
} from '@/components/workspace/panel-command-queue'
import {
  dividerForPanel,
  type PanelMotionDirection,
  panelMotionDirection,
} from '@/components/workspace/panel-motion'
import { TabStrip } from '@/components/workspace/TabStrip'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import {
  hashForExtPage,
  useExtPageRoute,
  useHashRoute,
  type View,
} from '@/hooks/use-hash-route'
import { useKeybindings } from '@/hooks/use-keybindings'
import { useMediaQuery } from '@/hooks/use-media-query'
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
import {
  bindingsFor,
  hoverTitle,
  keybindingGroups,
  resolveBindings,
} from '@/lib/keybindings/registry'
import { registerPageCommands } from '@/lib/page-commands'
import { onPaletteOpenRequest } from '@/lib/palette/open-request'
import {
  focusPane,
  requestPaneFocus,
  takePaneFocusRequest,
} from '@/lib/pane-focus'
import { subscribePanelOpen } from '@/lib/panel-context'
import { loadEdgeAddDiscovered, saveEdgeAddDiscovered } from '@/lib/storage'
import { cn } from '@/lib/utils'
import {
  clearTabDirty,
  dirtyReasonsForPane,
  dirtyReasonsForTab,
  pruneDirty,
  setPaneDirty,
  useDirtyEntries,
} from '@/lib/workspace-guards'
import {
  loadMobilePanelIndexes,
  persistMobilePanelIndexes,
} from '@/lib/workspace-mobile'
import {
  CHAT_SCREEN,
  extPageIdForScreen,
  MAX_COLUMNS,
  MIN_COLUMN_FRACTION,
  screenForView,
  type TabScreen,
  tabColumns,
  tabPaneIds,
  tabSizes,
  type WorkspaceTab,
} from '@/lib/workspace-tabs'
import { Configuration } from '@/pages/Configuration'
import { ExtPage } from '@/pages/Ext'
import { TracesV2 } from '@/pages/TracesV2'
import { Workers } from '@/pages/Workers'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'

function firstPartyPageTitle(screen: TabScreen): string {
  if (screen === CHAT_SCREEN) return 'Chat'
  if (screen === 'workers') return 'Workers'
  if (screen === 'traces') return 'Traces'
  return screen
}

/** Focus the pane of the screen the keyboard asked for, if it is open. */
function focusRequestedPane(tab: WorkspaceTab): void {
  const screen = takePaneFocusRequest(tab.screens)
  if (screen === null) return
  const paneId = tabPaneIds(tab)[tab.screens.indexOf(screen)]
  if (!paneId) return
  const root = document.querySelector<HTMLElement>(
    `[data-workspace-pane-id="${CSS.escape(paneId)}"]`,
  )
  if (root) focusPane(root)
}

/**
 * The `commands` a pane hands its page: registrations are keyed to this pane
 * so their keys fire only while focus is inside it, and every one still
 * registered when the pane unmounts is removed with it.
 */
function usePaneCommandsApi(
  pageId: string,
  pageTitle: string | undefined,
  paneId: string,
): PageCommandsApi {
  const removersRef = useRef(new Set<() => void>())
  const api = useMemo<PageCommandsApi>(
    () => ({
      register(commands) {
        const off = registerPageCommands({
          pageId,
          pageTitle,
          source: 'page',
          paneId,
          commands,
        })
        const remove = () => {
          removersRef.current.delete(remove)
          off()
        }
        removersRef.current.add(remove)
        return remove
      },
    }),
    [pageId, pageTitle, paneId],
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new api means a new pane identity; what the old one registered goes with it.
  useEffect(
    () => () => {
      for (const remove of [...removersRef.current]) remove()
    },
    [api],
  )
  return api
}

function paneIdsByTab(tabs: WorkspaceTab[]): Map<string, Set<string>> {
  return new Map(tabs.map((tab) => [tab.id, new Set(tabPaneIds(tab))]))
}

function hasExplicitHash(): boolean {
  if (typeof window === 'undefined') return false
  const hash = window.location.hash
  return hash !== '' && hash !== '#' && hash !== '#/'
}

interface WorkspacePanelCommands {
  openScreen: (screen: TabScreen) => void
  splitRight: () => void
}

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
  const [paletteQuery, setPaletteQuery] = useState('')
  useEffect(
    () =>
      onPaletteOpenRequest((request) => {
        setPaletteQuery(request.query ?? '')
        setPaletteOpen(true)
      }),
    [],
  )
  const workspace = useWorkspaceTabs()
  const { activeTab, activeTabId } = workspace
  const workspaceRef = useRef(workspace)
  workspaceRef.current = workspace
  // Per-workspace phone panel, derived so a tab switch renders its own panel first.
  const [mobilePanelIndexes, setMobilePanelIndexes] = useState(
    loadMobilePanelIndexes,
  )
  const mobilePanelIndex = mobilePanelIndexes[activeTabId] ?? 0
  const setMobilePanelIndex = useCallback((index: number) => {
    const tabId = workspaceRef.current.activeTabId
    setMobilePanelIndexes((current) => {
      if (current[tabId] === index) return current
      const next = { ...current, [tabId]: index }
      persistMobilePanelIndexes(next)
      return next
    })
  }, [])
  const layoutSignature = JSON.stringify(
    workspace.tabs.map((tab) => [tab.id, tabPaneIds(tab)]),
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: the layout signature is the trigger, not a value read here.
  useEffect(() => {
    const live = new Set(workspaceRef.current.tabs.map((tab) => tab.id))
    setMobilePanelIndexes((current) => {
      const kept = Object.fromEntries(
        Object.entries(current).filter(([id]) => live.has(id)),
      )
      if (Object.keys(kept).length === Object.keys(current).length)
        return current
      persistMobilePanelIndexes(kept)
      return kept
    })
    pruneDirty(live, paneIdsByTab(workspaceRef.current.tabs))
  }, [layoutSignature])

  // Unsaved work: the console's own dialog on close; the native prompt only for reload.
  const dirtyEntries = useDirtyEntries()
  const [discardPrompt, setDiscardPrompt] = useState<{
    title: string
    details: string[]
    proceed: () => void
  } | null>(null)
  const discardPromptRef = useRef(discardPrompt)
  discardPromptRef.current = discardPrompt
  useEffect(() => {
    if (dirtyEntries.length === 0) return
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault()
      event.returnValue = ''
      return ''
    }
    window.addEventListener('beforeunload', onBeforeUnload)
    return () => window.removeEventListener('beforeunload', onBeforeUnload)
  }, [dirtyEntries.length])
  const requestCloseTab = useCallback((id: string) => {
    if (discardPromptRef.current) return
    if (workspaceRef.current.tabs.length <= 1) return
    const reasons = dirtyReasonsForTab(id)
    const proceed = () => {
      clearTabDirty(id)
      workspaceRef.current.closeTab(id)
    }
    if (reasons.length === 0) {
      proceed()
      return
    }
    setDiscardPrompt({
      title: 'Close this workspace?',
      details: reasons,
      proceed,
    })
  }, [])
  const requestClosePane = useCallback(
    (tabId: string, paneId: string, close: () => void) => {
      if (discardPromptRef.current) return
      const reasons = dirtyReasonsForPane(tabId, paneId)
      const proceed = () => {
        setPaneDirty(tabId, paneId, false)
        close()
      }
      if (reasons.length === 0) {
        proceed()
        return
      }
      setDiscardPrompt({
        title: 'Close this panel?',
        details: reasons,
        proceed,
      })
    },
    [],
  )

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
    hasExplicitHash() ? null : hashScreen,
  )
  const panelCommandsRef = useRef<WorkspacePanelCommands | null>(null)
  const openWorkspaceScreen = useCallback((screen: TabScreen) => {
    // The keyboard asked for this screen, so the keyboard lands in it: the
    // panes consume the request once the screen has a pane.
    requestPaneFocus(screen)
    const commands = panelCommandsRef.current
    if (commands) commands.openScreen(screen)
    else workspaceRef.current.openScreen(screen)
    window.requestAnimationFrame(() =>
      focusRequestedPane(workspaceRef.current.activeTab),
    )
  }, [])
  const stepPaneFocus = useCallback((delta: 1 | -1) => {
    const roots = tabPaneIds(workspaceRef.current.activeTab)
      .map((paneId) =>
        document.querySelector<HTMLElement>(
          `[data-workspace-pane-id="${CSS.escape(paneId)}"]`,
        ),
      )
      .filter((root): root is HTMLElement => root !== null)
    if (roots.length === 0) return
    const current = roots.findIndex((root) =>
      root.contains(document.activeElement),
    )
    const next =
      current === -1
        ? delta === 1
          ? 0
          : roots.length - 1
        : (current + delta + roots.length) % roots.length
    focusPane(roots[next])
  }, [])
  const splitWorkspacePanel = useCallback(() => {
    const commands = panelCommandsRef.current
    if (commands) commands.splitRight()
    else {
      const current = workspaceRef.current
      current.addColumn(current.activeTab.id, 'right')
    }
  }, [])
  useEffect(
    () =>
      subscribePanelOpen((event) => {
        openWorkspaceScreen(`ext:${event.pageId}`)
      }),
    [openWorkspaceScreen],
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
  const layoutSource = workspace.layoutSource
  const lastLayoutSourceRef = useRef(layoutSource)
  useEffect(() => {
    if (layoutSource === 'pending') return
    if (lastLayoutSourceRef.current !== layoutSource) {
      lastLayoutSourceRef.current = layoutSource
      if (hasExplicitHash()) lastHashScreenRef.current = null
    }
    if (lastHashScreenRef.current === hashScreen) return
    lastHashScreenRef.current = hashScreen
    // No tab representation (settings overlay, unresolved ext route):
    // the tab strip has nothing to react to — and reacting to the ext
    // transient is what used to conjure duplicate tabs.
    if (hashScreen === null) return
    const ws = workspaceRef.current
    if (ws.activeTab.screens.includes(hashScreen)) return
    // Same placement as an agent's console::workspace::open and a worker's
    // panel-open: reuse the tab already showing it, else place it beside chat
    // in the active tab, else open a fresh chat + screen tab — never a bare
    // single-column tab.
    ws.openScreen(hashScreen)
  }, [hashScreen, layoutSource])

  // ── Tabs → hash ──
  // Activating a tab whose screens don't cover the current hash points the
  // hash at the tab's first routed screen, so page-internal sub-routes and
  // deep links keep working. Chat-only and empty tabs leave the hash alone.
  const prevActiveTabIdRef = useRef<string | null>(null)
  const prevSourceForHashRef = useRef<string>('pending')
  useEffect(() => {
    if (layoutSource === 'pending') {
      prevActiveTabIdRef.current = null
      return
    }
    if (prevSourceForHashRef.current !== layoutSource) {
      prevSourceForHashRef.current = layoutSource
      prevActiveTabIdRef.current = activeTabId
      return
    }
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
  }, [activeTabId, activeTab, hashScreen, setView, layoutSource])

  const paletteWorkspace = useMemo(
    (): PaletteWorkspace => ({
      tabs: workspace.tabs,
      activeTabId: workspace.activeTabId,
      activate: (id) => workspaceRef.current.activateTab(id),
      create: () => workspaceRef.current.createTab({ columns: 1 }),
      close: requestCloseTab,
      step: (delta) => workspaceRef.current.activateAdjacent(delta),
      split: splitWorkspacePanel,
      focusPane: stepPaneFocus,
    }),
    [
      workspace.tabs,
      workspace.activeTabId,
      requestCloseTab,
      splitWorkspacePanel,
      stepPaneFocus,
    ],
  )

  /* Every global shortcut the console has, dispatched from the registry.
     Which keys reach here while the caret is in a field, and which stand
     down, is the registry's call — not this component's. */
  useKeybindings({
    'palette.toggle': () => {
      setPaletteQuery('')
      setPaletteOpen((current) => !current)
    },
    'app.settings': toggleSettings,
    'page.chat': () => openWorkspaceScreen(CHAT_SCREEN),
    'page.workers': () => openWorkspaceScreen('workers'),
    'page.traces': () => openWorkspaceScreen('traces'),
    'workspace.create': () => workspaceRef.current.createTab({ columns: 1 }),
    'workspace.next': () => workspaceRef.current.activateAdjacent(1),
    'workspace.previous': () => workspaceRef.current.activateAdjacent(-1),
    'workspace.close': () => requestCloseTab(workspaceRef.current.activeTabId),
    'panel.split': splitWorkspacePanel,
    'panel.next': () => stepPaneFocus(1),
    'panel.previous': () => stepPaneFocus(-1),
    // Out of range is a no-op rather than a wrap: pressing 7 with four
    // workspaces open should do nothing, not land somewhere surprising.
    'workspace.selectByIndex': (index) => {
      const tab = workspaceRef.current.tabs[index]
      if (tab) workspaceRef.current.activateTab(tab.id)
    },
  })

  return (
    <ConversationsProvider
      injectableUiRuntime={injectableUiRuntime}
      onConversationRequested={() => {
        openWorkspaceScreen(CHAT_SCREEN)
      }}
    >
      <Sheet>
        <Header
          workspace={workspace}
          mobilePanelIndex={mobilePanelIndex}
          onCloseTab={requestCloseTab}
          settingsOpen={view === 'configuration'}
          onToggleSettings={toggleSettings}
          onOpenPalette={() => setPaletteOpen(true)}
        />
        <WorkspacePanes
          workspace={workspace}
          commandsRef={panelCommandsRef}
          mobilePanelIndex={mobilePanelIndex}
          onMobilePanelIndexChange={setMobilePanelIndex}
          onRequestClosePane={requestClosePane}
          onExtMissing={onExtMissing}
        />
        <ConfirmDialog
          open={discardPrompt !== null}
          onOpenChange={(open) => {
            if (!open) setDiscardPrompt(null)
          }}
          title={discardPrompt?.title ?? ''}
          description="Unsaved work in it will be lost."
          details={discardPrompt?.details}
          confirmLabel="Close and discard"
          cancelLabel="Keep working"
          onConfirm={() => discardPrompt?.proceed()}
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
          openScreen={openWorkspaceScreen}
          workspace={paletteWorkspace}
          onOpenSettings={() => setView('configuration')}
          onOpenShortcuts={() => setShortcutsOpen(true)}
          theme={theme}
          onThemeChange={setTheme}
          initialQuery={paletteQuery}
        />
      </Sheet>
    </ConversationsProvider>
  )
}

interface WorkspacePanesProps {
  workspace: UseWorkspaceTabsReturn
  commandsRef: MutableRefObject<WorkspacePanelCommands | null>
  mobilePanelIndex: number
  onMobilePanelIndexChange: (index: number) => void
  /** Close a pane after the dirty-work guard has had its say. */
  onRequestClosePane: (tabId: string, paneId: string, close: () => void) => void
  onExtMissing: () => void
}

interface PanelPresence {
  tabId: string
  paneId: string
  direction: PanelMotionDirection
  token: number
}

interface PendingPanelEntry {
  tabId: string
  column: number
  direction: PanelMotionDirection
}

interface PendingPanelRemoval {
  tabId: string
  paneId: string
}

const DESKTOP_PANEL_MOTION_QUERY = '(min-width: 640px)'
const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'
/** Conservative safety net if CSS animation events are interrupted. */
const PANEL_MOTION_FALLBACK_MS = 400

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
  commandsRef,
  mobilePanelIndex,
  onMobilePanelIndexChange,
  onRequestClosePane,
  onExtMissing,
}: WorkspacePanesProps) {
  const { screenOptions } = useScreenOptions()
  const { activeTab } = workspace
  const columns = tabColumns(activeTab)
  const containerRef = useRef<HTMLElement>(null)
  useEffect(() => {
    const frame = window.requestAnimationFrame(() =>
      focusRequestedPane(activeTab),
    )
    return () => window.cancelAnimationFrame(frame)
  }, [activeTab])
  const desktopPanelMotion = useMediaQuery(DESKTOP_PANEL_MOTION_QUERY)
  const reducedMotion = useMediaQuery(REDUCED_MOTION_QUERY)
  const paneIds = useMemo(() => tabPaneIds(activeTab), [activeTab])
  const layoutSignature = JSON.stringify([activeTab.id, ...paneIds])
  const workspaceSignature = useMemo(
    () => JSON.stringify([workspace.activeTabId, workspace.tabs]),
    [workspace.activeTabId, workspace.tabs],
  )
  const workspaceSignatureRef = useRef(workspaceSignature)
  workspaceSignatureRef.current = workspaceSignature

  const workspaceRef = useRef(workspace)
  workspaceRef.current = workspace
  const pendingPanelEntryRef = useRef<PendingPanelEntry | undefined>(undefined)
  const previousPanelLayoutRef = useRef({
    tabId: activeTab.id,
    paneIds,
    signature: layoutSignature,
  })
  const panelMotionTokenRef = useRef(0)
  const panelEntryCommandAckRef = useRef(false)
  const queuedPanelCommandsRef = useRef<PendingPanelCommand[]>([])
  const panelCommandInFlightRef = useRef<string | null>(null)
  const dispatchPanelCommandRef = useRef<
    ((command: PendingPanelCommand) => void) | null
  >(null)
  const pendingPanelRemovalsRef = useRef<PendingPanelRemoval[]>([])
  const [panelCommandRevision, setPanelCommandRevision] = useState(0)
  const [panelWritePending, setPanelWritePending] = useState(false)
  const [enteringPanel, setEnteringPanel] = useState<PanelPresence | null>(null)
  const [exitingPanel, setExitingPanel] = useState<PanelPresence | null>(null)
  const exitingPanelRef = useRef<PanelPresence | null>(null)

  const setPanelCommandInFlight = useCallback((signature: string | null) => {
    if (panelCommandInFlightRef.current === signature) return
    panelCommandInFlightRef.current = signature
    setPanelWritePending(signature !== null)
    setPanelCommandRevision((revision) => revision + 1)
  }, [])

  // Reconcile only after React commits. A layout effect can mark the new
  // pane before paint, while an interrupted concurrent render cannot consume
  // the pending intent or advance the previous-layout snapshot.
  useLayoutEffect(() => {
    const commandWorkspaceArrived =
      panelCommandInFlightRef.current !== null &&
      panelCommandInFlightRef.current !== workspaceSignature
    const exiting = exitingPanelRef.current
    if (exiting?.tabId === activeTab.id && !paneIds.includes(exiting.paneId)) {
      exitingPanelRef.current = null
      setExitingPanel(null)
    }
    const previousLayout = previousPanelLayoutRef.current
    if (previousLayout.signature === layoutSignature) {
      if (commandWorkspaceArrived) setPanelCommandInFlight(null)
      if (reducedMotion) setEnteringPanel(null)
      return
    }

    const pendingEntry = pendingPanelEntryRef.current
    const addedWithinActiveTab =
      previousLayout.tabId === activeTab.id &&
      paneIds.length === previousLayout.paneIds.length + 1
    const addedPaneIds = addedWithinActiveTab
      ? paneIds.filter((paneId) => !previousLayout.paneIds.includes(paneId))
      : []
    const addedPaneId = addedPaneIds.length === 1 ? addedPaneIds[0] : undefined
    const column = addedPaneId ? paneIds.indexOf(addedPaneId) : -1

    const nextEntry: PanelPresence | null =
      addedPaneId && !reducedMotion && !exitingPanelRef.current
        ? {
            tabId: activeTab.id,
            paneId: addedPaneId,
            direction:
              pendingEntry?.tabId === activeTab.id &&
              pendingEntry.column === column
                ? pendingEntry.direction
                : panelMotionDirection(column, columns),
            token: ++panelMotionTokenRef.current,
          }
        : null
    setEnteringPanel(nextEntry)
    if (commandWorkspaceArrived) {
      if (nextEntry) panelEntryCommandAckRef.current = true
      else setPanelCommandInFlight(null)
    }
    pendingPanelEntryRef.current = undefined
    previousPanelLayoutRef.current = {
      tabId: activeTab.id,
      paneIds,
      signature: layoutSignature,
    }
  }, [
    activeTab.id,
    columns,
    layoutSignature,
    paneIds,
    reducedMotion,
    setPanelCommandInFlight,
    workspaceSignature,
  ])

  useLayoutEffect(() => {
    if (!enteringPanel || !panelEntryCommandAckRef.current) return
    panelEntryCommandAckRef.current = false
    setPanelCommandInFlight(null)
  }, [enteringPanel, setPanelCommandInFlight])

  const clearPanelEntry = useCallback((token: number) => {
    setEnteringPanel((current) => (current?.token === token ? null : current))
  }, [])

  const enteringPanelToken = enteringPanel?.token
  useEffect(() => {
    if (enteringPanelToken === undefined) return
    if (reducedMotion) {
      clearPanelEntry(enteringPanelToken)
      return
    }
    const fallback = window.setTimeout(
      () => clearPanelEntry(enteringPanelToken),
      PANEL_MOTION_FALLBACK_MS,
    )
    return () => window.clearTimeout(fallback)
  }, [clearPanelEntry, enteringPanelToken, reducedMotion])

  const pendingMobileColumnRef = useRef<{
    tabId: string
    index: number
    armed: boolean
  } | null>(null)

  // First-run discoverability for the edge add zones: nudge until the user
  // adds a panel THROUGH a zone (either side), then remember in localStorage
  // so it never plays again. Deliberately not inferred from existing splits —
  // the default workspace already ships a 2-column tab.
  const [edgeNudge, setEdgeNudge] = useState(() => !loadEdgeAddDiscovered())
  const addEdgeColumn = useCallback(
    (side: 'left' | 'right', mobileIndex?: number) =>
      dispatchPanelCommandRef.current?.({
        type: 'add',
        tabId: activeTab.id,
        side,
        mobileIndex,
      }),
    [activeTab.id],
  )

  const addMobileColumn = useCallback(() => {
    if (
      columns >= MAX_COLUMNS ||
      pendingMobileColumnRef.current ||
      enteringPanel ||
      exitingPanelRef.current
    )
      return
    pendingMobileColumnRef.current = {
      tabId: activeTab.id,
      index: columns,
      armed: false,
    }
    addEdgeColumn('right', columns)
  }, [activeTab.id, addEdgeColumn, columns, enteringPanel])

  // A tab switch lands on the panel that tab was last showing; native
  // horizontal scrolling does the gesture work inside a tab.
  useEffect(() => {
    const container = containerRef.current
    if (!container || container.dataset.activeTab === activeTab.id) return
    pendingMobileColumnRef.current = null
    container.dataset.activeTab = activeTab.id
    const index = Math.max(0, Math.min(columns - 1, mobilePanelIndex))
    container.scrollTo({
      left: container.clientWidth * index,
      behavior: 'auto',
    })
  }, [activeTab.id, columns, mobilePanelIndex])

  useEffect(() => {
    const pending = pendingMobileColumnRef.current
    if (
      pending?.armed &&
      pending.tabId === activeTab.id &&
      pending.index < columns
    ) {
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
  const [isResizing, setIsResizing] = useState(false)
  const isResizingRef = useRef(false)
  const startResize = useCallback(() => {
    isResizingRef.current = true
    setIsResizing(true)
  }, [])
  const endResize = useCallback(() => {
    isResizingRef.current = false
    setIsResizing(false)
  }, [])
  useEffect(() => {
    void activeTab.id
    isResizingRef.current = false
    setIsResizing(false)
  }, [activeTab.id])
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
    if (enteringPanel || exitingPanelRef.current) return
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
    if (enteringPanel || exitingPanelRef.current) return
    const next = dragSizesRef.current
    if (!next) return
    if (
      next.length === storedSizes.length &&
      next.every((size, index) => Math.abs(size - storedSizes[index]) < 0.001)
    ) {
      commitPendingRef.current = false
      dragSizesRef.current = null
      setDragSizes(null)
      return
    }
    commitPendingRef.current = true
    setPanelCommandInFlight(workspaceSignatureRef.current)
    workspace.resizeColumns(activeTab.id, next)
  }

  const finalizePanelExit = useCallback(
    (token: number) => {
      const exiting = exitingPanelRef.current
      if (!exiting || exiting.token !== token) return

      exitingPanelRef.current = null
      const current = workspaceRef.current
      const target = current.tabs.find((tab) => tab.id === exiting.tabId)
      if (
        target &&
        tabColumns(target) > 1 &&
        tabPaneIds(target).includes(exiting.paneId)
      ) {
        setPanelCommandInFlight(workspaceSignatureRef.current)
        current.removeColumn(exiting.tabId, exiting.paneId)
      }
      setExitingPanel(null)
    },
    [setPanelCommandInFlight],
  )

  const requestPanelRemoval = useCallback(
    (column: number) => {
      if (columns <= 1 || column < 0 || column >= columns) return
      const paneId = paneIds[column]
      if (!paneId) return

      // Presence motion and optimistic workspace writes are serialized. A
      // second close is queued by pane identity instead of being discarded.
      if (
        enteringPanel ||
        exitingPanelRef.current ||
        panelCommandInFlightRef.current !== null ||
        isResizingRef.current
      ) {
        const alreadyQueued = pendingPanelRemovalsRef.current.some(
          (pending) =>
            pending.tabId === activeTab.id && pending.paneId === paneId,
        )
        if (!alreadyQueued) {
          pendingPanelRemovalsRef.current.push({
            tabId: activeTab.id,
            paneId,
          })
        }
        return
      }

      const panel = containerRef.current?.querySelector<HTMLElement>(
        `[data-workspace-panel="${column}"]`,
      )
      if (panel?.contains(document.activeElement)) {
        const neighborColumn = column < columns - 1 ? column + 1 : column - 1
        containerRef.current
          ?.querySelector<HTMLElement>(
            `[data-workspace-panel="${neighborColumn}"]`,
          )
          ?.focus({ preventScroll: true })
      }

      if (reducedMotion) {
        setPanelCommandInFlight(workspaceSignatureRef.current)
        workspaceRef.current.removeColumn(activeTab.id, paneId)
        return
      }

      const nextExit: PanelPresence = {
        tabId: activeTab.id,
        paneId,
        direction: panelMotionDirection(column, columns),
        token: ++panelMotionTokenRef.current,
      }
      exitingPanelRef.current = nextExit
      setExitingPanel(nextExit)
    },
    [
      activeTab.id,
      columns,
      enteringPanel,
      paneIds,
      reducedMotion,
      setPanelCommandInFlight,
    ],
  )

  useEffect(() => {
    // Background-tab writes can leave the active pane ids unchanged.
    void workspaceSignature
    if (
      enteringPanel ||
      exitingPanelRef.current ||
      panelCommandInFlightRef.current !== null ||
      isResizing
    )
      return
    while (pendingPanelRemovalsRef.current.length > 0) {
      const pending = pendingPanelRemovalsRef.current.shift()
      if (!pending) return

      if (pending.tabId !== activeTab.id) {
        const target = workspaceRef.current.tabs.find(
          (tab) => tab.id === pending.tabId,
        )
        if (
          !target ||
          tabColumns(target) <= 1 ||
          !tabPaneIds(target).includes(pending.paneId)
        )
          continue
        setPanelCommandInFlight(workspaceSignatureRef.current)
        workspaceRef.current.removeColumn(pending.tabId, pending.paneId)
        return
      }
      const column = paneIds.indexOf(pending.paneId)
      if (column < 0 || columns <= 1) continue
      requestPanelRemoval(column)
      return
    }
  }, [
    activeTab.id,
    columns,
    enteringPanel,
    isResizing,
    paneIds,
    requestPanelRemoval,
    setPanelCommandInFlight,
    workspaceSignature,
  ])

  const exitingPanelToken = exitingPanel?.token
  useEffect(() => {
    if (exitingPanelToken === undefined) return
    if (reducedMotion || exitingPanel?.tabId !== activeTab.id) {
      finalizePanelExit(exitingPanelToken)
      return
    }
    const fallback = window.setTimeout(
      () => finalizePanelExit(exitingPanelToken),
      PANEL_MOTION_FALLBACK_MS,
    )
    return () => window.clearTimeout(fallback)
  }, [
    activeTab.id,
    exitingPanel,
    exitingPanelToken,
    finalizePanelExit,
    reducedMotion,
  ])

  const enteringColumn = enteringPanel
    ? paneIds.indexOf(enteringPanel.paneId)
    : -1
  const exitingColumn =
    exitingPanel?.tabId === activeTab.id
      ? paneIds.indexOf(exitingPanel.paneId)
      : -1
  const enteringDivider =
    enteringColumn >= 0 ? dividerForPanel(enteringColumn, columns) : null
  const exitingDivider =
    exitingColumn >= 0 ? dividerForPanel(exitingColumn, columns) : null
  const panelMotionActive = enteringPanel !== null || exitingPanel !== null
  const panelMotionActiveRef = useRef(panelMotionActive)
  panelMotionActiveRef.current = panelMotionActive

  const executePanelCommand = useCallback(
    (command: PendingPanelCommand): boolean => {
      const current = workspaceRef.current
      if (command.type === 'open') {
        const origin = current.tabs.find((tab) => tab.id === command.tabId)
        if (!origin || origin.id === current.activeTab.id) {
          if (current.activeTab.screens.includes(command.screen)) return false
          current.openScreen(command.screen)
          return true
        }
        if (current.tabs.some((tab) => tab.screens.includes(command.screen))) {
          return false
        }
        current.openScreenInTab(origin.id, command.screen)
        return true
      }

      const side = command.type === 'add' ? command.side : 'right'
      const target = current.tabs.find((tab) => tab.id === command.tabId)
      if (!target || tabColumns(target) >= MAX_COLUMNS) {
        if (
          command.type === 'add' &&
          command.mobileIndex !== undefined &&
          pendingMobileColumnRef.current?.tabId === command.tabId &&
          pendingMobileColumnRef.current.index === command.mobileIndex
        ) {
          pendingMobileColumnRef.current = null
        }
        return false
      }

      if (
        command.type === 'add' &&
        command.mobileIndex !== undefined &&
        pendingMobileColumnRef.current?.tabId === command.tabId &&
        pendingMobileColumnRef.current.index === command.mobileIndex
      ) {
        pendingMobileColumnRef.current = {
          tabId: command.tabId,
          index: tabColumns(target),
          armed: true,
        }
      }

      if (current.activeTab.id === command.tabId) {
        if (!reducedMotion) {
          pendingPanelEntryRef.current = {
            tabId: target.id,
            column: side === 'left' ? 0 : tabColumns(target),
            direction: side,
          }
        }
      }
      if (edgeNudge) {
        saveEdgeAddDiscovered()
        setEdgeNudge(false)
      }
      current.addColumn(command.tabId, side)
      return true
    },
    [edgeNudge, reducedMotion],
  )

  const startPanelCommand = useCallback(
    (command: PendingPanelCommand) => {
      setPanelCommandInFlight(workspaceSignatureRef.current)
      if (!executePanelCommand(command)) {
        // A model no-op still releases the gate and wakes the next command.
        setPanelCommandInFlight(null)
      }
    },
    [executePanelCommand, setPanelCommandInFlight],
  )

  const dispatchPanelCommand = useCallback(
    (command: PendingPanelCommand) => {
      if (
        panelMotionActiveRef.current ||
        exitingPanelRef.current ||
        panelCommandInFlightRef.current !== null ||
        isResizingRef.current ||
        pendingPanelRemovalsRef.current.length > 0 ||
        queuedPanelCommandsRef.current.length > 0
      ) {
        if (enqueuePanelCommand(queuedPanelCommandsRef.current, command)) {
          setPanelCommandRevision((revision) => revision + 1)
        }
        return
      }

      startPanelCommand(command)
    },
    [startPanelCommand],
  )

  useLayoutEffect(() => {
    dispatchPanelCommandRef.current = dispatchPanelCommand
    return () => {
      if (dispatchPanelCommandRef.current === dispatchPanelCommand) {
        dispatchPanelCommandRef.current = null
      }
    }
  }, [dispatchPanelCommand])

  useLayoutEffect(() => {
    const commands: WorkspacePanelCommands = {
      openScreen: (screen) =>
        dispatchPanelCommand({ type: 'open', screen, tabId: activeTab.id }),
      splitRight: () =>
        dispatchPanelCommand({ type: 'split', tabId: activeTab.id }),
    }
    commandsRef.current = commands
    return () => {
      if (commandsRef.current === commands) commandsRef.current = null
    }
  }, [activeTab.id, commandsRef, dispatchPanelCommand])

  useEffect(() => {
    // A no-op command still increments this revision so the queue advances.
    void panelCommandRevision
    // A committed workspace snapshot acknowledges the previous command.
    void workspaceSignature
    if (
      panelMotionActive ||
      exitingPanelRef.current ||
      panelCommandInFlightRef.current !== null ||
      isResizing ||
      pendingPanelRemovalsRef.current.length > 0
    )
      return
    const next = queuedPanelCommandsRef.current.shift()
    if (next) startPanelCommand(next)
  }, [
    panelCommandRevision,
    panelMotionActive,
    isResizing,
    startPanelCommand,
    workspaceSignature,
  ])

  const panelCommandQueued = queuedPanelCommandsRef.current.length > 0
  const panelRemovalQueued = pendingPanelRemovalsRef.current.length > 0

  return (
    <section
      ref={containerRef}
      onScroll={(event) => handleMobileScroll(event.currentTarget)}
      className="relative flex min-h-0 flex-1 snap-x snap-mandatory overflow-x-auto overflow-y-hidden pb-0 sm:snap-none sm:px-4 sm:pb-1.5"
      aria-label="workspace panels"
    >
      {Array.from({ length: columns }, (_, column) => {
        const screen = activeTab.screens[column] ?? null
        const paneId = paneIds[column]
        const isExiting =
          exitingPanel?.tabId === activeTab.id && exitingPanel.paneId === paneId
        const isEntering =
          !isExiting &&
          enteringPanel?.tabId === activeTab.id &&
          enteringPanel.paneId === paneId
        const motionDirection = isExiting
          ? exitingPanel.direction
          : isEntering
            ? enteringPanel.direction
            : undefined
        // 'right' only for the rightmost column of a multi-column tab —
        // a full-width single column keeps the default 'left' orientation.
        const panelSide: PanelSide =
          columns > 1 && column === columns - 1 ? 'right' : 'left'
        // The header ✕ on every screen: in a split the column goes; the
        // last column detaches its screen instead (back to the attach
        // affordance) — a tab never loses its final pane.
        const closePane = () =>
          onRequestClosePane(activeTab.id, paneId, () =>
            columns > 1
              ? requestPanelRemoval(column)
              : workspace.detachScreen(activeTab.id, column),
          )
        const pane = (
          <section
            key="panel"
            // ×1000: flex-grow sums below 1 only distribute that fraction
            // of the free space — scaling keeps the ratios AND fills the row.
            style={
              {
                '--panel-grow': sizes[column] * 1000,
              } as CSSProperties
            }
            className={cn(
              'flex min-h-0 min-w-full basis-full shrink-0 snap-center flex-col overflow-hidden [scroll-snap-stop:always]',
              'focus-visible:-outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent',
              'sm:min-w-[17.5rem] sm:basis-0 sm:shrink sm:grow-[var(--panel-grow)]',
              isEntering &&
                (motionDirection === 'left'
                  ? 'workspace-panel-enter-left'
                  : 'workspace-panel-enter-right'),
              isExiting &&
                (motionDirection === 'left'
                  ? 'workspace-panel-exit-left pointer-events-none'
                  : 'workspace-panel-exit-right pointer-events-none'),
            )}
            data-workspace-panel={column}
            data-workspace-pane-id={paneId}
            data-motion-state={
              isEntering ? 'entering' : isExiting ? 'exiting' : 'idle'
            }
            inert={isExiting || undefined}
            aria-hidden={isExiting || undefined}
            aria-label={`panel ${column + 1} of ${columns}`}
            tabIndex={-1}
            onAnimationEnd={(event) => {
              const rootGeometryFinished =
                event.target === event.currentTarget &&
                (event.animationName === 'workspace-panel-expand' ||
                  event.animationName === 'workspace-panel-collapse')
              const mobileSurfaceFinished =
                !desktopPanelMotion &&
                event.target !== event.currentTarget &&
                event.animationName.startsWith('workspace-panel-surface-')
              if (
                isEntering &&
                enteringPanel &&
                (rootGeometryFinished || mobileSurfaceFinished)
              ) {
                clearPanelEntry(enteringPanel.token)
              }
              if (
                isExiting &&
                exitingPanel &&
                (rootGeometryFinished || mobileSurfaceFinished)
              ) {
                finalizePanelExit(exitingPanel.token)
              }
            }}
          >
            <div className="workspace-panel-surface flex min-h-0 min-w-full flex-1 flex-col overflow-hidden border-y border-edge bg-panel sm:min-w-[17.5rem] sm:rounded-sm sm:border">
              {screen === null ? (
                <EmptyPane
                  screenOptions={screenOptions}
                  onAttach={(next) =>
                    workspace.attachScreen(activeTab.id, column, next)
                  }
                  onRemove={
                    columns > 1
                      ? () =>
                          onRequestClosePane(activeTab.id, paneId, () =>
                            requestPanelRemoval(column),
                          )
                      : undefined
                  }
                />
              ) : (
                <ScreenBody
                  key={screen}
                  screen={screen}
                  panelSide={panelSide}
                  tabId={activeTab.id}
                  paneId={paneId}
                  onClose={closePane}
                  onExtMissing={onExtMissing}
                />
              )}
            </div>
          </section>
        )
        return (
          <Fragment key={`${activeTab.id}:${paneId}`}>
            {column > 0 ? (
              <ResizeHandle
                key="divider"
                value={sizes[column - 1] * 100}
                disabled={
                  panelMotionActive ||
                  panelWritePending ||
                  panelCommandQueued ||
                  panelRemovalQueued
                }
                motionState={
                  column === exitingDivider
                    ? 'exiting'
                    : column === enteringDivider
                      ? 'entering'
                      : undefined
                }
                onResize={(delta) => resizePair(column - 1, delta)}
                onCommit={commitResize}
                onResizeStart={startResize}
                onResizeEnd={endResize}
                containerWidth={() =>
                  containerRef.current?.getBoundingClientRect().width ?? 0
                }
              />
            ) : null}
            {pane}
          </Fragment>
        )
      })}

      {columns < MAX_COLUMNS ? (
        <section
          aria-label="swipe to create a new panel"
          className="flex min-h-0 min-w-full basis-full shrink-0 snap-center items-center justify-center border-y border-dashed border-edge bg-panel/60 px-6 text-center [scroll-snap-stop:always] sm:hidden"
        >
          <div className="flex flex-col items-center gap-2 font-sans text-ink-faint">
            <span className="flex size-12 items-center justify-center rounded-sm bg-surface">
              <Plus className="size-5 shrink-0" aria-hidden />
            </span>
            <span className="text-base">New panel</span>
            <span className="text-base text-ink-ghost">
              Keep swiping to add it
            </span>
          </div>
        </section>
      ) : null}

      {columns < MAX_COLUMNS ? (
        <>
          <EdgeAddZone
            side="left"
            disabled={
              panelMotionActive ||
              panelWritePending ||
              panelCommandQueued ||
              panelRemovalQueued ||
              isResizing
            }
            nudge={edgeNudge}
            onAdd={() => addEdgeColumn('left')}
          />
          <EdgeAddZone
            side="right"
            disabled={
              panelMotionActive ||
              panelWritePending ||
              panelCommandQueued ||
              panelRemovalQueued ||
              isResizing
            }
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
  /** Hosting pane id; the key under which the page reports unsaved work. */
  paneId: string
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
  paneId,
  onClose,
  onExtMissing,
}: ScreenBodyProps) {
  // The active conversation's working dir, forwarded live so ext pages
  // (e.g. the shell explorer) can follow the chat's folder in a split.
  const { active } = useConversationsCtx()
  const setDirty = useCallback(
    (dirty: boolean | string) =>
      setPaneDirty(
        tabId,
        paneId,
        dirty === false ? false : dirty === true ? 'Unsaved changes' : dirty,
      ),
    [tabId, paneId],
  )
  const extId = extPageIdForScreen(screen)
  const commands = usePaneCommandsApi(
    extId ?? screen,
    extId === null ? firstPartyPageTitle(screen) : undefined,
    paneId,
  )
  if (extId !== null) {
    return (
      <ExtPage
        pageId={extId}
        panelSide={panelSide}
        tabId={tabId}
        onRequestClose={onClose}
        setDirty={setDirty}
        commands={commands}
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
      return (
        <ChatPanel
          density="dock"
          panelSide={panelSide}
          onRequestClose={onClose}
          commands={commands}
        />
      )
    case 'workers':
      return <Workers onRequestClose={onClose} />
    default:
      return <TracesV2 onRequestClose={onClose} />
  }
}

interface HeaderProps {
  workspace: UseWorkspaceTabsReturn
  /** Close a workspace after the dirty-work guard has had its say. */
  onCloseTab: (id: string) => void
  mobilePanelIndex: number
  settingsOpen: boolean
  onToggleSettings: () => void
  onOpenPalette: () => void
}

function Header({
  workspace,
  onCloseTab,
  mobilePanelIndex,
  settingsOpen,
  onToggleSettings,
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
                      index === mobilePanelIndex ? 'bg-ink' : 'bg-ink-ghost/60',
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
        onActivate={workspace.activateTab}
        onCloseTab={onCloseTab}
        onToggleSettings={onToggleSettings}
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
            onClose={onCloseTab}
            onCreate={() => workspace.createTab({ columns: 1 })}
            onRename={workspace.renameTab}
            onReorder={workspace.reorderTab}
          />
        </div>
        <div className="flex items-center gap-2">
          {/* The key itself is the button: the console is reached through
              the palette, and the cap teaches the chord on sight. */}
          <button
            type="button"
            onClick={onOpenPalette}
            aria-label={hoverTitle('Search and commands', 'palette.toggle')}
            title={hoverTitle('Search and commands', 'palette.toggle')}
            className="relative flex h-10 items-center justify-center rounded-md border border-transparent bg-transparent px-2 text-ink-faint transition-colors hover:bg-surface-hover hover:text-ink focus-visible:border-accent focus-visible:outline-none active:scale-[0.97]"
          >
            <KeyCombo
              binding={bindingsFor('palette.toggle')[0] ?? 'Mod+K'}
              capClassName="min-w-6 px-1.5 py-0.5 text-[0.72rem] text-current"
            />
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
        <DialogTitle className="text-[11px] font-semibold text-ink-faint">
          Keyboard shortcuts
        </DialogTitle>
        <DialogDescription className="mt-1">
          Every row in{' '}
          <KeyCombo binding={bindingsFor('palette.toggle')[0] ?? 'Mod+K'} />{' '}
          shows its key, so this list is a reference, not homework.
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
