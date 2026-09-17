import type { HTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

export interface BreadcrumbItem {
  /** Authored or machine text; rendered verbatim in mono. */
  label: ReactNode
  /** Navigate to this ancestor. Omit on the current (last) item. */
  onClick?: () => void
  /** Stable key when labels can repeat (e.g. `a/b/a`); defaults to the index. */
  key?: string
}

export interface BreadcrumbProps
  extends Omit<HTMLAttributes<HTMLElement>, 'children'> {
  /** Root first, current location last. */
  items: readonly BreadcrumbItem[]
  /** Glyph between segments; `/` by default. */
  separator?: ReactNode
  /** Emphasise the first item (a bucket, a repository, a workspace root). */
  emphasizeRoot?: boolean
}

const segmentClassName =
  'inline-flex h-5 min-w-0 items-center rounded-md px-1 font-mono text-[11px]'

/**
 * A horizontal path: root › … › current, in the mono voice, inside a
 * `Toolbar` or above a record. Ancestors are ghost buttons that navigate;
 * the last item is the current location (`aria-current="page"`) and is not
 * clickable. Overflow scrolls horizontally with the scrollbar hidden, so a
 * deep path never wraps the toolbar. Selection/emphasis stays neutral
 * (`ink`), never the accent.
 */
export function Breadcrumb({
  items,
  separator = '/',
  emphasizeRoot = true,
  className,
  ...props
}: BreadcrumbProps) {
  return (
    <nav
      aria-label="Breadcrumb"
      className={cn(
        'min-w-0 flex-1 overflow-x-auto whitespace-nowrap text-ink-ghost [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
        className,
      )}
      {...props}
    >
      <ol className="flex items-center gap-1">
        {items.map((item, index) => {
          const isLast = index === items.length - 1
          const isRoot = index === 0
          const emphasis = isRoot && emphasizeRoot
          return (
            <li
              key={item.key ?? index}
              className="inline-flex min-w-0 items-center gap-1"
            >
              {index > 0 ? (
                <span aria-hidden="true" className="text-ink-ghost">
                  {separator}
                </span>
              ) : null}
              {isLast || !item.onClick ? (
                <span
                  aria-current={isLast ? 'page' : undefined}
                  className={cn(
                    segmentClassName,
                    'truncate',
                    emphasis ? 'font-semibold text-ink' : 'text-ink',
                  )}
                >
                  {item.label}
                </span>
              ) : (
                <button
                  type="button"
                  onClick={item.onClick}
                  className={cn(
                    segmentClassName,
                    'truncate cursor-pointer border-0 bg-transparent hover:bg-surface-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-rule-focus focus-visible:-outline-offset-2',
                    emphasis ? 'font-semibold text-ink' : 'text-ink-faint',
                  )}
                >
                  {item.label}
                </button>
              )}
            </li>
          )
        })}
      </ol>
    </nav>
  )
}
