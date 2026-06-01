import { GripVertical, Trash2 } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { DragReorder } from './useDragReorder'

type HandleProps = ReturnType<DragReorder['handleProps']>

/**
 * Grab handle shared by the array / dictionary rows. The DnD + keyboard
 * props from `useDragReorder` are spread onto the inner `<button>` (a host
 * element, so the callback `ref` and drag handlers type-check cleanly).
 */
export function DragHandle({
  label,
  handle,
}: {
  label: string
  handle: HandleProps
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title="drag to reorder (arrow keys also work)"
      className="shrink-0 flex items-center justify-center h-9 w-5 text-ink-faint hover:text-ink cursor-grab active:cursor-grabbing focus:outline-none focus-visible:text-accent"
      {...handle}
    >
      <GripVertical size={14} aria-hidden />
    </button>
  )
}

/**
 * Trash button shared by the array / dictionary rows. Disabled (with an
 * explanatory title) when removing would drop below the schema minimum.
 */
export function DeleteButton({
  label,
  onClick,
  disabled,
}: {
  label: string
  onClick: () => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={disabled ? 'minimum reached; cannot delete' : 'delete'}
      className={cn(
        'shrink-0 flex items-center justify-center h-9 w-8 border transition-colors',
        disabled
          ? 'border-rule text-ink-ghost cursor-not-allowed'
          : 'border-rule text-ink-faint hover:border-alert hover:text-alert',
      )}
    >
      <Trash2 size={14} aria-hidden />
    </button>
  )
}
