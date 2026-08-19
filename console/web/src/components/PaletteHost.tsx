/**
 * Owns ⌘K: the shortcut, the console-side rows, and where each row goes.
 *
 * It lives inside the conversations provider because chats are one of the
 * things you search for; everything else it needs (the workspace, the settings
 * overlay, the theme) arrives as props from `App`.
 */

import { useCallback, useEffect, useMemo, useRef } from 'react'
import { CommandPalette } from '@/components/CommandPalette'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import { useConversationsCtx } from '@/lib/conversations-context'
import type { PaletteEntry } from '@/lib/palette/sources'
import type { TabScreen } from '@/lib/workspace-tabs'
import { setPendingWorkerSearch } from '@/pages/Workers/pending-selection'

export interface PaletteHostProps {
  /** Owned by `App`: the shortcut is one way in, the phone header's search
   *  affordance is the other, and only one of them can live here. */
  open: boolean
  onOpenChange: (open: boolean) => void
  openScreen: (screen: TabScreen) => void
  onOpenSettings: () => void
  onOpenShortcuts: () => void
  theme: 'light' | 'dark'
  onThemeChange: (theme: 'light' | 'dark') => void
}

export function PaletteHost({
  open,
  onOpenChange,
  openScreen,
  onOpenSettings,
  onOpenShortcuts,
  theme,
  onThemeChange,
}: PaletteHostProps) {
  const { screenOptions } = useScreenOptions()
  const { conversations, select, createNew } = useConversationsCtx()

  // Read through a ref so the global listener is bound once, not rebound on
  // every open and close.
  const openRef = useRef(open)
  openRef.current = open

  // ⌘K / ctrl+K anywhere, including from inside the composer — the palette is
  // how you leave wherever you are, so it must not be shadowed by focus.
  useEffect(() => {
    if (typeof window === 'undefined') return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'k' && event.key !== 'K') return
      if (!event.metaKey && !event.ctrlKey) return
      event.preventDefault()
      onOpenChange(!openRef.current)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onOpenChange])

  // Both land on the workers page, filtered to what was picked: a worker by
  // its name, a function by the worker that registers it, falling back to the
  // function id when the engine reported no owner.
  const openWorker = useCallback(
    (name: string) => {
      setPendingWorkerSearch(name)
      openScreen('workers')
    },
    [openScreen],
  )
  const openFunction = useCallback(
    (functionId: string, worker: string) => {
      setPendingWorkerSearch(worker || functionId)
      openScreen('workers')
    },
    [openScreen],
  )

  const localEntries = useMemo((): PaletteEntry[] => {
    const pages: PaletteEntry[] = screenOptions.map((option) => ({
      id: `page:${option.value}`,
      kind: 'page',
      title: option.label,
      detail: option.description,
      keywords: option.keywords,
      icon: option.icon,
      run: () => openScreen(option.value),
    }))

    const chats: PaletteEntry[] = conversations
      .slice(0, 40)
      .map((conversation) => ({
        id: `chat:${conversation.id}`,
        kind: 'chat',
        title: conversation.title || 'untitled chat',
        detail: conversation.model ?? undefined,
        run: () => {
          select(conversation.id)
          openScreen('chat')
        },
      }))

    const actions: PaletteEntry[] = [
      {
        id: 'action:new-chat',
        kind: 'action',
        title: 'New chat',
        detail: 'Start a conversation',
        keywords: ['conversation', 'session'],
        run: () => {
          createNew()
          openScreen('chat')
        },
      },
      {
        id: 'action:settings',
        kind: 'action',
        title: 'Open settings',
        detail: 'Console and worker configuration',
        keywords: ['configuration', 'preferences'],
        run: onOpenSettings,
      },
      {
        id: 'action:shortcuts',
        kind: 'action',
        title: 'Keyboard shortcuts',
        detail: 'Show the shortcut overlay',
        keywords: ['keys', 'help'],
        run: onOpenShortcuts,
      },
      {
        id: 'action:theme',
        kind: 'action',
        title:
          theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme',
        detail: 'Toggle the console theme',
        keywords: ['dark', 'light', 'appearance'],
        run: () => onThemeChange(theme === 'dark' ? 'light' : 'dark'),
      },
    ]

    return [...actions, ...pages, ...chats]
  }, [
    screenOptions,
    conversations,
    select,
    createNew,
    openScreen,
    onOpenSettings,
    onOpenShortcuts,
    theme,
    onThemeChange,
  ])

  return (
    <CommandPalette
      open={open}
      onClose={() => onOpenChange(false)}
      localEntries={localEntries}
      onOpenWorker={openWorker}
      onOpenFunction={openFunction}
    />
  )
}
