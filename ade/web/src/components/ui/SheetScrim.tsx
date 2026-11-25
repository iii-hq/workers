import type { ComponentProps } from 'react'
import { SheetOverlay } from '@/components/shadcn/sheet'
import { cn } from '@/lib/utils'

/**
 * The dimming layer behind a phone drawer (the context pane's): the sheet's
 * overlay on the console's scrim token rather than its own faint wash.
 */
export function SheetScrim({
  className,
  ...props
}: ComponentProps<typeof SheetOverlay>) {
  return <SheetOverlay className={cn('bg-scrim', className)} {...props} />
}
