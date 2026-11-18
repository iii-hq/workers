import type * as React from 'react'
import { cn } from '@/lib/utils'

interface SkeletonProps extends React.HTMLAttributes<HTMLSpanElement> {
  className?: string
}

/**
 * The shadcn skeleton look (`animate-pulse rounded-md bg-muted`) on an
 * inline `span`, so a placeholder can stand in for a word inside a line of
 * text; `components/shadcn/skeleton` is the block version.
 */
export function Skeleton({ className, ...props }: SkeletonProps) {
  return (
    <span
      aria-hidden
      data-slot="skeleton"
      className={cn(
        'inline-block animate-pulse rounded-md bg-muted align-middle',
        className,
      )}
      {...props}
    />
  )
}
