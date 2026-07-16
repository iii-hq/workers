import { CircleQuestionMark, SettingsIcon } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { ChatDock } from '@/components/chat/ChatDock'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { Sheet } from '@/components/ui/Sheet'
import { Wordmark } from '@/components/ui/Wordmark'
import { useChatDock } from '@/hooks/use-chat-dock'
import { useHashRoute, type View } from '@/hooks/use-hash-route'
import { useTheme } from '@/hooks/use-theme'
import { type DockSignal, getDockSignal } from '@/lib/chat-activity'
import {
  ConversationsProvider,
  useConversationsCtx,
} from '@/lib/conversations-context'
import { buildViewOptions } from '@/lib/nav-options'
import { cn } from '@/lib/utils'
import { Browser } from '@/pages/Browser'
import { Configuration } from '@/pages/Configuration'
import { Memory } from '@/pages/Memory'
import { TracesV2 } from '@/pages/TracesV2'
import { Workers } from '@/pages/Workers'
import { Worktrees } from '@/pages/Worktrees'

export function App() {
  const [theme, setTheme] = useTheme()
  const [view, setView] = useHashRoute()
  const dock = useChatDock()
  const [shortcutsOpen, setShortcutsOpen] = useState(false)

  /* When the dock transitions from collapsed → expanded, focus the
     composer so the user can start typing immediately. requestAnimationFrame
     waits for the mount/paint cycle; if the editor isn't there (no active
     conversation, etc.) the focus is a no-op. */
  const wasCollapsedRef = useRef(dock.collapsed)
  useEffect(() => {
    const wasCollapsed = wasCollapsedRef.current
    wasCollapsedRef.current = dock.collapsed
    if (!wasCollapsed || dock.collapsed) return
    if (typeof window === 'undefined') return
    const frame = window.requestAnimationFrame(() => {
      const editor = document.querySelector<HTMLElement>('.composer-editor')
      editor?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [dock.collapsed])

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

  const collapseDock = useCallback(() => {
    dock.setCollapsed(true)
  }, [dock])

  return (
    <ConversationsProvider>
      <Sheet>
        <Header
          view={view}
          onViewChange={setView}
          dockCollapsed={dock.collapsed}
          onToggleDock={dock.toggleCollapsed}
          onOpenShortcuts={() => setShortcutsOpen(true)}
        />
        <div className="flex-1 flex min-h-0">
          <ChatDock
            width={dock.width}
            onWidthChange={dock.setWidth}
            collapsed={dock.collapsed}
            onCollapse={collapseDock}
          />
          <div className="flex-1 flex flex-col min-w-0 min-h-0">
            {view === 'configuration' ? (
              <Configuration theme={theme} onThemeChange={setTheme} />
            ) : view === 'workers' ? (
              <Workers />
            ) : view === 'worktrees' ? (
              <Worktrees />
            ) : view === 'browser' ? (
              <Browser />
            ) : view === 'memory' ? (
              <Memory />
            ) : (
              <TracesV2 />
            )}
          </div>
        </div>
        <ShortcutsDialog open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
      </Sheet>
    </ConversationsProvider>
  )
}

interface HeaderProps {
  view: View
  onViewChange: (next: View) => void
  dockCollapsed: boolean
  onToggleDock: () => void
  onOpenShortcuts: () => void
}

function Header({
  view,
  onViewChange,
  dockCollapsed,
  onToggleDock,
  onOpenShortcuts,
}: HeaderProps) {
  // Optional-worker entries appear only while their worker is present; a
  // direct #/worktrees or #/browser hit still lands on that page's install
  // notice.
  const { worktreeAvailable, browserAvailable, memoryAvailable } =
    useConversationsCtx()
  const viewOptions = buildViewOptions(
    worktreeAvailable,
    browserAvailable,
    memoryAvailable,
  )
  const onConsoleSettings = view === 'configuration'
  return (
    <header className="flex items-center justify-between pl-3 pr-6 h-12 border-b border-rule shrink-0">
      <div className="flex items-center gap-3">
        <DockToggle collapsed={dockCollapsed} onToggle={onToggleDock} />
        <Wordmark />
        <span className="font-mono text-[11px] mb-[-2px] leading-[14px] uppercase tracking-[0.16em] text-ink-faint font-semibold">
          {view}
        </span>
      </div>
      <div className="flex items-center gap-3">
        <ModeToggle<View>
          value={view}
          onChange={onViewChange}
          options={viewOptions}
        />
        <button
          type="button"
          onClick={onOpenShortcuts}
          aria-label="keyboard shortcuts (?)"
          title="keyboard shortcuts (?)"
          className="font-mono text-[14px] leading-none w-8 h-8 flex items-center justify-center border bg-transparent text-ink-faint border-rule hover:text-ink hover:border-ink transition-colors focus-visible:border-accent focus-visible:outline-none"
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
            'font-mono text-[14px] leading-none w-8 h-8 flex items-center justify-center border transition-colors',
            onConsoleSettings
              ? 'bg-ink text-bg border-ink'
              : 'bg-transparent text-ink-faint border-rule hover:text-ink hover:border-ink',
          )}
        >
          <SettingsIcon className="w-4 h-4" />
        </button>
      </div>
    </header>
  )
}

interface DockToggleProps {
  collapsed: boolean
  onToggle: () => void
}

/**
 * Global chat-dock toggle, pinned to the leftmost slot of the app header.
 * One typographic glyph (`>_`), one place: the button state communicates
 * open/closed via the same pressed-vs-outlined vocabulary the configuration
 * gear uses on the opposite end of the header. The `?` button between them
 * documents this binding (and every other) — no inline kbd hint needed.
 *
 * The `_` of the `>_` glyph blinks at terminal cadence so the toggle reads
 * as an AI prompt waiting for input. Suppressed during `active` so the
 * button never carries two concurrent motion sources (the square accent
 * ring is the focal motion; a flickering cursor underneath would be noise).
 *
 * Collapsed state is dimensional, not binary: `getDockSignal()` resolves
 * the active conversation into one of four states. `active` (streaming,
 * pending approval, running tool call) pulses accent so blocking events
 * don't get buried. `attention` and `error` (system message tones) tint
 * the border statically — motion is the wrong gesture for "the engine
 * reported a problem." Clicking the toggle in any signal state expands
 * the dock, which auto-scrolls to the latest message; the user lands on
 * the warn / error content without hunting.
 */
function DockToggle({ collapsed, onToggle }: DockToggleProps) {
  const { active } = useConversationsCtx()
  const signal = getDockSignal(active)
  const expanded = !collapsed
  const baseLabel = collapsed ? 'open chat dock' : 'collapse chat dock'
  const signalPreview = useDockSignalPreview(signal, active)
  /* When the toggle border carries a warn/error tint, surface the actual
     system-message text in `title` + `aria-label` so the user can triage
     without expanding the dock and hunting. Idle/active states get the
     plain label. */
  const fullLabel = signalPreview
    ? `${baseLabel} (⌘\\) — ${signalPreview}`
    : `${baseLabel} (⌘\\)`
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label={fullLabel}
      aria-expanded={expanded}
      aria-controls="chat-dock"
      title={fullLabel}
      className={cn(
        'font-mono text-[14px] leading-none w-7 h-7 flex items-center justify-center border transition-colors focus-visible:border-accent focus-visible:outline-none',
        expanded ? 'bg-ink text-bg border-ink' : dockToggleSignalClass(signal),
      )}
    >
      {/* Single optical mark: `>` plus a `_` cursor pulled snug underneath
          via negative letter-spacing so the cluster sits in roughly the
          same visual footprint as `⚙` and `?` next to it in the header. */}
      <span
        aria-hidden
        className="inline-flex items-baseline"
        style={{ letterSpacing: '-0.18em' }}
      >
        {'>'}
        <span className={signal === 'active' ? undefined : 'blink'}>_</span>
      </span>
    </button>
  )
}

