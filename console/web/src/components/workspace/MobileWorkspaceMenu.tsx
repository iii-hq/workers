import {
  Check,
  CircleQuestionMark,
  PanelsTopLeft,
  Plus,
  Search,
  Settings,
  X,
} from 'lucide-react'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { SheetPage } from '@/components/ui/SheetNavigation'
import type { UseWorkspaceTabsReturn } from '@/hooks/use-workspace-tabs'
import { cn } from '@/lib/utils'
import {
  screenLabel,
  type TabScreen,
  tabColumns,
  tabLabel,
} from '@/lib/workspace-tabs'

interface MobileWorkspaceMenuProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  workspace: UseWorkspaceTabsReturn
  extPageTitles: ReadonlyMap<string, string>
  settingsOpen: boolean
  onActivate: (tabId: string) => void
  onToggleSettings: () => void
  onOpenShortcuts: () => void
  onOpenPalette: () => void
}

export function MobileWorkspaceMenu({
  open,
  onOpenChange,
  workspace,
  extPageTitles,
  settingsOpen,
  onActivate,
  onToggleSettings,
  onOpenShortcuts,
  onOpenPalette,
}: MobileWorkspaceMenuProps) {
  return (
    <BottomSheet open={open} onOpenChange={onOpenChange}>
      <BottomSheetContent className="sm:hidden">
        <SheetPage
          title="Workspace"
          description="Choose a workspace, then swipe between its panels."
          contentClassName="flex flex-col overflow-hidden"
        >
          <div className="min-h-0 flex-1 overflow-y-auto px-3">
            {/* The same search the header opens, repeated here because this
                sheet is where a phone goes looking for everything else. */}
            <button
              type="button"
              onClick={() => {
                onOpenChange(false)
                onOpenPalette()
              }}
              className="mb-3 flex min-h-12 w-full items-center gap-3 rounded-sm bg-surface px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              <Search className="size-5" aria-hidden />
              Search the console
            </button>

            <ul className="space-y-1" aria-label="workspaces">
              {workspace.tabs.map((tab) => {
                const active = tab.id === workspace.activeTabId
                const panels = Array.from(
                  { length: tabColumns(tab) },
                  (_, column) => tab.screens[column] ?? null,
                )

                return (
                  <li
                    key={tab.id}
                    className={cn(
                      'group flex min-h-16 items-stretch rounded-sm',
                      active ? 'bg-surface-selected' : 'hover:bg-surface-hover',
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => {
                        onActivate(tab.id)
                        onOpenChange(false)
                      }}
                      className="flex min-w-0 flex-1 items-center gap-3 px-3 py-2 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus"
                    >
                      <PanelsTopLeft
                        className={cn(
                          'size-5 shrink-0',
                          active ? 'text-ink' : 'text-ink-faint',
                        )}
                        aria-hidden
                      />
                      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                        <span className="truncate font-sans text-base font-medium text-ink">
                          {tabLabel(tab, extPageTitles)}
                        </span>
                        <span className="truncate font-sans text-base text-ink-faint">
                          {panels
                            .map((screen, column) =>
                              screen
                                ? screenLabel(
                                    screen as TabScreen,
                                    extPageTitles,
                                  )
                                : `empty panel ${column + 1}`,
                            )
                            .join(' · ')}
                        </span>
                      </span>
                      {active ? (
                        <Check
                          className="size-5 shrink-0 text-ink"
                          aria-hidden
                        />
                      ) : null}
                    </button>
                    {workspace.tabs.length > 1 ? (
                      <button
                        type="button"
                        onClick={() => workspace.closeTab(tab.id)}
                        aria-label={`close ${tabLabel(tab, extPageTitles)}`}
                        className="flex w-12 shrink-0 items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-active hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus"
                      >
                        <X className="size-4" aria-hidden />
                      </button>
                    ) : null}
                  </li>
                )
              })}
            </ul>

            <button
              type="button"
              onClick={() => {
                workspace.createTab({ columns: 1 })
                onOpenChange(false)
              }}
              className="mt-2 flex min-h-12 w-full items-center gap-3 rounded-sm bg-surface px-3 font-sans text-base font-medium text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              <Plus className="size-5 text-ink-faint" aria-hidden />
              New workspace
            </button>
          </div>

          <div className="mt-3 grid shrink-0 grid-cols-2 gap-2 border-t border-rule-2 px-3 pt-3">
            <button
              type="button"
              onClick={() => {
                onOpenChange(false)
                onOpenShortcuts()
              }}
              className="flex min-h-12 items-center gap-2 rounded-sm px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              <CircleQuestionMark className="size-5" aria-hidden />
              Shortcuts
            </button>
            <button
              type="button"
              aria-pressed={settingsOpen}
              onClick={() => {
                onOpenChange(false)
                onToggleSettings()
              }}
              className={cn(
                'flex min-h-12 items-center gap-2 rounded-sm px-3 font-sans text-base hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
                settingsOpen
                  ? 'bg-surface-selected text-ink'
                  : 'text-ink-faint',
              )}
            >
              <Settings className="size-5" aria-hidden />
              Settings
            </button>
          </div>
        </SheetPage>
      </BottomSheetContent>
    </BottomSheet>
  )
}
