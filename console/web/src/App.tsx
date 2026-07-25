import { CircleQuestionMark, SettingsIcon } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
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
  screenForView,
  type TabScreen,
  tabColumns,
} from '@/lib/workspace-tabs'
import { Browser } from '@/pages/Browser'
import { Configuration } from '@/pages/Configuration'
import { ExtPage } from '@/pages/Ext'
import { Github } from '@/pages/Github'
import { Memory } from '@/pages/Memory'
import { TracesV2 } from '@/pages/TracesV2'
import { Workers } from '@/pages/Workers'
import { Worktrees } from '@/pages/Worktrees'

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
  useEffect(() => {
    if (lastHashScreenRef.current === hashScreen) return
    lastHashScreenRef.current = hashScreen
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
    if (activeTab.screens.includes(hashScreen)) return
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
          onViewChange={setView}
          onOpenShortcuts={() => setShortcutsOpen(true)}
        />
        <WorkspacePanes
          workspace={workspace}
          theme={theme}
          onThemeChange={setTheme}
          onExtMissing={onExtMissing}
        />
        <ShortcutsDialog open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
      </Sheet>
    </ConversationsProvider>
  )
}

interface WorkspacePanesProps {
  workspace: UseWorkspaceTabsReturn
  theme: ReturnType<typeof useTheme>[0]
  onThemeChange: (next: ReturnType<typeof useTheme>[0]) => void
  onExtMissing: () => void
}

/**
 * The active tab's columns, each a floating panel over the canvas. An
 * unattached column renders the attach affordance instead of a page.
 * Rendered under `ConversationsProvider` (the screen options need it).
 */
function WorkspacePanes({
  workspace,
  theme,
  onThemeChange,
  onExtMissing,
}: WorkspacePanesProps) {
  const { screenOptions } = useScreenOptions()
  const { activeTab } = workspace
  const columns = tabColumns(activeTab)
  return (
    <div className="flex-1 flex min-h-0 gap-1.5 px-1.5 pb-1.5">
      {Array.from({ length: columns }, (_, column) => {
        const screen = activeTab.screens[column] ?? null
        return (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: the column POSITION is the identity — the composite key deliberately remounts a pane when its tab or attached screen changes
            key={`${activeTab.id}:${column}:${screen ?? 'empty'}`}
            className="flex-1 basis-0 flex flex-col min-w-0 min-h-0 rounded-sm border border-edge bg-panel overflow-hidden"
          >
            {screen === null ? (
              <EmptyPane
                screenOptions={screenOptions}
                onAttach={(next) =>
                  workspace.attachScreen(activeTab.id, column, next)
                }
              />
            ) : (
              <ScreenBody
                screen={screen}
                theme={theme}
                onThemeChange={onThemeChange}
                onExtMissing={onExtMissing}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

interface ScreenBodyProps {
  screen: TabScreen
  theme: ReturnType<typeof useTheme>[0]
  onThemeChange: (next: ReturnType<typeof useTheme>[0]) => void
  onExtMissing: () => void
}

/** One workspace-tab column: the page (or chat view) the screen names. */
function ScreenBody({
  screen,
  theme,
  onThemeChange,
  onExtMissing,
}: ScreenBodyProps) {
  const extId = extPageIdForScreen(screen)
  if (extId !== null) {
    return <ExtPage pageId={extId} onMissing={onExtMissing} />
  }
  switch (screen) {
    case CHAT_SCREEN:
      // The compact header variant — a tab column is width-constrained the
      // same way the old side dock was, especially in two-column layouts.
      return <ChatPanel density="dock" />
    case 'configuration':
      return <Configuration theme={theme} onThemeChange={onThemeChange} />
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
      return <TracesV2 />
  }
}

interface HeaderProps {
  workspace: UseWorkspaceTabsReturn
  onViewChange: (next: View) => void
  onOpenShortcuts: () => void
}

function Header({ workspace, onViewChange, onOpenShortcuts }: HeaderProps) {
  const { extPageTitles } = useScreenOptions()
  const onConsoleSettings =
    workspace.activeTab.screens.includes('configuration')
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
          onCreate={(columns) => workspace.createTab({ columns })}
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
          onClick={() => onViewChange('configuration')}
          aria-pressed={onConsoleSettings}
          aria-label="console settings"
          title="console settings"
          className={cn(
            'font-mono text-[14px] leading-none w-8 h-8 flex items-center justify-center rounded-sm border transition-colors',
            onConsoleSettings
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
