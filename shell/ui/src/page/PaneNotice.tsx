/* What the main pane shows instead of content when it cannot show
   content: an icon, a title that says what happened in plain words, the
   path it happened to, one line on what that means, and the verbs that
   get the reader out of it. One shape for every such state (a file that
   is gone, a read that failed, a diff that could not load), so they read
   as one page. */

import type { ComponentType, ReactNode } from 'react'

export interface PaneNoticeProps {
  Icon: ComponentType<{ 'aria-hidden'?: boolean; className?: string }>
  title: string
  /** Monospace, ellipsised, the full value in the tooltip. */
  path?: string
  detail?: ReactNode
  /** `Button`s, rendered in a row. */
  actions?: ReactNode
  tone?: 'neutral' | 'warn'
}

export function PaneNotice({ Icon, title, path, detail, actions, tone = 'neutral' }: PaneNoticeProps) {
  return (
    <div className={`shui-pane-notice${tone === 'warn' ? ' warn' : ''}`} role="status">
      <span className="shui-pane-notice-icon" aria-hidden="true">
        <Icon aria-hidden />
      </span>
      <h3 className="shui-pane-notice-title">{title}</h3>
      {path ? (
        <p className="shui-pane-notice-path" title={path}>
          {path}
        </p>
      ) : null}
      {detail ? <p className="shui-pane-notice-detail">{detail}</p> : null}
      {actions ? <div className="shui-pane-notice-actions">{actions}</div> : null}
    </div>
  )
}
