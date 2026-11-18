import type * as React from 'react'
import { Badge as BaseBadge, badgeVariants } from '@/components/shadcn/badge'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * The console's status tones on the shadcn badge: `default` is the
 * neutral `muted` wash, `ok` / `warn` / `alert` tint with the status colour
 * and `accent` with the primary fill's tint. A tone is never a solid fill.
 */
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
    <BaseBadge
      variant="outline"
      className={cn(
        // Touch-sized on phones, 12px type from `sm` up.
        'h-auto max-w-full rounded-full border-transparent px-2.5 py-1 font-sans text-base sm:text-xs',
        variantTone[variant],
        className,
      )}
      data-badge-variant={variant}
      {...props}
    >
      {children}
    </BaseBadge>
  )
}

export { badgeVariants }
