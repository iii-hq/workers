import type * as React from 'react'
import { cn } from '@/lib/utils'

type BadgeVariant = 'default' | 'warn' | 'alert' | 'accent'

const variantTone: Record<BadgeVariant, string> = {
  default: 'text-ink-faint',
  warn: 'text-warn',
  alert: 'text-alert',
  accent: 'text-accent',
}

interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant
}

export function Badge({
  variant = 'default',
  className,
  children,
  ...props
}: BadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 font-mono text-[11px] font-medium uppercase tracking-[0.06em]',
        variantTone[variant],
        className,
      )}
      {...props}
    >
      {children}
    </span>
  )
}
