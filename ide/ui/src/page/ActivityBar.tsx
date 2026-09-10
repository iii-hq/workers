/* The vertical rail on the sidebar's outer edge — VS Code's activity bar:
   one icon per view, the active one marked with an edge rule, counters as
   small badges. It is the page's own control; the console sidebar it sits
   in owns collapse/resize. */

import { FolderTree, GitBranch, History, Search } from 'lucide-react'
import type { ComponentType } from 'react'
import { HoverTip } from './HoverTip'

export type SideView = 'files' | 'search' | 'scm' | 'timeline'

interface ViewSpec {
  id: SideView
  label: string
  Icon: ComponentType<{ 'aria-hidden'?: boolean; className?: string }>
  /** The page key that opens the view, shown in the tooltip. */
  key: string
}

export const SIDE_VIEWS: readonly ViewSpec[] = [
  { id: 'files', label: 'Explorer', Icon: FolderTree, key: 'E' },
  { id: 'search', label: 'Search', Icon: Search, key: 'F' },
  { id: 'scm', label: 'Source control', Icon: GitBranch, key: 'S' },
  { id: 'timeline', label: 'Timeline', Icon: History, key: 'H' },
]

export function ActivityBar({
  active,
  onSelect,
  badges,
  side,
}: {
  active: SideView
  onSelect: (view: SideView) => void
  badges: Partial<Record<SideView, number>>
  side: 'left' | 'right'
}) {
  return (
    <nav className={`shui-activity-bar side-${side}`} aria-label="Sidebar views">
      {SIDE_VIEWS.map(({ id, label, Icon, key }) => {
        const count = badges[id]
        const isActive = active === id
        return (
          <HoverTip key={id} label={`${label} (${key})`}>
            <button
              type="button"
              className={`shui-activity-item${isActive ? ' active' : ''}`}
              aria-label={label}
              aria-pressed={isActive}
              onClick={() => onSelect(id)}
            >
              <Icon aria-hidden className="shui-activity-icon" />
              {count !== undefined && count > 0 ? (
                <span className="shui-activity-badge" title={`${count} ${label.toLowerCase()} items`}>
                  {count > 99 ? '99+' : count}
                </span>
              ) : null}
            </button>
          </HoverTip>
        )
      })}
    </nav>
  )
}
