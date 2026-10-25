import { cn } from '@/lib/utils'

interface EmptyStateProps {
  title: string
  description: string
  className?: string
}

/**
 * Local empty state: a titled explanation that teaches the next action, set
 * in the page column like any other settings copy. Not reusing
 * `components/ui/EmptyState` because that one wraps a heavier `<Cell>` and
 * is designed for inline placement in the traces panel.
 */
export function EditorEmptyState({
  title,
  description,
  className,
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex flex-1 flex-col items-center justify-center gap-1 px-6 py-16 text-center',
        className,
      )}
    >
      <p className="text-base font-medium leading-6 text-foreground">{title}</p>
      <p className="max-w-md text-pretty text-sm leading-5 text-muted-foreground">
        {description}
      </p>
    </div>
  )
}
