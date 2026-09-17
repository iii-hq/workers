import type * as React from 'react'
import { Button } from '@/components/ui/Button'
import { Cell } from '@/components/ui/Cell'
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
  /** Inside a card or a list: no cell, 13px title, tighter padding. */
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
    <div className={cn('flex flex-wrap gap-1.5', compact ? 'mt-2' : 'mt-3')}>
      {all.map((a) => (
        <Button key={a.label} variant="ghost" size="sm" onClick={a.onClick}>
          {a.label}
        </Button>
      ))}
    </div>
  ) : null
  if (compact) {
    return (
      <div
        className={cn(
          'flex items-start gap-3 rounded-md bg-surface px-3 py-2.5',
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
  return (
    <Cell title={title} className={className}>
      <div className="flex items-start gap-3">
        {Icon ? (
          <Icon
            aria-hidden
            className="w-4 h-4 text-ink-faint shrink-0 mt-0.5"
          />
        ) : null}
        <div className="flex-1">
          <p className="font-sans text-[13px] text-ink-faint">{description}</p>
          {buttons}
        </div>
      </div>
    </Cell>
  )
}
