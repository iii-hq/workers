import type * as React from 'react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'

export interface EmptyStateAction {
  label: string
  onClick: () => void
}

interface EmptyStateProps {
  /** Any icon component that takes a className (lucide icons qualify);
   *  kept wider than LucideIcon so injected worker UI can pass its own. */
  icon?: React.ComponentType<{ className?: string }>
  title: string
  description: string
  /** The one action of a full cell. `actions` adds more (first is primary). */
  action?: EmptyStateAction
  actions?: readonly EmptyStateAction[]
  /** Inside a card or a list: a tinted row, 13px title, tighter padding. */
  compact?: boolean
  className?: string
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  actions,
  compact,
  className,
}: EmptyStateProps) {
  const all = [...(action ? [action] : []), ...(actions ?? [])]
  const buttons = all.length ? (
    <div
      className={cn(
        'flex flex-wrap gap-1.5',
        compact ? 'mt-2' : 'mt-5 justify-center',
      )}
    >
      {all.map((a) => (
        <Button key={a.label} variant="secondary" size="sm" onClick={a.onClick}>
          {a.label}
        </Button>
      ))}
    </div>
  ) : null
  if (compact) {
    return (
      <div
        className={cn(
          'flex items-start gap-3 rounded-xl bg-surface px-3 py-2.5',
          className,
        )}
      >
        {Icon ? (
          <Icon aria-hidden className="mt-0.5 size-4 shrink-0 text-ink-faint" />
        ) : null}
        <div className="min-w-0 flex-1">
          <p className="font-sans text-[13px] font-semibold text-ink">
            {title}
          </p>
          <p className="font-sans text-[12px] text-ink-faint">{description}</p>
          {buttons}
        </div>
      </div>
    )
  }
  // The full state stands on the canvas, not in a card: a centred column of
  // icon tile, title, one or two lines of copy, then the actions.
  return (
    <div
      className={cn(
        'flex flex-col items-center px-6 py-10 text-center font-inter',
        className,
      )}
    >
      {Icon ? (
        <div className="mb-4 flex size-10 items-center justify-center rounded-xl border border-transparent bg-card text-ink-faint shadow-[0_0_0_1px_var(--color-card-edge),0_1px_2px_rgb(0_0_0/0.06)]">
          <Icon aria-hidden className="size-5" />
        </div>
      ) : null}
      <p className="text-balance text-[14px] font-medium text-ink">{title}</p>
      <p className="mt-1 max-w-[44ch] text-pretty text-[14px] leading-relaxed text-ink-faint">
        {description}
      </p>
      {buttons}
    </div>
  )
}
