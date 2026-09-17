import type { ReactNode } from 'react'
import { Badge } from '@/components/ui/Badge'
import { cn } from '@/lib/utils'

// The card strip and action line are shared with worker UI now.
export { ActionLine, MetaRow } from '@/components/ui/ActivityMetadata'

export function Chip({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center rounded-sm border border-rule-2 bg-paper-2 px-1.5 py-0.5 font-mono text-[11px] text-ink-faint',
        className,
      )}
    >
      {children}
    </span>
  )
}

export function StatusPill({
  label,
  variant = 'default',
}: {
  label: string
  variant?: 'default' | 'warn' | 'alert' | 'accent'
}) {
  return (
    <Badge variant={variant} className="normal-case tracking-normal">
      {label}
    </Badge>
  )
}
