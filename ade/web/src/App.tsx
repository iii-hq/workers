import { Plus } from 'lucide-react'
import {
  type CSSProperties,
  Fragment,
  type MutableRefObject,
  type DragEvent as ReactDragEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { ConnectionNotice } from '@/components/ConnectionNotice'
import { ChatPanel } from '@/components/chat/ChatPanel'
import { EmailPrompt } from '@/components/EmailPrompt'
import { ExtOverlays } from '@/components/ExtOverlays'
import { OnboardingWizardHost } from '@/components/onboarding/OnboardingWizard'
import { PaletteHost, type PaletteWorkspace } from '@/components/PaletteHost'
import { ShortcutsDialog } from '@/components/ShortcutsDialog'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { Sheet } from '@/components/ui/Sheet'
import { EmptyPane } from '@/components/workspace/EmptyPane'
import {
  EdgeAddZone,
  PanelDragHandle,
  ResizeHandle,
} from '@/components/workspace/pane-controls'
import {
  enqueuePanelCommand,
  type PendingPanelCommand,
} from '@/components/workspace/panel-command-queue'
import {
  dividerForPanel,
  type PanelMotionDirection,
  panelMotionDirection,
} from '@/components/workspace/panel-motion'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import { WorkspaceHeader } from '@/components/workspace/WorkspaceHeader'
import {
  hashForSettingsLanding,
  replaceHash,
  useHashRoute,
} from '@/hooks/use-hash-route'
import { useKeybindings } from '@/hooks/use-keybindings'
import { REDUCED_MOTION_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { useTabLimit } from '@/hooks/use-tab-limit'
import {
  type ThemePreference,
  useTheme,
  useThemePreference,
} from '@/hooks/use-theme'
import {
  type UseWorkspaceTabsReturn,
  useWorkspaceTabs,
} from '@/hooks/use-workspace-tabs'
import { requestChatDialog } from '@/lib/chat-requests'
import { armCompletionBell } from '@/lib/completion-bell'
import {
  ConversationsProvider,
  type InjectableUiRuntime,
  useConversationsCtx,
} from '@/lib/conversations-context'
import { registerPageCommands } from '@/lib/page-commands'
import { onPaletteOpenRequest } from '@/lib/palette/open-request'
import {
  focusPane,
  requestPaneFocus,
  takePaneFocusRequest,
} from '@/lib/pane-focus'
import { subscribePanelOpen, subscribeScreenOpen } from '@/lib/panel-context'
import { onShortcutsDialogRequest } from '@/lib/shortcuts-request'
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
  chatSessionScreen,
  deepLinkScreen,
  extPageIdForScreen,
  isChatScreen,
  MAX_COLUMNS,
  MIN_COLUMN_FRACTION,
  type ScreenPlacement,
  sessionIdForChatScreen,
  type TabScreen,
  tabColumns,
  tabPaneIds,
  tabSizes,
  type WorkspaceTab,
} from '@/lib/workspace-tabs'
import { Configuration } from '@/pages/Configuration'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import { ExtPage } from '@/pages/Ext'
import { TracesV2 } from '@/pages/TracesV2'
import { Workers } from '@/pages/Workers'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

function firstPartyPageTitle(screen: TabScreen): string {
  if (isChatScreen(screen)) return 'Chat'
  if (screen === 'workers') return 'Workers'
  if (screen === 'traces') return 'Traces'
  return screen
}

function paneRoot(paneId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(
    `[data-workspace-pane-id="${CSS.escape(paneId)}"]`,
  )
}

/** Focus the pane of the screen the keyboard asked for, if it is open. */
function focusRequestedPane(tab: WorkspaceTab): void {
  const screen = takePaneFocusRequest(tab.screens)
  if (screen === null) return
  const paneId = tabPaneIds(tab)[tab.screens.indexOf(screen)]
  if (!paneId) return
  const root = paneRoot(paneId)
  if (root) focusPane(root)
}

/**
 * The `commands` a pane hands its page: registrations are keyed to this pane
 * so their keys fire only while focus is inside it, and every one still
 * registered when the pane unmounts is removed with it.
 */
export function usePaneCommandsApi(
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

interface WorkspacePanelCommands {
  openScreen: (screen: TabScreen, placement?: ScreenPlacement) => void
  split: (side: 'left' | 'right') => void
}

export function App({
  injectableUiRuntime,
}: {
  injectableUiRuntime?: Promise<InjectableUiRuntime>
}) {
  // The preference (light / dark / system) is what the switch and settings
  // edit; the resolved theme is what surfaces that only know two render in.
  const [themePreference, setThemePreference] = useThemePreference()
  const [theme] = useTheme()
  useEffect(() => armCompletionBell(), [])
  const { setDirty: setSettingsDirty, tryNavigate: trySettingsNavigation } =
    useUnsavedGuard({ guardHashNavigation: true })
  const [view] = useHashRoute()
  const narrowSettings = useMediaQuery('(max-width: 639px)')
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  // The sidebar's help button and any worker page open the same dialog.
  useEffect(() => onShortcutsDialogRequest(() => setShortcutsOpen(true)), [])
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
  const tabLimit = useTabLimit()
  const workspace = useWorkspaceTabs({ tabLimit })
  const { activeTabId } = workspace
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

  const panelCommandsRef = useRef<WorkspacePanelCommands | null>(null)
  const openWorkspaceScreen = useCallback(
    (screen: TabScreen, placement?: ScreenPlacement) => {
      // The keyboard asked for this screen, so the keyboard lands in it: the
      // panes consume the request once the screen has a pane.
      requestPaneFocus(screen)
      const commands = panelCommandsRef.current
      if (commands) commands.openScreen(screen, placement)
      else workspaceRef.current.openScreen(screen, placement)
      window.requestAnimationFrame(() =>
        focusRequestedPane(workspaceRef.current.activeTab),
      )
    },
    [],
  )
  const stepPaneFocus = useCallback((delta: 1 | -1) => {
    const roots = tabPaneIds(workspaceRef.current.activeTab)
      .map(paneRoot)
      .filter((root): root is HTMLElement => root !== null)
    if (roots.length === 0) return
    const current = roots.findIndex((root) =>
      root.contains(document.activeElement),
    )
    const start = delta === 1 ? 0 : roots.length - 1
    const next =
      current === -1 ? start : (current + delta + roots.length) % roots.length
    focusPane(roots[next])
  }, [])
  const splitWorkspacePanel = useCallback((side: 'left' | 'right') => {
    const commands = panelCommandsRef.current
    if (commands) commands.split(side)
    else {
      const current = workspaceRef.current
      current.addColumn(current.activeTab.id, side)
    }
  }, [])
  useEffect(
    () =>
      subscribePanelOpen((event) => {
        openWorkspaceScreen(`ext:${event.pageId}`)
      }),
    [openWorkspaceScreen],
  )
  // A worker page placing any screen from this browser (`host.panels.
  // openScreen`): the same placement as the palette's, in the tab on screen.
  useEffect(
    () =>
      subscribeScreenOpen(({ screen, relativeTo, direction, sizes }) => {
        openWorkspaceScreen(screen, { relativeTo, direction, sizes })
      }),
    [openWorkspaceScreen],
  )
  // Closing settings only drops the `#/configuration` hash: the hash never
  // named the active tab, so the workspace stays exactly where it was.
  const closeSettings = useCallback(() => {
    window.location.replace('#/')
  }, [])
  const requestCloseSettings = useCallback(() => {
    trySettingsNavigation(closeSettings)
  }, [closeSettings, trySettingsNavigation])
  const openSettings = useCallback(() => {
    const targetHash = hashForSettingsLanding(narrowSettings)
    if (window.location.hash !== targetHash) window.location.hash = targetHash
  }, [narrowSettings])
  const toggleSettings = useCallback(() => {
    if (view === 'configuration') requestCloseSettings()
    else openSettings()
  }, [view, openSettings, requestCloseSettings])
  // ── Deep links ──
  // `#/workers` (and `#/chat` / `#/traces-v2`, legacy aliases of traces) are
  // one-shot commands, never state: open the screen with the same placement
  // as an agent's console::workspace::open and a worker's panel-open (reuse
  // the tab showing it, else beside chat, else a fresh tab), then drop the
  // hash so a reload trusts the workspace store — the active tab — instead
  // of replaying the link. Waits for the layout to hydrate so the link lands
  // on the real tabs, not the local copy. `#/traces` and worker pages have
  // no deep link here: `#/traces` and `#/worker/<scope>` boot the standalone
  // shell instead (main.tsx).
  const layoutSource = workspace.layoutSource
  useEffect(() => {
    if (layoutSource === 'pending') return
    const consume = () => {
      const screen = deepLinkScreen(window.location.hash)
      if (screen === null) return
      replaceHash('#/')
      workspaceRef.current.openScreen(screen)
    }
    consume()
    window.addEventListener('hashchange', consume)
    return () => window.removeEventListener('hashchange', consume)
  }, [layoutSource])

  const paletteWorkspace = useMemo(
    (): PaletteWorkspace => ({
      tabs: workspace.tabs,
      activeTabId: workspace.activeTabId,
      activate: (id) => workspaceRef.current.activateTab(id),
      canCreate: workspace.canCreateTab,
      create: () => workspaceRef.current.createTab({ columns: 1 }),
      close: requestCloseTab,
      step: (delta) => workspaceRef.current.activateAdjacent(delta),
      split: splitWorkspacePanel,
      focusPane: stepPaneFocus,
    }),
    [
      workspace.tabs,
      workspace.activeTabId,
      workspace.canCreateTab,
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
    'app.shortcuts': () => setShortcutsOpen((current) => !current),
    // Both open the chat pane first: the dialogs live in it.
    'chat.new': () => {
      openWorkspaceScreen(CHAT_SCREEN)
      requestChatDialog('new-chat')
    },
    'project.add': () => {
      openWorkspaceScreen(CHAT_SCREEN)
      requestChatDialog('add-project')
    },
    'page.chat': () => openWorkspaceScreen(CHAT_SCREEN),
    'page.workers': () => openWorkspaceScreen('workers'),
    'page.traces': () => openWorkspaceScreen('traces'),
    'workspace.create': () => workspaceRef.current.createTab({ columns: 1 }),
    'workspace.next': () => workspaceRef.current.activateAdjacent(1),
    'workspace.previous': () => workspaceRef.current.activateAdjacent(-1),
    'workspace.close': () => requestCloseTab(workspaceRef.current.activeTabId),
    'panel.split': () => splitWorkspacePanel('right'),
    'panel.splitLeft': () => splitWorkspacePanel('left'),
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
      onConversationPanelRequested={(conversationId) => {
        openWorkspaceScreen(chatSessionScreen(conversationId))
      }}
    >
      <Sheet>
        <WorkspaceHeader
          workspace={workspace}
          onOpenScreen={openWorkspaceScreen}
          mobilePanelIndex={mobilePanelIndex}
          onCloseTab={requestCloseTab}
          settingsOpen={view === 'configuration'}
          onToggleSettings={toggleSettings}
          onOpenPalette={() => {
            setPaletteQuery('')
            setPaletteOpen(true)
          }}
          theme={themePreference}
          onThemeChange={setThemePreference}
        />
        <ConnectionNotice />
        <WorkspacePanes
          workspace={workspace}
          commandsRef={panelCommandsRef}
          mobilePanelIndex={mobilePanelIndex}
          onMobilePanelIndexChange={setMobilePanelIndex}
          onRequestClosePane={requestClosePane}
        />
        <ExtOverlays />
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
            theme={themePreference}
            onThemeChange={setThemePreference}
            onDirtyChange={setSettingsDirty}
            tryNavigate={trySettingsNavigation}
            onClose={requestCloseSettings}
          />
        ) : null}
        <ShortcutsDialog open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
        <EmailPrompt />
        <OnboardingWizardHost />
        <PaletteHost
          open={paletteOpen}
          onOpenChange={setPaletteOpen}
          openScreen={openWorkspaceScreen}
          workspace={paletteWorkspace}
          onOpenSettings={openSettings}
          onOpenShortcuts={() => setShortcutsOpen(true)}
          theme={theme}
          onThemeChange={setThemePreference}
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

interface PanelDragState {
  paneId: string
  overPaneId: string
}

const DESKTOP_PANEL_MOTION_QUERY = '(min-width: 640px)'
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
  const pendingMobileScreenRef = useRef<TabScreen | null>(null)

  const focusMobilePanel = useCallback(
    (index: number) => {
      if (window.matchMedia('(min-width: 640px)').matches) return
      onMobilePanelIndexChange(index)
      const container = containerRef.current
      if (container) {
        container.scrollTo({
          left: container.clientWidth * index,
          behavior: 'auto',
        })
      }
    },
    [onMobilePanelIndexChange],
  )

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

  // A tab switch restores the panel that tab was last showing. An `open`
  // command takes precedence and reveals its exact target once committed.
  // Keeping both cases in one layout effect prevents the tab restoration
  // from undoing the requested focus before paint.
  useLayoutEffect(() => {
    const container = containerRef.current
    const tabChanged = container?.dataset.activeTab !== activeTab.id
    if (container && tabChanged) {
      pendingMobileColumnRef.current = null
      container.dataset.activeTab = activeTab.id
    }

    const pendingScreen = pendingMobileScreenRef.current
    const pendingIndex = pendingScreen
      ? activeTab.screens.indexOf(pendingScreen)
      : -1
    if (pendingIndex >= 0) {
      pendingMobileScreenRef.current = null
      focusMobilePanel(pendingIndex)
      return
    }
    if (container && tabChanged) {
      const index = Math.max(0, Math.min(columns - 1, mobilePanelIndex))
      container.scrollTo({
        left: container.clientWidth * index,
        behavior: 'auto',
      })
    }
  }, [
    activeTab.id,
    activeTab.screens,
    columns,
    focusMobilePanel,
    mobilePanelIndex,
  ])

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
  const [panelDrag, setPanelDrag] = useState<PanelDragState | null>(null)
  const panelDragRef = useRef<PanelDragState | null>(null)
  const clearPanelDrag = useCallback(() => {
    panelDragRef.current = null
    setPanelDrag(null)
  }, [])
  const startPanelDrag = useCallback(
    (paneId: string, event: ReactDragEvent<HTMLButtonElement>) => {
      const next = { paneId, overPaneId: paneId }
      panelDragRef.current = next
      setPanelDrag(next)
      event.dataTransfer.effectAllowed = 'move'
      event.dataTransfer.setData('text/plain', paneId)

      const panel = paneRoot(paneId)
      if (!panel) return
      const rect = panel.getBoundingClientRect()
      event.dataTransfer.setDragImage(
        panel,
        Math.max(0, Math.min(rect.width, event.clientX - rect.left)),
        Math.max(0, Math.min(rect.height, event.clientY - rect.top)),
      )
    },
    [],
  )
  const targetPanelDrag = useCallback((paneId: string) => {
    const current = panelDragRef.current
    if (!current || current.overPaneId === paneId) return
    const next = { ...current, overPaneId: paneId }
    panelDragRef.current = next
    setPanelDrag(next)
  }, [])
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
    clearPanelDrag()
    isResizingRef.current = false
    setIsResizing(false)
  }, [activeTab.id, clearPanelDrag])
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
        isResizingRef.current ||
        panelDragRef.current !== null
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
      isResizing ||
      panelDrag !== null
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
    panelDrag,
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
          const existingColumn = current.activeTab.screens.indexOf(
            command.screen,
          )
          if (existingColumn >= 0) {
            focusMobilePanel(existingColumn)
            return false
          }
          pendingMobileScreenRef.current = command.screen
          current.openScreen(command.screen, command.placement)
          return true
        }
        if (current.tabs.some((tab) => tab.screens.includes(command.screen))) {
          return false
        }
        current.openScreenInTab(origin.id, command.screen, command.placement)
        return true
      }

      const side = command.side
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
    [edgeNudge, focusMobilePanel, reducedMotion],
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
        panelDragRef.current !== null ||
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
      openScreen: (screen, placement) =>
        dispatchPanelCommand({
          type: 'open',
          screen,
          tabId: activeTab.id,
          placement,
        }),
      split: (side) =>
        dispatchPanelCommand({ type: 'add', tabId: activeTab.id, side }),
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
      panelDrag !== null ||
      pendingPanelRemovalsRef.current.length > 0
    )
      return
    const next = queuedPanelCommandsRef.current.shift()
    if (next) startPanelCommand(next)
  }, [
    panelCommandRevision,
    panelMotionActive,
    isResizing,
    panelDrag,
    startPanelCommand,
    workspaceSignature,
  ])

  const panelCommandQueued = queuedPanelCommandsRef.current.length > 0
  const panelRemovalQueued = pendingPanelRemovalsRef.current.length > 0
  const panelInteractionDisabled =
    panelMotionActive ||
    panelWritePending ||
    panelCommandQueued ||
    panelRemovalQueued ||
    isResizing

  const requestPanelReorder = useCallback(
    (paneId: string, targetPaneId: string) => {
      if (paneId === targetPaneId) return
      const current = workspaceRef.current
      const tab = current.tabs.find(
        (candidate) => candidate.id === activeTab.id,
      )
      const currentPaneIds = tab ? tabPaneIds(tab) : []
      if (
        !currentPaneIds.includes(paneId) ||
        !currentPaneIds.includes(targetPaneId) ||
        panelMotionActiveRef.current ||
        exitingPanelRef.current ||
        panelCommandInFlightRef.current !== null ||
        isResizingRef.current ||
        pendingPanelRemovalsRef.current.length > 0 ||
        queuedPanelCommandsRef.current.length > 0
      )
        return
      current.reorderPanel(activeTab.id, paneId, targetPaneId)
    },
    [activeTab.id],
  )

  const dropPanel = useCallback(
    (targetPaneId: string) => {
      const sourcePaneId = panelDragRef.current?.paneId
      clearPanelDrag()
      if (sourcePaneId) requestPanelReorder(sourcePaneId, targetPaneId)
    },
    [clearPanelDrag, requestPanelReorder],
  )

  return (
    // The edge add zones are absolutely positioned siblings of the scroller,
    // not children: their nudge/preview transforms push past the scroller's
    // edge and would count toward its scrollWidth, flashing a classic
    // horizontal scrollbar that steals height from every pane (MOT-5202).
    <div className="relative flex min-h-0 flex-1">
      <section
        ref={containerRef}
        onScroll={(event) => handleMobileScroll(event.currentTarget)}
        className="relative flex min-h-0 min-w-0 flex-1 snap-x snap-mandatory overflow-x-auto overflow-y-hidden pb-0 sm:snap-none sm:px-1 sm:pb-1.5"
        aria-label="workspace panels"
      >
        {Array.from({ length: columns }, (_, column) => {
          const screen = activeTab.screens[column] ?? null
          const paneId = paneIds[column]
          const isDragged = panelDrag?.paneId === paneId
          const isDropTarget =
            panelDrag?.overPaneId === paneId && panelDrag.paneId !== paneId
          const isExiting =
            exitingPanel?.tabId === activeTab.id &&
            exitingPanel.paneId === paneId
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
                'group/panel relative flex min-h-0 min-w-full basis-full shrink-0 snap-center flex-col overflow-hidden [scroll-snap-stop:always]',
                'focus-visible:-outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent',
                'sm:min-w-[17.5rem] sm:basis-0 sm:shrink sm:grow-[var(--panel-grow)]',
                isDragged && 'opacity-60',
                isDropTarget &&
                  'z-10 outline-2 -outline-offset-2 outline-accent',
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
              data-panel-drag-state={
                isDragged ? 'dragging' : isDropTarget ? 'target' : undefined
              }
              data-motion-state={
                isEntering ? 'entering' : isExiting ? 'exiting' : 'idle'
              }
              inert={isExiting || undefined}
              aria-hidden={isExiting || undefined}
              aria-label={`panel ${column + 1} of ${columns}`}
              tabIndex={-1}
              onDragOver={(event) => {
                if (!panelDragRef.current) return
                event.preventDefault()
                event.dataTransfer.dropEffect = 'move'
                targetPanelDrag(paneId)
              }}
              onDrop={(event) => {
                if (!panelDragRef.current) return
                event.preventDefault()
                dropPanel(paneId)
              }}
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
              {columns > 1 ? (
                <PanelDragHandle
                  index={column}
                  count={columns}
                  disabled={panelInteractionDisabled}
                  dragging={isDragged}
                  onDragStart={(event) => startPanelDrag(paneId, event)}
                  onDragEnd={clearPanelDrag}
                  onMove={(nextIndex) => {
                    const targetPaneId = paneIds[nextIndex]
                    if (targetPaneId) requestPanelReorder(paneId, targetPaneId)
                  }}
                />
              ) : null}
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
                  disabled={panelInteractionDisabled || panelDrag !== null}
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
            aria-label="swipe to split right"
            className="flex min-h-0 min-w-full basis-full shrink-0 snap-center items-center justify-center border-y border-dashed border-edge bg-panel/60 px-6 text-center [scroll-snap-stop:always] sm:hidden"
          >
            <div className="flex flex-col items-center gap-2 font-sans text-ink-faint">
              <span className="flex size-12 items-center justify-center rounded-sm bg-surface">
                <Plus className="size-5 shrink-0" aria-hidden />
              </span>
              <span className="text-base">Split right</span>
              <span className="text-base text-ink-ghost">
                Keep swiping to add it
              </span>
            </div>
          </section>
        ) : null}
      </section>

      {columns < MAX_COLUMNS ? (
        <>
          <EdgeAddZone
            side="left"
            columns={columns}
            disabled={panelInteractionDisabled || panelDrag !== null}
            nudge={edgeNudge}
            onAdd={() => addEdgeColumn('left')}
          />
          <EdgeAddZone
            side="right"
            columns={columns}
            disabled={panelInteractionDisabled || panelDrag !== null}
            nudge={edgeNudge}
            onAdd={() => addEdgeColumn('right')}
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
  /** Hosting pane id; the key under which the page reports unsaved work. */
  paneId: string
  /** Close this pane — the standard PageHeader ✕ on screens that carry it. */
  onClose: () => void
}

/** One workspace-tab column: the page (or chat view) the screen names.
    Configuration never appears here — it opens as an overlay page. */
function ScreenBody({
  screen,
  panelSide,
  tabId,
  paneId,
  onClose,
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
  if (isChatScreen(screen)) {
    const conversationId = sessionIdForChatScreen(screen)
    // The compact header variant — a tab column is width-constrained the
    // same way the old side dock was, especially in two-column layouts.
    return (
      <ChatPanel
        density="dock"
        panelSide={panelSide}
        onRequestClose={onClose}
        conversationId={conversationId ?? undefined}
        commands={commands}
      />
    )
  }
  if (extId !== null) {
    return (
      <ExtPage
        pageId={extId}
        panelSide={panelSide}
        tabId={tabId}
        paneId={paneId}
        onRequestClose={onClose}
        setDirty={setDirty}
        commands={commands}
        workingDir={active?.workingDir ?? null}
        conversationId={active?.id ?? null}
      />
    )
  }
  if (screen === 'workers')
    return (
      <Workers
        onRequestClose={onClose}
        commands={commands}
        panelSide={panelSide}
        tabId={tabId}
        paneId={paneId}
        setDirty={setDirty}
      />
    )
  return <TracesV2 onRequestClose={onClose} commands={commands} />
}

interface ConfigurationOverlayProps {
  theme: ThemePreference
  onThemeChange: (next: ThemePreference) => void
  onDirtyChange: (dirty: boolean) => void
  tryNavigate: (action: () => void) => boolean
  onClose: () => void
}

/**
 * Console settings in one modal over the workspace — never a tab screen (the
 * tab model rejects it; `screenForView` maps the route to null). The workspace
 * stays mounted underneath, so closing restores the panes exactly as they
 * were. Deep-linkable via `#/configuration` and its worker sub-routes.
 */
export function ConfigurationOverlay({
  theme,
  onThemeChange,
  onDirtyChange,
  tryNavigate,
  onClose,
}: ConfigurationOverlayProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent
        showCloseButton={false}
        initialFocus={() =>
          document.querySelector<HTMLElement>('[data-settings-focus-root]') ??
          true
        }
        className="fixed inset-0 top-0 left-0 flex h-dvh max-h-none w-screen max-w-none translate-x-0 translate-y-0 flex-col gap-0 rounded-none bg-white p-0 ring-0 sm:max-w-none data-open:animate-none data-closed:animate-none dark:bg-black"
      >
        <DialogTitle className="sr-only">Settings</DialogTitle>
        <DialogDescription className="sr-only">
          Configure the ADE and registered workers.
        </DialogDescription>
        <Configuration
          theme={theme}
          onThemeChange={onThemeChange}
          onDirtyChange={onDirtyChange}
          tryNavigate={tryNavigate}
          onClose={onClose}
        />
      </DialogContent>
    </Dialog>
  )
}
