import type { LucideIcon } from 'lucide-react'
import type * as React from 'react'
import { Badge, type BadgeVariant } from '@/components/ui/Badge'
import { cn } from '@/lib/utils'

export type ActivityStatusTone =
  | 'positive'
  | 'neutral'
  | 'accent'
  | 'warning'
  | 'danger'

export type ActivityStatusMotion = 'none' | 'pulse' | 'spin'

const toneClasses: Record<
  ActivityStatusTone,
  { badge: BadgeVariant; dot: string; icon: string }
> = {
  positive: {
    badge: 'ok',
    dot: 'bg-ok',
    icon: 'stroke-ok',
  },
  neutral: {
    badge: 'default',
    dot: 'bg-trigger-running',
    icon: 'stroke-trigger-running',
  },
  accent: {
    badge: 'accent',
    dot: 'bg-accent',
    icon: 'stroke-accent',
  },
  warning: {
    badge: 'warn',
    dot: 'bg-warn',
    icon: 'stroke-warn',
  },
  danger: {
    badge: 'alert',
    dot: 'bg-alert',
    icon: 'stroke-alert',
  },
}

export interface ActivityStatusProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children'> {
  label: string
  detail?: string | null
  icon: LucideIcon
  tone?: ActivityStatusTone
  motion?: ActivityStatusMotion
}

/** Shared live-status treatment used by prominent Console activity widgets. */
export function ActivityStatus({
  label,
  detail,
  icon: Icon,
  tone = 'neutral',
  motion = 'none',
  className,
  ...props
}: ActivityStatusProps) {
  const classes = toneClasses[tone]

  return (
    <div
      className={cn('flex min-w-0 items-start gap-2.5', className)}
      role="status"
      aria-live="polite"
      data-activity-status-tone={tone}
      {...props}
    >
      <Icon
        aria-hidden
        className={cn(
          'size-5 h-lh shrink-0 sm:size-4 mt-1',
          classes.icon,
          motion === 'pulse' && 'animate-pulse motion-reduce:animate-none',
          motion === 'spin' && 'animate-spin motion-reduce:animate-none',
        )}
      />
      <div className="min-w-0">
        <Badge variant={classes.badge}>
          <span className="truncate">{label}</span>
        </Badge>
        {detail ? (
          <div className="flex min-w-0 items-center gap-1.5 pt-1 font-sans text-base text-ink-ghost sm:text-xs">
            <span
              aria-hidden
              className={cn('size-2 shrink-0 rounded-full', classes.dot)}
            />
            <span className="truncate tabular-nums">{detail}</span>
          </div>
        ) : null}
      </div>
    </div>
  )
}
