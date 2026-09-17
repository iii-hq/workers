import type * as React from 'react'
import { cn } from '@/lib/utils'

export type StatusVariant = 'info' | 'success' | 'warn' | 'alert'

const variantTone: Record<
  StatusVariant,
  { fill: string; icon: string; headline: string }
> = {
  info: {
    fill: 'bg-surface',
    icon: 'text-ink',
    headline: 'text-ink',
  },
  success: {
    fill: 'bg-ok-muted',
    icon: 'text-ok',
    headline: 'text-ok',
  },
  warn: {
    fill: 'bg-warn-muted',
    icon: 'text-warn',
    headline: 'text-warn',
  },
  alert: {
    fill: 'bg-alert-muted',
    icon: 'text-alert',
    headline: 'text-alert',
  },
}

interface StatusPanelProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: StatusVariant
  icon?: React.ReactNode
  headline: React.ReactNode
  detail?: React.ReactNode
  /** Trailing slot for a retry/dismiss `Button`; never text. */
  action?: React.ReactNode
}

/**
 * The one way to show a status with copy: a tinted block, headline in the
 * status colour, detail in faint ink, an optional action at the end. `role`
 * passes through — `alert` for an error the user must see now, `status`
 * for the rest.
 */
export function StatusPanel({
  variant = 'info',
  icon,
  headline,
  detail,
  action,
  className,
  ...props
}: StatusPanelProps) {
  const tone = variantTone[variant]
  return (
    <div
      className={cn(
        'flex items-start gap-x-3 rounded-md px-3.5 py-3',
        tone.fill,
        className,
      )}
      {...props}
    >
      {icon ? (
        <span aria-hidden className={cn('size-[18px] shrink-0', tone.icon)}>
          {icon}
        </span>
      ) : null}
      <div className="min-w-0 flex flex-col gap-y-0.5">
        <div
          className={cn('font-sans text-[13px] font-semibold', tone.headline)}
        >
          {headline}
        </div>
        {detail ? (
          <div className="font-sans text-[12px] text-ink-faint">{detail}</div>
        ) : null}
      </div>
      {action ? (
        <div className="ml-auto flex shrink-0 items-center gap-1.5 self-center">
          {action}
        </div>
      ) : null}
    </div>
  )
}
