import { GripVertical } from 'lucide-react'
import { type KeyboardEvent, type PointerEvent, useRef } from 'react'
import { cn } from '@/lib/utils'

export type ReorderTarget = { before: string } | { after: string }

/** The items of the handle's list on screen: `[data-reorder-item]` children. */
function listItems(handle: Element): HTMLElement[] {
  const list = handle.closest('[data-reorder-list]')
  if (!list) return []
  return [
    ...list.querySelectorAll<HTMLElement>(':scope > [data-reorder-item]'),
  ].filter((item) => !item.hidden)
}

const idOf = (item: HTMLElement) => item.dataset.reorderItem ?? ''

/**
 * The grip that moves an item of the closest `[data-reorder-list]`: drag it
 * (mouse or touch; the list reorders as it goes) or use the arrow keys,
 * Home and End. It names its target by a neighbour on screen, so items
 * that are not shown keep their places.
 */
export function ReorderHandle({
  label,
  onMove,
  className,
}: {
  /** The item's name, for "Move <label>". */
  label: string
  onMove: (target: ReorderTarget) => void
  className?: string
}) {
  const ref = useRef<HTMLButtonElement>(null)
  const dragging = useRef(false)

  const move = (target: ReorderTarget) => {
    onMove(target)
    // A reorder can take the focused node out of the DOM for a moment.
    requestAnimationFrame(() => ref.current?.focus({ preventScroll: true }))
  }

  const onPointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    dragging.current = true
    event.currentTarget
      .closest('[data-reorder-item]')
      ?.setAttribute('data-dragging', '')
  }

  const onPointerMove = (event: PointerEvent<HTMLButtonElement>) => {
    if (!dragging.current) return
    const mine = event.currentTarget.closest<HTMLElement>('[data-reorder-item]')
    if (!mine) return
    const items = listItems(event.currentTarget)
    const others = items.filter((item) => item !== mine)
    const above = others.filter((item) => {
      const rect = item.getBoundingClientRect()
      return rect.top + rect.height / 2 < event.clientY
    })
    if (items.indexOf(mine) === above.length) return
    const last = above.at(-1)
    if (last) onMove({ after: idOf(last) })
    else if (others[0]) onMove({ before: idOf(others[0]) })
  }

  const onPointerEnd = (event: PointerEvent<HTMLButtonElement>) => {
    dragging.current = false
    event.currentTarget
      .closest('[data-reorder-item]')
      ?.removeAttribute('data-dragging')
  }

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const mine = event.currentTarget.closest<HTMLElement>('[data-reorder-item]')
    if (!mine) return
    const items = listItems(event.currentTarget)
    const at = items.indexOf(mine)
    const first = items[0]
    const last = items.at(-1)
    let target: ReorderTarget | null = null
    if (event.key === 'ArrowUp' && at > 0) {
      target = { before: idOf(items[at - 1]) }
    } else if (event.key === 'ArrowDown' && at < items.length - 1) {
      target = { after: idOf(items[at + 1]) }
    } else if (event.key === 'Home' && first && first !== mine) {
      target = { before: idOf(first) }
    } else if (event.key === 'End' && last && last !== mine) {
      target = { after: idOf(last) }
    }
    if (!target) return
    event.preventDefault()
    move(target)
  }

  return (
    <button
      ref={ref}
      type="button"
      aria-label={`Move ${label}`}
      title="Drag, or use the arrow keys"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onKeyDown={onKeyDown}
      className={cn(
        'flex shrink-0 cursor-grab touch-none items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus active:cursor-grabbing',
        className,
      )}
    >
      <GripVertical aria-hidden className="size-4" />
    </button>
  )
}
