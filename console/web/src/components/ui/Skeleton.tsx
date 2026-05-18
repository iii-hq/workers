import type * as React from 'react'
import { cn } from '@/lib/utils'

interface SkeletonProps extends React.HTMLAttributes<HTMLSpanElement> {
  className?: string
}

export function Skeleton({ className, ...props }: SkeletonProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'inline-block bg-panel skeleton-pulse align-middle',
        className,
      )}
      {...props}
    />
  )
}
