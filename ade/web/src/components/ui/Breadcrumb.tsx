import { Fragment, type HTMLAttributes, type ReactNode } from 'react'
import {
  Breadcrumb as BaseBreadcrumb,
  BreadcrumbItem as BaseBreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/components/shadcn/breadcrumb'
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
  'inline-flex h-5 min-w-0 items-center truncate rounded-md px-1 font-mono text-meta'

/**
 * A horizontal path on the shadcn breadcrumb: root › … › current, in the
 * mono voice, inside a `Toolbar` or above a record. Ancestors are buttons
 * that navigate; the last item is the current location
 * (`aria-current="page"`) and is not clickable. Overflow scrolls
 * horizontally with the scrollbar hidden, so a deep path never wraps the
 * toolbar.
 */
export function Breadcrumb({
  items,
  separator = '/',
  emphasizeRoot = true,
  className,
  ...props
}: BreadcrumbProps) {
  return (
    <BaseBreadcrumb
      aria-label="Breadcrumb"
      className={cn(
        'min-w-0 flex-1 overflow-x-auto whitespace-nowrap [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
        className,
      )}
      {...props}
    >
      <BreadcrumbList className="flex-nowrap gap-1 text-meta">
        {items.map((item, index) => {
          const isLast = index === items.length - 1
          const emphasis = index === 0 && emphasizeRoot
          // The separator is a list item of its own, beside the segment
          // (an <li> inside the item's <li> is invalid HTML; React logged it).
          return (
            <Fragment key={item.key ?? index}>
              {index > 0 ? (
                <BreadcrumbSeparator className="text-ink-ghost">
                  {separator}
                </BreadcrumbSeparator>
              ) : null}
              <BaseBreadcrumbItem className="min-w-0">
                {isLast || !item.onClick ? (
                  <BreadcrumbPage
                    aria-current={isLast ? 'page' : undefined}
                    className={cn(
                      segmentClassName,
                      'text-foreground',
                      emphasis && 'font-semibold',
                    )}
                  >
                    {item.label}
                  </BreadcrumbPage>
                ) : (
                  <BreadcrumbLink
                    render={<button type="button" onClick={item.onClick} />}
                    className={cn(
                      segmentClassName,
                      'cursor-pointer border-0 bg-transparent hover:bg-muted hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none',
                      emphasis
                        ? 'font-semibold text-foreground'
                        : 'text-muted-foreground',
                    )}
                  >
                    {item.label}
                  </BreadcrumbLink>
                )}
              </BaseBreadcrumbItem>
            </Fragment>
          )
        })}
      </BreadcrumbList>
    </BaseBreadcrumb>
  )
}
