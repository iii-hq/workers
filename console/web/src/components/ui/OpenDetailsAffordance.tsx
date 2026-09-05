import { ArrowUpRight } from 'lucide-react'
import type * as React from 'react'
import { cn } from '@/lib/utils'

/** Visual action label for a widget whose surrounding control owns the click.
    Children replace the default "Open details" copy when the destination
    is more specific (a sub-agent card opens the child's chat). */
export type OpenDetailsAffordanceProps = React.HTMLAttributes<HTMLSpanElement>

export function OpenDetailsAffordance({
  className,
  children = 'Open details',
  ...props
}: OpenDetailsAffordanceProps) {
  return (
    <span
      className={cn(
        'inline-flex h-10 shrink-0 items-center justify-center gap-2 rounded-md bg-surface py-2 pr-3 pl-2 font-sans text-base font-medium text-ink sm:h-8 sm:py-1.5 sm:pr-2.5 sm:pl-1.5 sm:text-sm',
        className,
      )}
      {...props}
    >
      <ArrowUpRight aria-hidden className="size-4 h-lh shrink-0" />
      {children}
    </span>
  )
}
