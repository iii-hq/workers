import type { ComponentProps } from 'react'
import { Button } from '@/components/shadcn/button'
import { cn } from '@/lib/utils'

/**
 * A quiet text action in a toolbar row (the Add folder picker's Browse and
 * New folder): a small ghost button resting in the muted ink, lifting to
 * full ink on hover. Callers pass layout only.
 */
export function QuietButton({
  className,
  ...props
}: Omit<ComponentProps<typeof Button>, 'variant' | 'size'>) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className={cn(
        'gap-1.5 px-2 text-muted-foreground hover:text-foreground',
        className,
      )}
      {...props}
    />
  )
}
