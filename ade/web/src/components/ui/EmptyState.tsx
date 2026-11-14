import type * as React from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/shadcn/empty'
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

/**
 * What this place is and how to fill it: the shadcn `Empty` composition
 * (icon tile, title, one or two lines, then the actions), or a compact
 * tinted row inside a card or a list.
 */
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
        compact ? 'mt-2' : 'justify-center',
      )}
    >
      {all.map((a, index) => (
        <Button
          key={a.label}
          variant={index === 0 && !compact ? 'default' : 'secondary'}
          size="sm"
          onClick={a.onClick}
        >
          {a.label}
        </Button>
      ))}
    </div>
  ) : null
  if (compact) {
    return (
      <div
        className={cn(
          'flex items-start gap-3 rounded-lg bg-muted px-3 py-2.5',
          className,
        )}
      >
        {Icon ? (
          <Icon
            aria-hidden
            className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          />
        ) : null}
        <div className="min-w-0 flex-1">
          <p className="text-body font-semibold text-foreground">{title}</p>
          <p className="text-xs text-muted-foreground">{description}</p>
          {buttons}
        </div>
      </div>
    )
  }
  return (
    <Empty className={cn('py-10 font-inter', className)}>
      <EmptyHeader>
        {Icon ? (
          <EmptyMedia variant="icon">
            <Icon aria-hidden className="size-4" />
          </EmptyMedia>
        ) : null}
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription className="max-w-[44ch] text-pretty">
          {description}
        </EmptyDescription>
      </EmptyHeader>
      {buttons ? <EmptyContent>{buttons}</EmptyContent> : null}
    </Empty>
  )
}
