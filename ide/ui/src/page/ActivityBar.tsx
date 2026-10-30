/* The sidebar's view switcher: one icon per view in a strip along the top
   of the sidebar (the console's header row), the active one on the neutral
   selected fill, counters as small badges. In the collapsed rail the same
   strip stands on end. Changes shows only while a comparison is open (Show
   Diff with Working Tree). Each item is a shadcn Toggle (`aria-pressed`),
   so it wears the console's pressed, hover and focus states; the strip
   only sizes it. `children` is the strip's tail: the page hangs its
   overflow menu there. */

import { Toggle, Tooltip } from '@iii-dev/console-ui'
import { FileDiff, Files, GitBranch, History, Search } from 'lucide-react'
import type { ComponentType, ReactNode } from 'react'

export type SideView = 'files' | 'search' | 'scm' | 'timeline' | 'changes'

interface ViewSpec {
  id: SideView
  label: string
  Icon: ComponentType<{ 'aria-hidden'?: boolean; className?: string }>
  /** The page key that opens the view, shown in the tooltip; '' for none. */
  key: string
}

export const SIDE_VIEWS: readonly ViewSpec[] = [
  { id: 'files', label: 'Explorer', Icon: Files, key: 'E' },
  { id: 'search', label: 'Search', Icon: Search, key: 'F' },
  { id: 'scm', label: 'Source control', Icon: GitBranch, key: 'S' },
  { id: 'timeline', label: 'Timeline', Icon: History, key: 'H' },
  { id: 'changes', label: 'Changes', Icon: FileDiff, key: '' },
]

export function ActivityBar({
  active,
  onSelect,
  badges,
  orientation = 'horizontal',
  changes = false,
  children,
}: {
  active: SideView
  onSelect: (view: SideView) => void
  badges: Partial<Record<SideView, number>>
  /** `horizontal` along the sidebar's top row; `vertical` in the collapsed rail. */
  orientation?: 'horizontal' | 'vertical'
  /** A comparison is open: its view shows. */
  changes?: boolean
  children?: ReactNode
}) {
  return (
    <nav className={`shui-activity-bar ${orientation}`} aria-label="Sidebar views">
      {SIDE_VIEWS.filter(({ id }) => id !== 'changes' || changes).map(({ id, label, Icon, key }) => {
        const count = badges[id]
        const isActive = active === id
        return (
          <Tooltip key={id} label={key === '' ? label : `${label} (${key})`}>
            <Toggle
              className="shui-activity-item"
              aria-label={label}
              pressed={isActive}
              // Pressing the active view again is how the sidebar folds away
              // (the page decides); the strip never holds an unpressed state.
              onPressedChange={() => onSelect(id)}
            >
              <Icon aria-hidden className="shui-activity-icon" />
              {count !== undefined && count > 0 ? (
                <span className="shui-activity-badge" title={`${count} ${label.toLowerCase()} items`}>
                  {count > 99 ? '99+' : count}
                </span>
              ) : null}
            </Toggle>
          </Tooltip>
        )
      })}
      {children}
    </nav>
  )
}
