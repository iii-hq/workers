/* One drag/keyboard resizer for both split separators (the docked
   terminal panel's edge and the pane splits inside it). The caller keeps
   `role="separator"` and its `aria-value*` on the element and spreads the
   returned handlers; pointer capture makes the drag survive leaving the
   handle. Arrow keys step along the separator's axis: Left/Up is -1,
   Right/Down is +1. */

import type { KeyboardEvent, PointerEvent } from 'react'
import { useRef } from 'react'

interface SplitDragOptions<S> {
  /** The separator moves along the horizontal axis (else vertical). */
  horizontal: boolean
  /** Capture the drag origin from the pointer-down; `null` refuses the drag. */
  begin(event: PointerEvent<HTMLElement>): S | null
  /** The pointer moved `delta` px along the axis since `begin`. */
  move(origin: S, delta: number): void
  /** An arrow key along the axis: -1 towards start, +1 towards end. */
  step(direction: -1 | 1, event: KeyboardEvent<HTMLElement>): void
}

export function useSplitDrag<S>({ horizontal, begin, move, step }: SplitDragOptions<S>) {
  const dragRef = useRef<{ start: number; origin: S } | null>(null)
  const coordinate = (event: PointerEvent<HTMLElement>) => (horizontal ? event.clientX : event.clientY)

  const onPointerDown = (event: PointerEvent<HTMLElement>) => {
    const origin = begin(event)
    if (origin === null) return
    dragRef.current = { start: coordinate(event), origin }
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  const onPointerMove = (event: PointerEvent<HTMLElement>) => {
    const drag = dragRef.current
    if (drag) move(drag.origin, coordinate(event) - drag.start)
  }
  const onPointerUp = (event: PointerEvent<HTMLElement>) => {
    dragRef.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const [back, forward] = horizontal ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown']
    if (event.key !== back && event.key !== forward) return
    event.preventDefault()
    step(event.key === back ? -1 : 1, event)
  }
  return {
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel: onPointerUp,
    onLostPointerCapture: onPointerUp,
    onKeyDown,
  }
}
