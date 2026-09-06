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

/** A collapsible section inside a view (Staged Changes, a folder). */
export function ViewSection({
  title,
  count,
  open,
  onToggle,
  actions,
  children,
}: {
  title: string
  count?: number
  open: boolean
  onToggle: () => void
  actions?: ReactNode
  children?: ReactNode
}) {
  return (
    <section className={`shui-view-section${open ? ' open' : ''}`}>
      <div className="shui-view-section-head">
        <button type="button" className="shui-view-section-toggle" aria-expanded={open} onClick={onToggle}>
          <span className={`chevron${open ? ' open' : ''}`} aria-hidden />
          <span className="shui-view-section-title">{title}</span>
          {count !== undefined ? <span className="shui-view-section-count">{count}</span> : null}
        </button>
        {actions ? <span className="shui-view-actions">{actions}</span> : null}
      </div>
      {open ? <div className="shui-view-section-body">{children}</div> : null}
    </section>
  )
}
