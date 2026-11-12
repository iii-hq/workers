import type * as React from 'react'
import { cn } from './utils'

interface SkeletonProps extends React.HTMLAttributes<HTMLSpanElement> {
  className?: string
}

export function Skeleton({ className, ...props }: SkeletonProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'inline-block rounded-xs bg-surface skeleton-pulse align-middle',
        className,
      )}
      {...props}
    />
  )
}
