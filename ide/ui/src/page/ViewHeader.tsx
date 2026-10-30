/* The title row of a sidebar view, drawn as a section header the way VS
   Code's explorer draws a folder's: a small capitals name at the left,
   the view's verbs (new file, refresh, collapse) as compact icon buttons
   at the right that show while the pointer or focus is on the view.

   With `onToggle` the name is a disclosure: the explorer folds its tree
   under the project name, as the reference sidebar does. The actions stay
   put either way, so a folded section can still be refreshed. */

import { Eyebrow } from '@iii-dev/console-ui'
import { ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'

export function ViewHeader({
  title,
  detail,
  actions,
  collapsed = false,
  onToggle,
}: {
  title: string
  /** Faint text beside the title, like a count. */
  detail?: ReactNode
  actions?: ReactNode
  /** The section is folded (only with `onToggle`). */
  collapsed?: boolean
  /** Present: the title folds and unfolds the section. */
  onToggle?: () => void
}) {
  return (
    <div className="shui-view-header" data-collapsed={collapsed ? '' : undefined}>
      {onToggle ? (
        <button type="button" className="shui-view-toggle" aria-expanded={!collapsed} onClick={onToggle}>
          <ChevronRight aria-hidden className="shui-view-chevron" />
          <Eyebrow size="lg" className="shui-view-title">
            {title}
          </Eyebrow>
        </button>
      ) : (
        <Eyebrow size="lg" className="shui-view-title">
          {title}
        </Eyebrow>
      )}
      {detail ? <span className="shui-view-detail">{detail}</span> : null}
      <span className="spacer" />
      {actions ? <span className="shui-view-actions">{actions}</span> : null}
    </div>
  )
}
