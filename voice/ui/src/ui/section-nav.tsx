/**
 * The page's section list, on the shadcn sidebar menu recipe (and the ADE's
 * own chat sidebar rows): a 36px row, icon then label in Inter 13px, quiet
 * until hovered; the open section takes the row wash and full ink, never a
 * border or a ring. Trailing meta (a count, a live dot) rides at the edge.
 */

import type { LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'

export interface SectionNavItem<Id extends string> {
  id: Id
  label: string
  icon: LucideIcon
  /** A count or a status dot at the row's edge. */
  meta?: ReactNode
}

export function SectionNav<Id extends string>({
  items,
  current,
  onSelect,
  'aria-label': ariaLabel,
}: {
  items: readonly SectionNavItem<Id>[]
  current: Id
  onSelect: (id: Id) => void
  'aria-label': string
}) {
  return (
    <nav className="voice-nav" aria-label={ariaLabel}>
      <ul className="voice-nav-list">
        {items.map((item) => {
          const Icon = item.icon
          const active = item.id === current
          return (
            <li key={item.id}>
              <button
                type="button"
                className="voice-nav-button"
                data-active={active || undefined}
                aria-current={active ? 'page' : undefined}
                onClick={() => onSelect(item.id)}
              >
                <Icon aria-hidden className="voice-nav-icon" />
                <span className="voice-nav-label">{item.label}</span>
                {item.meta ? <span className="voice-nav-meta">{item.meta}</span> : null}
              </button>
            </li>
          )
        })}
      </ul>
    </nav>
  )
}
