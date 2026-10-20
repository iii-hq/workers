import { Columns2, Menu, Plus, Search, SquarePen } from 'lucide-react'
import { useMemo, useState } from 'react'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { ThemeSwitch } from '@/components/ui/ThemeSwitch'
import { Wordmark } from '@/components/ui/Wordmark'
import { MobileWorkspaceMenu } from '@/components/workspace/MobileWorkspaceMenu'
import { TabStrip } from '@/components/workspace/TabStrip'
import { useScreenOptions } from '@/components/workspace/use-screen-options'
import type { ThemePreference } from '@/hooks/use-theme'
import type { UseWorkspaceTabsReturn } from '@/hooks/use-workspace-tabs'
import { useConversationsCtx } from '@/lib/conversations-context'
import { bindingsFor, hoverTitle } from '@/lib/keybindings/registry'
import { tabLimitReachedTitle } from '@/lib/tab-limit'
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
  'inline-flex shrink-0 items-center justify-center rounded-xl text-neutral-500 transition-colors duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-neutral-300/60 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 active:bg-neutral-300 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100 dark:focus-visible:ring-neutral-400 dark:active:bg-neutral-700'

export interface WorkspaceHeaderProps {
  workspace: UseWorkspaceTabsReturn
  /** Close a workspace after the dirty-work guard has had its say. */
  onCloseTab: (id: string) => void
  onOpenScreen: (screen: TabScreen) => void
  mobilePanelIndex: number
  settingsOpen: boolean
  onToggleSettings: () => void
  onOpenPalette: () => void
  /** Add an empty panel on the right of the active workspace. */
  onSplit: () => void
  theme: ThemePreference
  onThemeChange: (theme: ThemePreference) => void
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
  onSplit,
  theme,
  onThemeChange,
}: WorkspaceHeaderProps) {
  const { extPageTitles, screenOptions } = useScreenOptions()
  const screenIcons = useMemo(
    () => new Map(screenOptions.map((option) => [option.value, option.icon])),
    [screenOptions],
  )
  const { createNew } = useConversationsCtx()
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)
  const columns = tabColumns(workspace.activeTab)
  const mobileScreen = workspace.activeTab.screens[mobilePanelIndex] ?? null
  const mobileIsChat = mobileScreen !== null && isChatScreen(mobileScreen)
  const paletteBinding = bindingsFor('palette.toggle')[0] ?? 'Mod+K'
  const paletteTitle = hoverTitle('Search and commands', 'palette.toggle')
  const limitTitle = tabLimitReachedTitle(workspace.tabLimit)
  // The phone's trailing button is "new chat" on a chat panel and "new
  // workspace" elsewhere; only the latter is subject to the tab limit.
  const mobileCreateBlocked = !mobileIsChat && !workspace.canCreateTab

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
            aria-disabled={mobileCreateBlocked || undefined}
            disabled={mobileCreateBlocked}
            title={mobileCreateBlocked ? limitTitle : undefined}
            className={cn(
              GHOST_CONTROL,
              'size-12 disabled:pointer-events-none disabled:opacity-40',
            )}
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
      {/* The bar is the browser's frame: a step darker than the canvas, so
          the active tab (canvas-coloured, flared at its feet) reads as the
          sheet the workspace is drawn on. */}
      <header className="onboarding-menu-bar hidden h-12 shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 bg-neutral-200 px-3 font-inter text-neutral-900 sm:grid dark:bg-neutral-900 dark:text-neutral-50">
        <div className="flex h-full min-w-0 items-end gap-1">
          <span className="flex size-8 shrink-0 items-center justify-center self-center">
            {/* 20% under the 24px mark it replaced: 19.2px. */}
            <Wordmark className="mx-0 h-[1.2rem]" />
          </span>
          <TabStrip
            tabs={workspace.tabs}
            activeTabId={workspace.activeTabId}
            extPageTitles={extPageTitles}
            screenIcons={screenIcons}
            onActivate={workspace.activateTab}
            onClose={onCloseTab}
            onCreate={() => workspace.createTab({ columns: 1 })}
            createDisabled={!workspace.canCreateTab}
            createDisabledTitle={limitTitle}
            onRename={workspace.renameTab}
            onReorder={workspace.reorderTab}
          />
        </div>

        {/* The bar's right-hand cluster: search, then the two actions that
            change the whole workspace rather than one pane — a split and the
            theme. Search keeps a fixed pill and carries its own chord (the
            caps teach ⌘K on sight); it sits right, not centre, so the tab
            strip gets every other pixel of the bar. */}
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={onOpenPalette}
            aria-label={paletteTitle}
            title={paletteTitle}
            /* onboarding-palette: tour anchor (workers/onboarding). Do not remove. */
            className="onboarding-palette inline-flex h-8 w-[16.2rem] max-w-full items-center gap-2 rounded-xl bg-white pr-1.5 pl-3 text-[13px] font-medium text-neutral-500 transition-[background-color,color,transform] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-neutral-50 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 active:scale-[0.98] dark:bg-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-700 dark:hover:text-neutral-100 dark:focus-visible:ring-neutral-400 gap-4"
          >
            <Search className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate text-left">Search</span>
            <KeyCombo
              binding={paletteBinding}
              className="gap-2"
              capClassName="min-w-0 rounded-md bg-neutral-100 px-1 py-px text-[11px] font-medium text-neutral-500 shadow-none ring-1 ring-neutral-200 dark:bg-neutral-900 dark:text-neutral-400 dark:ring-neutral-700"
            />
          </button>
          <button
            type="button"
            onClick={onSplit}
            aria-label={hoverTitle('Split right', 'panel.split')}
            title={hoverTitle('Split right', 'panel.split')}
            className={cn(GHOST_CONTROL, 'size-8')}
          >
            <Columns2 className="size-4 shrink-0" aria-hidden />
          </button>
          <ThemeSwitch value={theme} onChange={onThemeChange} />
        </div>
      </header>
    </>
  )
}
