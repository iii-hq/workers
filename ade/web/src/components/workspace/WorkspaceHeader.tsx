import { Menu, Plus, Search, SettingsIcon, SquarePen } from 'lucide-react'
import { useState } from 'react'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { Wordmark } from '@/components/ui/Wordmark'
import { MobileWorkspaceMenu } from '@/components/workspace/MobileWorkspaceMenu'
import { TabStrip } from '@/components/workspace/TabStrip'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import type { UseWorkspaceTabsReturn } from '@/hooks/use-workspace-tabs'
import { useConversationsCtx } from '@/lib/conversations-context'
import { bindingsFor, hoverTitle } from '@/lib/keybindings/registry'
import { cn } from '@/lib/utils'
import {
  CHAT_SCREEN,
  isChatScreen,
  type TabScreen,
  tabColumns,
} from '@/lib/workspace-tabs'

// viewport: phone chrome — the two sm utilities here swap the phone header
// (menu, panel dots, 48px touch targets) for the desktop menu bar.

/**
 * The header's ghost control: a neutral icon button that only paints on
 * hover, pressed, or keyboard focus. Shared by the phone and desktop bars so
 * every control in the chrome answers the pointer the same way.
 */
const GHOST_CONTROL =
  'inline-flex shrink-0 items-center justify-center rounded-md text-neutral-500 transition-colors duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 active:bg-neutral-200 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100 dark:focus-visible:ring-neutral-400 dark:active:bg-neutral-700'

export interface WorkspaceHeaderProps {
  workspace: UseWorkspaceTabsReturn
  /** Close a workspace after the dirty-work guard has had its say. */
  onCloseTab: (id: string) => void
  onOpenScreen: (screen: TabScreen) => void
  mobilePanelIndex: number
  settingsOpen: boolean
  onToggleSettings: () => void
  onOpenPalette: () => void
}

/**
 * The console's top bar, set in Inter on the neutral palette.
 *
 * Desktop (640px and up) is a 48px menu bar: the wordmark, the workspace
 * tab strip, then a search affordance that spells its own chord and the
 * settings toggle. Phones get a 56px header with the workspace menu, the
 * panel position dots, search and a new-chat/new-workspace action at 48px
 * touch targets; the menu itself is a bottom sheet (`MobileWorkspaceMenu`).
 *
 * The `onboarding-*` classes are tour anchors read by the onboarding
 * worker, never styling; keep them on the same elements.
 */
export function WorkspaceHeader({
  workspace,
  onCloseTab,
  onOpenScreen,
  mobilePanelIndex,
  settingsOpen,
  onToggleSettings,
  onOpenPalette,
}: WorkspaceHeaderProps) {
  const { extPageTitles } = useScreenOptions()
  const { createNew } = useConversationsCtx()
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)
  const columns = tabColumns(workspace.activeTab)
  const mobileScreen = workspace.activeTab.screens[mobilePanelIndex] ?? null
  const mobileIsChat = mobileScreen !== null && isChatScreen(mobileScreen)
  const paletteBinding = bindingsFor('palette.toggle')[0] ?? 'Mod+K'
  const paletteTitle = hoverTitle('Search and commands', 'palette.toggle')

  return (
    <>
      <header className="grid h-14 shrink-0 grid-cols-[1fr_auto_1fr] items-center bg-white px-2 font-inter text-neutral-900 sm:hidden dark:bg-neutral-950 dark:text-neutral-50">
        <div className="flex items-center">
          <button
            type="button"
            onClick={() => setMobileMenuOpen(true)}
            aria-label="open workspace menu"
            className={cn(GHOST_CONTROL, 'size-12')}
          >
            <Menu className="size-5" aria-hidden />
          </button>
        </div>

        <div className="flex h-12 min-w-12 items-center justify-center">
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
                      'size-1.5 rounded-full transition-colors duration-[var(--motion-duration-control)]',
                      index === mobilePanelIndex
                        ? 'bg-neutral-900 dark:bg-neutral-50'
                        : 'bg-neutral-300 dark:bg-neutral-700',
                    )}
                    aria-hidden
                  />
                ))
              ) : (
                <span
                  className="font-mono text-[13px] text-neutral-500 tabular-nums dark:text-neutral-400"
                  aria-hidden
                >
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
            aria-label="search the ADE"
            className={cn(GHOST_CONTROL, 'size-12')}
          >
            <Search className="size-5" aria-hidden />
          </button>
          <button
            type="button"
            onClick={() => {
              if (mobileScreen === CHAT_SCREEN) {
                createNew()
              } else if (mobileIsChat) {
                createNew()
                onOpenScreen(CHAT_SCREEN)
              } else {
                workspace.createTab({ columns: 1 })
              }
            }}
            aria-label={mobileIsChat ? 'new chat' : 'new workspace'}
            className={cn(GHOST_CONTROL, 'size-12')}
          >
            {mobileIsChat ? (
              <SquarePen className="size-5" aria-hidden />
            ) : (
              <Plus className="size-5" aria-hidden />
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

      {/* onboarding-menu-bar: tour anchor (workers/onboarding) — a selector,
          not styling. Do not remove. */}
      <header className="onboarding-menu-bar hidden h-12 shrink-0 items-center gap-4 bg-white pr-3 pl-3 font-inter text-neutral-900 sm:flex dark:bg-neutral-950 dark:text-neutral-50">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <span className="flex size-8 shrink-0 items-center justify-center">
            <Wordmark className="mx-0 h-6" />
          </span>
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

        <div className="flex shrink-0 items-center gap-1.5">
          {/* The console is reached through the palette, so the search
              control carries its own chord: the caps teach ⌘K on sight. */}
          <button
            type="button"
            onClick={onOpenPalette}
            aria-label={paletteTitle}
            title={paletteTitle}
            /* onboarding-palette: tour anchor (workers/onboarding). Do not remove. */
            className="onboarding-palette inline-flex h-8 shrink-0 items-center gap-2 rounded-md bg-neutral-100 pr-1.5 pl-2.5 text-[13px] font-medium text-neutral-500 transition-[background-color,color,transform] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 active:scale-[0.98] dark:bg-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100 dark:focus-visible:ring-neutral-400"
          >
            <Search className="size-4 shrink-0" aria-hidden />
            <span>Search</span>
            <KeyCombo
              binding={paletteBinding}
              className="gap-0.5"
              capClassName="min-w-5 rounded bg-white px-1 py-px text-[11px] font-medium text-neutral-500 shadow-none ring-1 ring-neutral-200 dark:bg-neutral-800 dark:text-neutral-400 dark:ring-neutral-700"
            />
          </button>

          <button
            type="button"
            onClick={onToggleSettings}
            aria-pressed={settingsOpen}
            aria-label="ADE settings"
            title="ADE settings"
            className={cn(
              // onboarding-settings: tour anchor (workers/onboarding). Do not remove.
              'onboarding-settings size-8',
              settingsOpen
                ? 'inline-flex shrink-0 items-center justify-center rounded-md bg-neutral-900 text-neutral-50 transition-colors duration-[var(--motion-duration-control)] hover:bg-neutral-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 dark:focus-visible:ring-neutral-400'
                : GHOST_CONTROL,
            )}
          >
            <SettingsIcon className="size-4 shrink-0" aria-hidden />
          </button>
        </div>
      </header>
    </>
  )
}
