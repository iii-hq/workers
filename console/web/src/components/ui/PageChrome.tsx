/**
 * Page chrome — THE layout design system every workspace pane follows,
 * console-native screens (chat, traces) and worker-injected pages alike.
 * The whole set is exported through `@iii-dev/console-ui`, so a worker
 * page composes the exact same components (and picks up console-side
 * restyles for free) instead of re-approximating the look per worker.
 *
 * The vocabulary, one surface per role (see index.css's surface ramp):
 *
 *   PageShell    the pane's root column — fills the pane, `bg-panel`
 *   PageHeader   the standard top bar: `bg-panel-raised`, hairline edge
 *                border, icon + title + description, an actions slot,
 *                and the ✕ that closes the hosting pane (`onClose` —
 *                pages get it as `PageRenderProps.onRequestClose`)
 *   PageBody     the row under the header (`side` mirrors it so a
 *                sidebar can hug the pane's outer edge)
 *   PageSidebar  the navigation column — `bg-sidebar`, bounded width
 *   PageMain     the primary workspace — `bg-panel`
 *
 * PageBody separates its children with a 1px `bg-edge` gap instead of
 * per-child borders, so any composition (sidebar left, right, absent)
 * gets the same hairline without border bookkeeping.
 */

import { X } from 'lucide-react'
import type * as React from 'react'
import { cn } from '@/lib/utils'

/** The pane's root column. Fills the pane whether the parent is a flex
    column (native screens) or a block scroller (the ext-page host). */
export function PageShell({
  className,
  ...rest
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        'flex-1 h-full min-h-0 min-w-0 flex flex-col bg-panel',
        className,
      )}
      {...rest}
    />
  )
}

export interface PageHeaderProps {
  /** Identity glyph, rendered at 16px in faint ink (any svg fits). */
  icon?: React.ReactNode
  /** The page's name — console chrome vocabulary: mono, lowercase. */
  title?: React.ReactNode
  /** One short descriptor; truncates before anything else gives. */
  description?: React.ReactNode
  /** Free-form middle content (fills the flexible gap before actions). */
  children?: React.ReactNode
  /** Right-side controls, rendered before the close affordance. */
  actions?: React.ReactNode
  /** Close the hosting pane — renders the standard ✕ when present.
      Worker pages: pass `PageRenderProps.onRequestClose` through. */
  onClose?: () => void
  className?: string
}

/** The standard pane top bar: slightly raised, hairline bottom edge. */
export function PageHeader({
  icon,
  title,
  description,
  children,
  actions,
  onClose,
  className,
}: PageHeaderProps) {
  return (
    <header
      className={cn(
        'flex items-center gap-2.5 h-11 px-3 shrink-0 whitespace-nowrap',
        'bg-panel-raised border-b border-edge',
        className,
      )}
    >
      {icon ? (
        <span
          aria-hidden
          className="shrink-0 flex items-center text-ink-faint [&>svg]:size-4"
        >
          {icon}
        </span>
      ) : null}
      {title ? (
        <span className="shrink-0 font-mono text-[12px] lowercase font-medium text-ink">
          {title}
        </span>
      ) : null}
      {description ? (
        <span className="font-mono text-[11px] text-ink-ghost truncate min-w-0">
          {description}
        </span>
      ) : null}
      <div className="flex-1 min-w-0 flex items-center gap-2.5">{children}</div>
      {actions ? (
        <div className="shrink-0 flex items-center gap-1.5">{actions}</div>
      ) : null}
      {onClose ? (
        <button
          type="button"
          onClick={onClose}
          aria-label="close panel"
          title="close panel"
          className="shrink-0 flex items-center justify-center size-7 rounded-sm text-ink-faint hover:text-ink hover:bg-surface-hover transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
        >
          <X aria-hidden className="size-4" />
        </button>
      ) : null}
    </header>
  )
}

export interface PageBodyProps extends React.HTMLAttributes<HTMLDivElement> {
  /** The pane's side of the tab (`PageRenderProps.panelSide`) — `right`
      mirrors the row so the sidebar hugs the pane's outer edge. */
  side?: 'left' | 'right'
}

/** The row under the header. The 1px gap paints in `bg-edge`, giving a
    hairline between sidebar and main with zero border bookkeeping. */
export function PageBody({ side = 'left', className, ...rest }: PageBodyProps) {
  return (
    <div
      className={cn(
        'flex-1 min-h-0 min-w-0 flex gap-px bg-edge',
        side === 'right' && 'flex-row-reverse',
        className,
      )}
      {...rest}
    />
  )
}

export interface PageSidebarProps extends React.HTMLAttributes<HTMLElement> {
  /** Column width in px (fixed — navigation stays put while main flexes). */
  width?: number
}

/** The navigation column: slightly gray, fixed width, own scroll. */
export function PageSidebar({
  width = 280,
  className,
  style,
  ...rest
}: PageSidebarProps) {
  return (
    <aside
      style={{ width, ...style }}
      className={cn(
        'shrink-0 min-h-0 flex flex-col overflow-hidden bg-sidebar',
        className,
      )}
      {...rest}
    />
  )
}

/** The primary workspace column: white (`bg-panel`), takes what's left. */
export function PageMain({
  className,
  ...rest
}: React.HTMLAttributes<HTMLElement>) {
  return (
    <main
      className={cn(
        'flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden bg-panel',
        className,
      )}
      {...rest}
    />
  )
}
