/* The title row of a sidebar view (VS Code's view title): the name at the
   left, its actions at the right. Actions are the view's verbs — new file,
   refresh, collapse — as compact icon buttons. */

import type { ReactNode } from 'react'

export function ViewHeader({
  title,
  detail,
  actions,
}: {
  title: string
  /** Faint text beside the title, like the browsed folder name. */
  detail?: ReactNode
  actions?: ReactNode
}) {
  return (
    <div className="shui-view-header">
      <span className="shui-view-title">{title}</span>
      {detail ? <span className="shui-view-detail">{detail}</span> : null}
      <span className="spacer" />
      {actions ? <span className="shui-view-actions">{actions}</span> : null}
    </div>
  )
}
