import { type ReactNode, useRef } from 'react'
import { useVisualViewportFit } from '@/hooks/use-visual-viewport-fit'
import { cn } from '@/lib/utils'

interface SheetProps {
  children: ReactNode
  className?: string
}

/**
 * The app's full-height shell. On a phone it tracks the visual viewport, so
 * a raised keyboard shrinks the column instead of covering its foot.
 */
export function Sheet({ children, className }: SheetProps) {
  const ref = useRef<HTMLDivElement>(null)
  useVisualViewportFit(ref)
  return (
    <div
      ref={ref}
      className={cn(
        'w-full min-h-dvh bg-bg flex flex-col h-dvh overflow-hidden isolate',
        className,
      )}
    >
      {children}
    </div>
  )
}
