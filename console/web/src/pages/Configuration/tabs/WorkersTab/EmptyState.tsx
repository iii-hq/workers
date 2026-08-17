import { SlidersHorizontal } from 'lucide-react'
import { cn } from '@/lib/utils'
import { wt } from './typography'

interface EmptyStateProps {
  title: string
  description: string
}

/**
 * Local empty state, styled like the directory worker's workspace hero:
 * a quiet glyph over a titled explanation that teaches the next action.
 * Not reusing `components/ui/EmptyState` because that one wraps a heavier
 * `<Cell>` and is designed for inline placement in the traces panel.
 */
export function EditorEmptyState({ title, description }: EmptyStateProps) {
  return (
    <div className="flex-1 flex flex-col items-center justify-center gap-2 px-6 py-12 text-center bg-panel">
      <SlidersHorizontal className="size-7 text-ink-ghost mb-2" aria-hidden />
      <p className={cn(wt.bodyLg, 'font-semibold text-ink lowercase')}>
        {title}
      </p>
      <p
        className={cn(
          wt.bodySm,
          'text-ink-faint lowercase max-w-md leading-relaxed',
        )}
      >
        {description}
      </p>
    </div>
  )
}
