import { cn } from '@/lib/utils'
import { wt } from './typography'

interface EmptyStateProps {
  title: string
  description: string
}

/**
 * Local empty state. Not reusing `components/ui/EmptyState` because that
 * one wraps a heavier `<Cell>` and is designed for inline placement in
 * the traces panel; the editor's empty/no-selection state wants the
 * lighter centered treatment that matches the routing fallback.
 */
export function EditorEmptyState({ title, description }: EmptyStateProps) {
  return (
    <div className="flex-1 flex items-center justify-center px-6">
      <div className="max-w-md text-center space-y-2">
        <p className={cn(wt.bodyLg, 'text-ink lowercase')}>{title}</p>
        <p className={cn(wt.bodySm, 'text-ink-faint lowercase')}>
          {description}
        </p>
      </div>
    </div>
  )
}
