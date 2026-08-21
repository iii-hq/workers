/**
 * The console-side palette rows, and where each one goes.
 *
 * It lives inside the conversations provider because chats are one of the
 * things you search for; everything else it needs (the workspace, the settings
 * overlay, the theme) arrives as props from `App`, which also owns the
 * shortcut that opens it.
 */

import { useCallback, useMemo } from 'react'
import { CommandPalette } from '@/components/CommandPalette'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import { useConversationsCtx } from '@/lib/conversations-context'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  type KeybindingActionId,
  keybinding,
} from '@/lib/keybindings/registry'
import type { PaletteEntry } from '@/lib/palette/sources'
import {
  type TabScreen,
  tabColumns,
  tabLabel,
  type WorkspaceTab,
} from '@/lib/workspace-tabs'
import { setPendingWorkerSearch } from '@/pages/Workers/pending-selection'

/** The workspace actions the palette duplicates from the shortcut registry,
 *  in the order they read. Titles and keywords come from the registry so the
 *  palette, the shortcut overlay and the key itself all say the same thing. */
type WorkspaceActionId = Extract<
  KeybindingActionId,
  | 'workspace.create'
  | 'panel.split'
  | 'workspace.next'
  | 'workspace.previous'
  | 'workspace.close'
>

const WORKSPACE_ACTIONS: ReadonlyArray<{
  id: WorkspaceActionId
  detail: string
}> = [
  { id: 'workspace.create', detail: 'Open an empty workspace' },
  { id: 'panel.split', detail: 'Add a panel beside the current one' },
  { id: 'workspace.next', detail: 'Switch to the workspace on the right' },
  { id: 'workspace.previous', detail: 'Switch to the workspace on the left' },
  { id: 'workspace.close', detail: 'Close the current workspace' },
]

/** First-party pages with a go-to chord; injected pages have no stable letter. */
const PAGE_ACTIONS: Partial<Record<string, KeybindingActionId>> = {
  chat: 'page.chat',
  workers: 'page.workers',
  traces: 'page.traces',
}

export interface PaletteWorkspace {
  tabs: readonly WorkspaceTab[]
  activeTabId: string
  activate: (id: string) => void
  create: () => void
  close: (id: string) => void
  step: (delta: 1 | -1) => void
  split: () => void
}

export interface PaletteHostProps {
  /** Owned by `App`, which dispatches `palette.toggle` from the keybinding
   *  registry and hands the phone header the same opener. */
  open: boolean
  onOpenChange: (open: boolean) => void
  openScreen: (screen: TabScreen) => void
  workspace: PaletteWorkspace
  onOpenSettings: () => void
  onOpenShortcuts: () => void
  theme: 'light' | 'dark'
  onThemeChange: (theme: 'light' | 'dark') => void
}

export function PaletteHost({
  open,
  onOpenChange,
  openScreen,
  workspace,
  onOpenSettings,
  onOpenShortcuts,
  theme,
  onThemeChange,
}: PaletteHostProps) {
  const { screenOptions, extPageTitles } = useScreenOptions()
  const { conversations, select, createNew } = useConversationsCtx()

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
    const platform = shortcutPlatform()
    const pages: PaletteEntry[] = screenOptions.map((option) => {
      const goTo = PAGE_ACTIONS[option.value]
      return {
        id: `page:${option.value}`,
        kind: 'page',
        title: option.label,
        detail: option.description,
        keywords: option.keywords,
        icon: option.icon,
        shortcut: goTo ? bindingsFor(goTo, platform)[0] : undefined,
        run: () => openScreen(option.value),
      }
    })

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

    const digit = bindingsFor('workspace.selectByIndex', platform)[0]
    const workspaces: PaletteEntry[] = workspace.tabs.map((tab, index) => {
      const panels = tabColumns(tab)
      const current = tab.id === workspace.activeTabId
      return {
        id: `workspace:${tab.id}`,
        kind: 'workspace',
        title: tabLabel(tab, extPageTitles),
        detail: current
          ? 'The current workspace'
          : `Switch to this workspace · ${panels} ${panels === 1 ? 'panel' : 'panels'}`,
        keywords: ['workspace', 'tab', 'switch'],
        shortcut: digit && index < 9 ? String(index + 1) : undefined,
        run: () => workspace.activate(tab.id),
      }
    })

    const workspaceRun: Record<WorkspaceActionId, () => void> = {
      'workspace.create': workspace.create,
      'panel.split': workspace.split,
      'workspace.next': () => workspace.step(1),
      'workspace.previous': () => workspace.step(-1),
      'workspace.close': () => workspace.close(workspace.activeTabId),
    }
    const several = workspace.tabs.length > 1
    const workspaceActions: PaletteEntry[] = WORKSPACE_ACTIONS.filter(
      ({ id }) => several || id === 'workspace.create' || id === 'panel.split',
    ).map(({ id, detail }) => {
      const definition = keybinding(id)
      return {
        id: `action:${id}`,
        kind: 'action',
        title: definition.title,
        detail,
        keywords: [...(definition.keywords ?? [])],
        shortcut: bindingsFor(id, platform)[0],
        run: workspaceRun[id],
      }
    })

    const actions: PaletteEntry[] = [
      ...workspaceActions,
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
        shortcut: bindingsFor('app.settings', platform)[0],
        run: onOpenSettings,
      },
      {
        id: 'action:shortcuts',
        kind: 'action',
        title: 'Keyboard shortcuts',
        detail: 'Every key the console listens for, in one list',
        keywords: ['keys', 'help', 'shortcut', 'reference'],
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

    return [...actions, ...workspaces, ...pages, ...chats]
  }, [
    screenOptions,
    extPageTitles,
    workspace,
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
