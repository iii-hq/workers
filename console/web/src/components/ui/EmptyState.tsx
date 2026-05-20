import type { LucideIcon } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Cell } from '@/components/ui/Cell'

interface EmptyStateProps {
  icon?: LucideIcon
  title: string
  description: string
  action?: {
    label: string
    onClick: () => void
  }
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: EmptyStateProps) {
  return (
    <Cell title={title}>
      <div className="flex items-start gap-3">
        {Icon ? (
          <Icon
            aria-hidden
            className="w-4 h-4 text-ink-faint shrink-0 mt-0.5"
          />
        ) : null}
        <div className="flex-1">
          <p className="font-mono text-[13px] text-ink-faint lowercase">
            {description}
          </p>
          {action ? (
            <div className="mt-3">
              <Button variant="ghost" size="sm" onClick={action.onClick}>
                {action.label}
              </Button>
            </div>
          ) : null}
        </div>
      </div>
    </Cell>
  )
}