const PREVIEW_MAX_LENGTH = 80

/**
 * Returns a truncated preview of the latest warn/error system message in
 * the active conversation, or `null` when the dock signal doesn't warrant
 * one. Surfaced through the toggle's `title` and `aria-label` so the user
 * gets inline triage from the chrome itself.
 */
function useDockSignalPreview(
  signal: DockSignal,
  active: ReturnType<typeof useConversationsCtx>['active'],
): string | null {
  if (signal !== 'error' && signal !== 'attention') return null
  if (!active) return null
  for (let i = active.messages.length - 1; i >= 0; i--) {
    const m = active.messages[i]
    if (m && m.role === 'system' && (m.tone === 'error' || m.tone === 'warn')) {
      const flat = m.content.replace(/\s+/g, ' ').trim()
      return flat.length > PREVIEW_MAX_LENGTH
        ? `${flat.slice(0, PREVIEW_MAX_LENGTH - 1)}…`
        : flat
    }
  }
  return null
}

function dockToggleSignalClass(signal: DockSignal): string {
  switch (signal) {
    case 'active':
      return 'bg-transparent text-accent border-accent pulse-square'
    case 'attention':
      return 'bg-transparent text-warn border-warn'
    case 'error':
      return 'bg-transparent text-alert border-alert'
    default:
      return 'bg-transparent text-ink-faint border-rule hover:text-ink hover:border-ink'
  }
}

interface ShortcutsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

const SHORTCUTS: { combo: string; description: string }[] = [
  { combo: '⌘\\', description: 'toggle chat dock' },
  { combo: 'Esc', description: 'collapse chat dock when focused inside' },
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
