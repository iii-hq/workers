import type * as React from 'react'
import { cn } from '@/lib/utils'

export type BadgeVariant = 'default' | 'ok' | 'warn' | 'alert' | 'accent'

const variantTone: Record<BadgeVariant, string> = {
  default: 'bg-surface text-ink-faint',
  ok: 'bg-ok-muted text-ok',
  warn: 'bg-warn-muted text-warn',
  alert: 'bg-alert-muted text-alert',
  accent: 'bg-accent-muted text-accent',
}

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant
}

/** Compact semantic status label shared by the Console and worker UIs. */
export function Badge({
  variant = 'default',
  className,
  children,
  ...props
}: BadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-1 font-sans text-base font-medium sm:text-xs',
        variantTone[variant],
        className,
      )}
      data-badge-variant={variant}
      {...props}
    >
      {children}
    </span>
  )
}
