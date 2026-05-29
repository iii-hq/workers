import { type DragEvent, type KeyboardEvent, useRef, useState } from 'react'

/**
 * Drag-to-reorder wiring shared by `ArrayField` and `DictionaryField`.
 *
 * Native HTML5 DnD, driven from a dedicated grab handle so the row's
 * inputs (including the Lexical env editor) stay fully usable — only the
 * handle is `draggable`, and the drag image is set to the whole row via
 * `setDragImage`. Arrow keys on the handle move the item too, so reorder
 * works without a pointer; focus follows the moved item.
 *
 * The hook is presentation-agnostic: it returns prop bags plus the live
 * `dragIndex` / `overIndex` so the caller can style the dragged and
 * drop-target rows however it likes. Reordering itself is delegated to
 * `onReorder(from, to)` (use the `moveItem` helper).
 */
export interface DragReorder {
  /** Index currently being dragged, or `null`. */
  dragIndex: number | null
  /** Index the pointer is hovering as a drop target, or `null`. */
  overIndex: number | null
  /** Callback ref for each row container (used as the drag image). */
  setRowRef: (index: number) => (el: HTMLElement | null) => void
  /** Spread onto the grab handle element. */
  handleProps: (index: number) => {
    ref: (el: HTMLElement | null) => void
    draggable: true
    onDragStart: (e: DragEvent) => void
    onDragEnd: () => void
    onKeyDown: (e: KeyboardEvent) => void
  }
  /** Spread onto each row container. */
  rowProps: (index: number) => {
    onDragOver: (e: DragEvent) => void
    onDrop: (e: DragEvent) => void
  }
}

export function useDragReorder(
  count: number,
  onReorder: (from: number, to: number) => void,
): DragReorder {
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [overIndex, setOverIndex] = useState<number | null>(null)
  const rowRefs = useRef<(HTMLElement | null)[]>([])
  const handleRefs = useRef<(HTMLElement | null)[]>([])

  function reset() {
    setDragIndex(null)
    setOverIndex(null)
  }

  function keyboardMove(from: number, to: number) {
    onReorder(from, to)
    // Keep focus on the item the operator just moved so repeated presses
    // keep nudging the same row. The list re-renders first, so defer.
    requestAnimationFrame(() => handleRefs.current[to]?.focus())
  }

  return {
    dragIndex,
    overIndex,
    setRowRef: (index) => (el) => {
      rowRefs.current[index] = el
    },
    handleProps: (index) => ({
      ref: (el: HTMLElement | null) => {
        handleRefs.current[index] = el
      },
      draggable: true,
      onDragStart: (e: DragEvent) => {
        setDragIndex(index)
        e.dataTransfer.effectAllowed = 'move'
        // Firefox needs data set for the drag to start.
        e.dataTransfer.setData('text/plain', String(index))
        const row = rowRefs.current[index]
        if (row) e.dataTransfer.setDragImage(row, 12, 12)
      },
      onDragEnd: reset,
      onKeyDown: (e: KeyboardEvent) => {
        if ((e.key === 'ArrowUp' || e.key === 'ArrowLeft') && index > 0) {
          e.preventDefault()
          keyboardMove(index, index - 1)
        } else if (
          (e.key === 'ArrowDown' || e.key === 'ArrowRight') &&
          index < count - 1
        ) {
          e.preventDefault()
          keyboardMove(index, index + 1)
        }
      },
    }),
    rowProps: (index) => ({
      onDragOver: (e: DragEvent) => {
        if (dragIndex === null) return
        e.preventDefault()
        e.dataTransfer.dropEffect = 'move'
        if (overIndex !== index) setOverIndex(index)
      },
      onDrop: (e: DragEvent) => {
        e.preventDefault()
        if (dragIndex !== null && dragIndex !== index) {
          onReorder(dragIndex, index)
        }
        reset()
      },
    }),
  }
}
