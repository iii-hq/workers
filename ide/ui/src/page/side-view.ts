import type { SideView } from './ActivityBar'

export interface SideViewState {
  view: SideView
  collapsed: boolean
}

/**
 * What a press on a sidebar view icon does. `from` is where the icon sits:
 * the strip along the open sidebar's top, or the rail the folded sidebar
 * leaves behind. Pressing the open view again in the strip folds the sidebar
 * (except narrow, where the sidebar is a sheet with its own close).
 *
 * `focusSearch`: the Search view takes the caret into its box, so what the
 * person types next searches. Left on the icon, focus stayed on a button and
 * the first Space pressed it again, folding the sidebar (QA 2026-11-16).
 */
export function selectSideView(
  current: SideViewState,
  view: SideView,
  { from, narrow }: { from: 'strip' | 'rail'; narrow: boolean },
): SideViewState & { focusSearch: boolean } {
  if (from === 'strip' && view === current.view && !narrow && !current.collapsed) {
    return { view, collapsed: true, focusSearch: false }
  }
  return { view, collapsed: false, focusSearch: view === 'search' }
}
