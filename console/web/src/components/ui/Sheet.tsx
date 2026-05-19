import type * as React from 'react'
import { cn } from '@/lib/utils'

interface SheetProps {
  children: React.ReactNode
  className?: string
}

export function Sheet({ children, className }: SheetProps) {
  return (
    <div
      className={cn(
        'w-full min-h-dvh bg-bg flex flex-col h-dvh isolate',
        className,
      )}
    >
      {children}
    </div>
  )
}
