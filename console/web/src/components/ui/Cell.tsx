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
        <div className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink mb-3 lowercase">
          {title}
        </div>
      ) : null}
      <div className="font-mono text-[13px] leading-[1.7] text-ink-faint max-w-[34ch]">
        {children}
      </div>
    </div>
  )
}
