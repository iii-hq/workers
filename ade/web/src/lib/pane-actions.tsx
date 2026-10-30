import { createContext, type ReactNode, useContext } from 'react'
import type { TabScreen } from '@/lib/workspace-tabs'

/**
 * What a workspace pane can do to the split around it, handed to whatever
 * page it hosts. `PageHeader` reads it to draw the pane's split control, so
 * every pane (console screens and worker pages alike) offers the same
 * actions in the same place. Absent outside a desktop workspace pane: the
 * standalone worker route, settings, and phones, where panes are swiped.
 */
export interface PaneActions {
  /** A new column right beside this pane, empty or showing `screen`. */
  split(side: 'left' | 'right', screen?: TabScreen): void
  /** Close every other pane of the tab. Absent while this pane is alone. */
  closeOthers?(): void
  /** Screens the tab already shows: they cannot be opened beside again. */
  openScreens: readonly TabScreen[]
  /** A pane is entering, leaving or being resized: try again in a moment. */
  busy: boolean
}

const PaneActionsContext = createContext<PaneActions | undefined>(undefined)

export function PaneActionsProvider({
  actions,
  children,
}: {
  actions?: PaneActions
  children: ReactNode
}) {
  return (
    <PaneActionsContext.Provider value={actions}>
      {children}
    </PaneActionsContext.Provider>
  )
}

export function usePaneActions(): PaneActions | undefined {
  return useContext(PaneActionsContext)
}
