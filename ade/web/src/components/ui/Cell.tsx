import type * as React from 'react'
import { cn } from '@/lib/utils'

interface CellProps {
  title?: React.ReactNode
  children: React.ReactNode
  className?: string
}

export function Cell({ title, children, className }: CellProps) {
  return (
    <div className={cn('rounded-md bg-surface p-5', className)}>
      {title ? (
        <div className="mb-3 font-sans text-[16px] font-semibold tracking-[-0.01em] text-ink">
          {title}
        </div>
      ) : null}
      <div className="max-w-[34ch] font-sans text-[13px] leading-[1.7] text-ink-faint">
        {children}
      </div>
    </div>
  )
}
