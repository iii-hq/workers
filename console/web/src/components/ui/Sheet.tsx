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
        'mx-auto w-full max-w-[1200px] border-x border-rule min-h-screen bg-bg flex flex-col h-screen',
        className,
      )}
    >
      {children}
    </div>
  )
}
